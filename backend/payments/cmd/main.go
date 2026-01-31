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
	"github.com/redis/go-redis/v9"
	"github.com/shopspring/decimal"
	log "github.com/sirupsen/logrus"

	"payments/internal/ledger"
	"payments/internal/orchestrator"
	"payments/internal/rails/paypalpay"
	"payments/internal/rails/stripepay"
)

func main() {
	// Setup logging
	log.SetFormatter(&log.JSONFormatter{})
	log.SetLevel(log.InfoLevel)

	// Connect to Redis
	redisClient := redis.NewClient(&redis.Options{
		Addr:     getEnv("REDIS_ADDR", "localhost:6379"),
		Password: getEnv("REDIS_PASSWORD", ""),
		DB:       0,
	})

	ctx := context.Background()
	if err := redisClient.Ping(ctx).Err(); err != nil {
		log.WithError(err).Fatal("Failed to connect to Redis")
	}
	log.Info("Connected to Redis")

	// Initialize ledger
	ledgerService := ledger.NewDoubleEntryLedger(redisClient)

	// Initialize payment orchestrator
	paymentOrchestrator := ledger.NewPaymentOrchestrator(ledgerService, redisClient)

	// Initialize payment manager
	paymentManager := orchestrator.NewPaymentManager(redisClient)

	// Register payment rails
	stripeRail := stripepay.NewStripeRail(getEnv("STRIPE_API_KEY", ""))
	paymentManager.RegisterRail(orchestrator.MethodStripe, stripeRail)

	paypalRail := paypalpay.NewPayPalRail(
		getEnv("PAYPAL_CLIENT_ID", ""),
		getEnv("PAYPAL_CLIENT_SECRET", ""),
		getEnv("PAYPAL_PRODUCTION", "false") == "true",
	)
	paymentManager.RegisterRail(orchestrator.MethodPayPal, paypalRail)

	// Setup HTTP server
	router := mux.NewRouter()

	// Health check
	router.HandleFunc("/health", func(w http.ResponseWriter, r *http.Request) {
		json.NewEncoder(w).Encode(map[string]string{"status": "healthy"})
	}).Methods("GET")

	// Get balance
	router.HandleFunc("/v1/balance/{publisher_id}", func(w http.ResponseWriter, r *http.Request) {
		vars := mux.Vars(r)
		publisherID := vars["publisher_id"]
		currency := r.URL.Query().Get("currency")
		if currency == "" {
			currency = "USD"
		}

		balance, err := ledgerService.GetBalance(r.Context(), publisherID, currency)
		if err != nil {
			http.Error(w, err.Error(), http.StatusInternalServerError)
			return
		}

		w.Header().Set("Content-Type", "application/json")
		json.NewEncoder(w).Encode(map[string]interface{}{
			"publisher_id": publisherID,
			"balance":      balance.String(),
			"currency":     currency,
		})
	}).Methods("GET")

	// Record revenue
	router.HandleFunc("/v1/revenue", func(w http.ResponseWriter, r *http.Request) {
		var req struct {
			PublisherID  string `json:"publisher_id"`
			Amount       string `json:"amount"`
			Currency     string `json:"currency"`
			ImpressionID string `json:"impression_id"`
		}

		if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
			http.Error(w, err.Error(), http.StatusBadRequest)
			return
		}

		amount, err := decimal.NewFromString(req.Amount)
		if err != nil {
			http.Error(w, "Invalid amount", http.StatusBadRequest)
			return
		}

		if err := ledgerService.RecordRevenue(r.Context(), req.PublisherID, amount, req.Currency, req.ImpressionID); err != nil {
			http.Error(w, err.Error(), http.StatusInternalServerError)
			return
		}

		w.WriteHeader(http.StatusCreated)
		json.NewEncoder(w).Encode(map[string]string{"status": "recorded"})
	}).Methods("POST")

	// Get transactions
	router.HandleFunc("/v1/transactions/{publisher_id}", func(w http.ResponseWriter, r *http.Request) {
		vars := mux.Vars(r)
		publisherID := vars["publisher_id"]

		transactions, err := ledgerService.GetTransactions(r.Context(), publisherID, 100)
		if err != nil {
			http.Error(w, err.Error(), http.StatusInternalServerError)
			return
		}

		w.Header().Set("Content-Type", "application/json")
		json.NewEncoder(w).Encode(map[string]interface{}{
			"transactions": transactions,
			"count":        len(transactions),
		})
	}).Methods("GET")

	// Payout validation constants - SECURITY: P0 Fix #11
	var (
		minPayoutAmount = decimal.NewFromFloat(10.0)    // Minimum $10 payout
		maxPayoutAmount = decimal.NewFromFloat(50000.0) // Maximum $50,000 per payout
		validCurrencies = map[string]bool{
			"USD": true, "EUR": true, "GBP": true, "CAD": true, "AUD": true,
		}
	)

	// Initiate payout
	router.HandleFunc("/v1/payout", func(w http.ResponseWriter, r *http.Request) {
		var req struct {
			PublisherID string `json:"publisher_id"`
			Amount      string `json:"amount"`
			Currency    string `json:"currency"`
		}

		if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
			http.Error(w, err.Error(), http.StatusBadRequest)
			return
		}

		// SECURITY: Validate publisher ID format
		if req.PublisherID == "" || len(req.PublisherID) > 64 {
			http.Error(w, "Invalid publisher_id", http.StatusBadRequest)
			return
		}

		// SECURITY: Validate currency
		currency := req.Currency
		if currency == "" {
			currency = "USD" // Default to USD
		}
		if !validCurrencies[currency] {
			http.Error(w, "Invalid or unsupported currency", http.StatusBadRequest)
			return
		}

		amount, err := decimal.NewFromString(req.Amount)
		if err != nil {
			http.Error(w, "Invalid amount format", http.StatusBadRequest)
			return
		}

		// SECURITY: Validate amount is positive (P0 Fix #11)
		if amount.LessThanOrEqual(decimal.Zero) {
			log.WithFields(log.Fields{
				"publisher_id": req.PublisherID,
				"amount":       req.Amount,
			}).Warn("Payout rejected: non-positive amount")
			http.Error(w, "Amount must be greater than zero", http.StatusBadRequest)
			return
		}

		// SECURITY: Validate minimum payout threshold (P0 Fix #11)
		if amount.LessThan(minPayoutAmount) {
			log.WithFields(log.Fields{
				"publisher_id": req.PublisherID,
				"amount":       req.Amount,
				"minimum":      minPayoutAmount.String(),
			}).Warn("Payout rejected: below minimum threshold")
			http.Error(w, "Amount below minimum payout threshold ($10)", http.StatusBadRequest)
			return
		}

		// SECURITY: Validate maximum payout limit (P0 Fix #11)
		if amount.GreaterThan(maxPayoutAmount) {
			log.WithFields(log.Fields{
				"publisher_id": req.PublisherID,
				"amount":       req.Amount,
				"maximum":      maxPayoutAmount.String(),
			}).Warn("Payout rejected: exceeds maximum limit")
			http.Error(w, "Amount exceeds maximum payout limit ($50,000)", http.StatusBadRequest)
			return
		}

		// SECURITY: Validate amount has reasonable precision (max 2 decimal places)
		// This prevents floating point manipulation attacks
		truncated := amount.Truncate(2)
		if !amount.Equal(truncated) {
			log.WithFields(log.Fields{
				"publisher_id": req.PublisherID,
				"amount":       req.Amount,
			}).Warn("Payout rejected: excessive decimal precision")
			http.Error(w, "Amount must have at most 2 decimal places", http.StatusBadRequest)
			return
		}

		payoutID, err := paymentOrchestrator.InitiatePayout(r.Context(), req.PublisherID, amount, currency)
		if err != nil {
			http.Error(w, err.Error(), http.StatusInternalServerError)
			return
		}

		w.WriteHeader(http.StatusCreated)
		json.NewEncoder(w).Encode(map[string]string{
			"payout_id": payoutID,
			"status":    "pending",
			"currency":  currency,
		})
	}).Methods("POST")

	// Get payout status
	router.HandleFunc("/v1/payout/{payout_id}", func(w http.ResponseWriter, r *http.Request) {
		vars := mux.Vars(r)
		payoutID := vars["payout_id"]

		status, err := paymentManager.GetPayoutStatus(r.Context(), payoutID)
		if err != nil {
			http.Error(w, err.Error(), http.StatusInternalServerError)
			return
		}

		w.Header().Set("Content-Type", "application/json")
		json.NewEncoder(w).Encode(map[string]interface{}{
			"payout_id": payoutID,
			"status":    status,
		})
	}).Methods("GET")

	// Start server
	port := getEnv("PORT", "8083")
	srv := &http.Server{
		Addr:         ":" + port,
		Handler:      router,
		ReadTimeout:  15 * time.Second,
		WriteTimeout: 15 * time.Second,
		IdleTimeout:  60 * time.Second,
	}

	// Graceful shutdown
	go func() {
		sigChan := make(chan os.Signal, 1)
		signal.Notify(sigChan, os.Interrupt, syscall.SIGTERM)
		<-sigChan

		log.Info("Shutting down payment service...")

		ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
		defer cancel()

		if err := srv.Shutdown(ctx); err != nil {
			log.WithError(err).Error("Server shutdown failed")
		}
	}()

	log.WithField("port", port).Info("Payment service started")
	if err := srv.ListenAndServe(); err != nil && err != http.ErrServerClosed {
		log.WithError(err).Fatal("Server failed")
	}
}

func getEnv(key, defaultValue string) string {
	if value := os.Getenv(key); value != "" {
		return value
	}
	return defaultValue
}
