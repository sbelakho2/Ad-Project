package main

import (
	"context"
	"encoding/json"
	"net/http"
	"os"
	"os/signal"
	"syscall"
	"time"

	"github.com/gorilla/mux"
	"github.com/jackc/pgx/v5/pgxpool"
	"github.com/redis/go-redis/v9"
	"github.com/sbelakho2/apex-mediation-platform/backend/config/internal/api"
	"github.com/sbelakho2/apex-mediation-platform/backend/config/internal/auth"
	"github.com/sbelakho2/apex-mediation-platform/backend/config/internal/killswitch"
	"github.com/sbelakho2/apex-mediation-platform/backend/config/internal/rollout"
	"github.com/sbelakho2/apex-mediation-platform/backend/config/internal/signing"
	"github.com/sbelakho2/apex-mediation-platform/backend/config/internal/validation"
	log "github.com/sirupsen/logrus"
)

type Config struct {
	Port           string
	SigningKeyPath string
	RedisAddr      string
	DatabaseURL    string
	Environment    string
}

func main() {
	// Load configuration
	cfg := loadConfig()

	// Setup logging
	setupLogging(cfg.Environment)

	log.Info("Starting Configuration Service...")

	// Initialize dependencies
	ctx := context.Background()

	// Redis client
	redisClient := redis.NewClient(&redis.Options{
		Addr: cfg.RedisAddr,
		DB:   0,
	})
	defer redisClient.Close()

	// Test Redis connection
	if err := redisClient.Ping(ctx).Err(); err != nil {
		log.Fatalf("Failed to connect to Redis: %v", err)
	}

	// Postgres pool (used for SDK key auth)
	pgPool, err := pgxpool.New(ctx, cfg.DatabaseURL)
	if err != nil {
		log.Fatalf("Failed to initialize Postgres pool: %v", err)
	}
	defer pgPool.Close()
	if err := pgPool.Ping(ctx); err != nil {
		log.Fatalf("Failed to connect to Postgres: %v", err)
	}

	// Ed25519 signer
	signer, err := signing.NewEd25519Signer(cfg.SigningKeyPath)
	if err != nil {
		log.Fatalf("Failed to initialize signer: %v", err)
	}

	// Rollout controller
	rolloutController := rollout.NewController(redisClient)

	// Kill switch manager
	killSwitchManager := killswitch.NewManager(redisClient)

	// Config validator (JSON + schema hooks)
	validator := validation.NewValidator()

	// API handlers
	apiHandler := api.NewHandler(
		signer,
		rolloutController,
		killSwitchManager,
		redisClient,
		validator,
	)

	// Setup HTTP server
	router := mux.NewRouter()

	// Health check
	router.HandleFunc("/health", healthHandler).Methods("GET")
	router.HandleFunc("/ready", readyHandler(redisClient, pgPool)).Methods("GET")

	// Config endpoints
	sdk := router.NewRoute().Subrouter()
	sdk.Use(auth.Middleware(pgPool, "app_id"))
	sdk.HandleFunc("/v1/config/{app_id}", apiHandler.GetConfig).Methods("GET")

	router.HandleFunc("/v1/config", apiHandler.DeployConfig).Methods("POST")
	router.HandleFunc("/v1/config/rollback", apiHandler.RollbackConfig).Methods("POST")

	// Kill switch endpoints
	router.HandleFunc("/v1/killswitch/{type}/{id}", apiHandler.ActivateKillSwitch).Methods("POST")
	router.HandleFunc("/v1/killswitch/{type}/{id}", apiHandler.DeactivateKillSwitch).Methods("DELETE")
	router.HandleFunc("/v1/killswitch", apiHandler.ListKillSwitches).Methods("GET")

	// Rollout endpoints
	router.HandleFunc("/v1/rollout/status", apiHandler.GetRolloutStatus).Methods("GET")

	// Metrics endpoint
	router.HandleFunc("/metrics", metricsHandler).Methods("GET")

	// Create HTTP server
	srv := &http.Server{
		Addr:         ":" + cfg.Port,
		Handler:      router,
		ReadTimeout:  15 * time.Second,
		WriteTimeout: 15 * time.Second,
		IdleTimeout:  60 * time.Second,
	}

	// Start server in goroutine
	go func() {
		log.Infof("Server listening on port %s", cfg.Port)
		if err := srv.ListenAndServe(); err != nil && err != http.ErrServerClosed {
			log.Fatalf("Server error: %v", err)
		}
	}()

	// Wait for interrupt signal
	quit := make(chan os.Signal, 1)
	signal.Notify(quit, syscall.SIGINT, syscall.SIGTERM)
	<-quit

	log.Info("Shutting down server...")

	// Graceful shutdown
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()

	if err := srv.Shutdown(ctx); err != nil {
		log.Errorf("Server forced to shutdown: %v", err)
	}

	log.Info("Server stopped")
}

func loadConfig() *Config {
	return &Config{
		Port:           getEnv("PORT", "8081"),
		SigningKeyPath: getEnv("SIGNING_KEY_PATH", "/keys/signing.key"),
		RedisAddr:      getEnv("REDIS_ADDR", "localhost:6379"),
		DatabaseURL:    getEnv("DATABASE_URL", "postgresql://postgres:postgres@localhost:5432/apexmediation"),
		Environment:    getEnv("ENV", "development"),
	}
}

func setupLogging(env string) {
	log.SetFormatter(&log.JSONFormatter{})
	log.SetOutput(os.Stdout)

	if env == "development" {
		log.SetLevel(log.DebugLevel)
	} else {
		log.SetLevel(log.InfoLevel)
	}
}

func healthHandler(w http.ResponseWriter, r *http.Request) {
	w.WriteHeader(http.StatusOK)
	json.NewEncoder(w).Encode(map[string]string{
		"status": "healthy",
		"time":   time.Now().UTC().Format(time.RFC3339),
	})
}

func readyHandler(redisClient *redis.Client, pgPool *pgxpool.Pool) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		ctx, cancel := context.WithTimeout(r.Context(), 2*time.Second)
		defer cancel()

		if err := redisClient.Ping(ctx).Err(); err != nil {
			w.WriteHeader(http.StatusServiceUnavailable)
			json.NewEncoder(w).Encode(map[string]string{
				"status": "not ready",
				"error":  err.Error(),
			})
			return
		}

		if err := pgPool.Ping(ctx); err != nil {
			w.WriteHeader(http.StatusServiceUnavailable)
			json.NewEncoder(w).Encode(map[string]string{
				"status": "not ready",
				"error":  err.Error(),
			})
			return
		}

		w.WriteHeader(http.StatusOK)
		json.NewEncoder(w).Encode(map[string]string{
			"status": "ready",
		})
	}
}

func metricsHandler(w http.ResponseWriter, r *http.Request) {
	// TODO: Implement Prometheus metrics
	w.WriteHeader(http.StatusOK)
	w.Write([]byte("# Metrics endpoint\n"))
}

func getEnv(key, defaultValue string) string {
	if value := os.Getenv(key); value != "" {
		return value
	}
	return defaultValue
}
