package api

import (
	"context"
	"encoding/json"
	"net/http"
	"time"

	"github.com/gorilla/mux"
	"github.com/redis/go-redis/v9"
	"github.com/sbelakho2/apex-mediation-platform/backend/config/internal/killswitch"
	"github.com/sbelakho2/apex-mediation-platform/backend/config/internal/rollout"
	"github.com/sbelakho2/apex-mediation-platform/backend/config/internal/signing"
	"github.com/sbelakho2/apex-mediation-platform/backend/config/internal/validation"
	log "github.com/sirupsen/logrus"
)

// Handler manages HTTP API endpoints
type Handler struct {
	signer            *signing.Ed25519Signer
	rolloutController *rollout.Controller
	killSwitchManager *killswitch.Manager
	redis             *redis.Client
	validator         *validation.Validator
}

// NewHandler creates a new API handler
func NewHandler(
	signer *signing.Ed25519Signer,
	rolloutController *rollout.Controller,
	killSwitchManager *killswitch.Manager,
	redis *redis.Client,
	validator *validation.Validator,
) *Handler {
	return &Handler{
		signer:            signer,
		rolloutController: rolloutController,
		killSwitchManager: killSwitchManager,
		redis:             redis,
		validator:         validator,
	}
}

// GetConfig retrieves configuration for an app
func (h *Handler) GetConfig(w http.ResponseWriter, r *http.Request) {
	vars := mux.Vars(r)
	appID := vars["app_id"]

	if appID == "" {
		respondError(w, http.StatusBadRequest, "app_id is required")
		return
	}

	ctx := r.Context()

	// Check global kill switch
	if h.killSwitchManager.IsActive(ctx, "global", "all") {
		respondError(w, http.StatusServiceUnavailable, "Service temporarily unavailable")
		return
	}

	// Get routing configuration
	configID, err := h.getRoutedConfig(ctx, appID)
	if err != nil {
		log.Errorf("Failed to get routed config: %v", err)
		respondError(w, http.StatusInternalServerError, "Internal server error")
		return
	}

	// Retrieve signed configuration
	signedConfig, err := h.getSignedConfig(ctx, configID)
	if err != nil {
		log.Errorf("Failed to get signed config: %v", err)
		respondError(w, http.StatusNotFound, "Configuration not found")
		return
	}

	// Verify signature before returning to clients
	if err := h.signer.Verify(signedConfig); err != nil {
		log.Errorf("Signature verification failed for config %s: %v", configID, err)
		respondError(w, http.StatusInternalServerError, "Configuration verification failed")
		return
	}

	// Return configuration
	respondJSON(w, http.StatusOK, signedConfig)
}

// DeployConfig deploys a new configuration with staged rollout
func (h *Handler) DeployConfig(w http.ResponseWriter, r *http.Request) {
	var req struct {
		ConfigID   string             `json:"config_id"`
		Payload    json.RawMessage    `json:"payload"`
		Stages     []rollout.Stage    `json:"stages,omitempty"`
		SLOTargets rollout.SLOTargets `json:"slo_targets,omitempty"`
	}

	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		respondError(w, http.StatusBadRequest, "Invalid request body")
		return
	}

	// Validate required fields
	if req.ConfigID == "" {
		respondError(w, http.StatusBadRequest, "config_id is required")
		return
	}

	if len(req.Payload) == 0 {
		respondError(w, http.StatusBadRequest, "payload is required")
		return
	}

	// Basic validation (size + required fields) before signing
	if err := h.validator.ValidateSize(req.Payload); err != nil {
		respondError(w, http.StatusBadRequest, err.Error())
		return
	}
	if err := h.validator.ValidateJSON(req.Payload); err != nil {
		respondError(w, http.StatusBadRequest, err.Error())
		return
	}

	ctx := r.Context()

	// Sign configuration
	version := time.Now().Unix()
	signedConfig, err := h.signer.Sign(req.ConfigID, version, req.Payload)
	if err != nil {
		log.Errorf("Failed to sign config: %v", err)
		respondError(w, http.StatusInternalServerError, "Failed to sign configuration")
		return
	}

	// Save signed configuration
	if err := h.saveSignedConfig(ctx, signedConfig); err != nil {
		log.Errorf("Failed to save signed config: %v", err)
		respondError(w, http.StatusInternalServerError, "Failed to save configuration")
		return
	}

	// Create rollout configuration
	rolloutConfig := &rollout.RolloutConfig{
		ConfigID:   req.ConfigID,
		Stages:     req.Stages,
		SLOTargets: req.SLOTargets,
	}

	// Start staged rollout
	if err := h.rolloutController.Deploy(ctx, rolloutConfig); err != nil {
		log.Errorf("Failed to start rollout: %v", err)
		respondError(w, http.StatusInternalServerError, "Failed to start rollout")
		return
	}

	log.Infof("Started rollout for config %s", req.ConfigID)

	respondJSON(w, http.StatusAccepted, map[string]interface{}{
		"config_id": req.ConfigID,
		"version":   version,
		"status":    "deploying",
		"message":   "Staged rollout initiated",
	})
}

// RollbackConfig manually triggers a configuration rollback
func (h *Handler) RollbackConfig(w http.ResponseWriter, r *http.Request) {
	var req struct {
		ConfigID string `json:"config_id"`
		Reason   string `json:"reason"`
	}

	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		respondError(w, http.StatusBadRequest, "Invalid request body")
		return
	}

	if req.ConfigID == "" {
		respondError(w, http.StatusBadRequest, "config_id is required")
		return
	}

	ctx := r.Context()

	// Get rollout status
	_, err := h.rolloutController.GetStatus(ctx, req.ConfigID)
	if err != nil {
		respondError(w, http.StatusNotFound, "Rollout not found")
		return
	}

	// Update routing to previous config
	if err := h.rollbackRouting(ctx); err != nil {
		log.Errorf("Failed to rollback routing: %v", err)
		respondError(w, http.StatusInternalServerError, "Failed to rollback")
		return
	}

	reason := req.Reason
	if reason == "" {
		reason = "Manual rollback triggered"
	}

	log.Warnf("Manual rollback triggered for config %s: %s", req.ConfigID, reason)

	respondJSON(w, http.StatusOK, map[string]interface{}{
		"config_id": req.ConfigID,
		"status":    "rolled_back",
		"reason":    reason,
		"timestamp": time.Now().UTC(),
	})
}

// ActivateKillSwitch activates a kill switch
func (h *Handler) ActivateKillSwitch(w http.ResponseWriter, r *http.Request) {
	vars := mux.Vars(r)
	switchType := vars["type"]
	switchID := vars["id"]

	if switchType == "" || switchID == "" {
		respondError(w, http.StatusBadRequest, "type and id are required")
		return
	}

	ctx := r.Context()

	if err := h.killSwitchManager.Activate(ctx, switchType, switchID); err != nil {
		log.Errorf("Failed to activate kill switch: %v", err)
		respondError(w, http.StatusInternalServerError, "Failed to activate kill switch")
		return
	}

	log.Warnf("Kill switch activated: %s/%s", switchType, switchID)

	respondJSON(w, http.StatusOK, map[string]interface{}{
		"type":      switchType,
		"id":        switchID,
		"status":    "active",
		"timestamp": time.Now().UTC(),
	})
}

// DeactivateKillSwitch deactivates a kill switch
func (h *Handler) DeactivateKillSwitch(w http.ResponseWriter, r *http.Request) {
	vars := mux.Vars(r)
	switchType := vars["type"]
	switchID := vars["id"]

	if switchType == "" || switchID == "" {
		respondError(w, http.StatusBadRequest, "type and id are required")
		return
	}

	ctx := r.Context()

	if err := h.killSwitchManager.Deactivate(ctx, switchType, switchID); err != nil {
		log.Errorf("Failed to deactivate kill switch: %v", err)
		respondError(w, http.StatusInternalServerError, "Failed to deactivate kill switch")
		return
	}

	log.Infof("Kill switch deactivated: %s/%s", switchType, switchID)

	respondJSON(w, http.StatusOK, map[string]interface{}{
		"type":      switchType,
		"id":        switchID,
		"status":    "inactive",
		"timestamp": time.Now().UTC(),
	})
}

// ListKillSwitches lists all active kill switches
func (h *Handler) ListKillSwitches(w http.ResponseWriter, r *http.Request) {
	ctx := r.Context()

	switches, err := h.killSwitchManager.ListActive(ctx)
	if err != nil {
		log.Errorf("Failed to list kill switches: %v", err)
		respondError(w, http.StatusInternalServerError, "Failed to list kill switches")
		return
	}

	respondJSON(w, http.StatusOK, map[string]interface{}{
		"switches": switches,
		"count":    len(switches),
	})
}

// GetRolloutStatus returns the current rollout status
func (h *Handler) GetRolloutStatus(w http.ResponseWriter, r *http.Request) {
	configID := r.URL.Query().Get("config_id")
	if configID == "" {
		respondError(w, http.StatusBadRequest, "config_id query parameter is required")
		return
	}

	ctx := r.Context()

	rolloutConfig, err := h.rolloutController.GetStatus(ctx, configID)
	if err != nil {
		respondError(w, http.StatusNotFound, "Rollout not found")
		return
	}

	respondJSON(w, http.StatusOK, rolloutConfig)
}

// Helper functions

func (h *Handler) getRoutedConfig(ctx context.Context, appID string) (string, error) {
	// Get routing configuration from Redis
	key := "config:routing"
	data, err := h.redis.Get(ctx, key).Bytes()
	if err != nil {
		// Default config if routing not found
		return "default", nil
	}

	var routing struct {
		ConfigID string `json:"config_id"`
	}

	if err := json.Unmarshal(data, &routing); err != nil {
		return "", err
	}

	return routing.ConfigID, nil
}

func (h *Handler) getSignedConfig(ctx context.Context, configID string) (*signing.SignedConfig, error) {
	key := "config:signed:" + configID
	data, err := h.redis.Get(ctx, key).Bytes()
	if err != nil {
		return nil, err
	}

	var config signing.SignedConfig
	if err := json.Unmarshal(data, &config); err != nil {
		return nil, err
	}

	return &config, nil
}

func (h *Handler) saveSignedConfig(ctx context.Context, config *signing.SignedConfig) error {
	key := "config:signed:" + config.ConfigID
	data, err := json.Marshal(config)
	if err != nil {
		return err
	}

	return h.redis.Set(ctx, key, data, 0).Err()
}

func (h *Handler) rollbackRouting(ctx context.Context) error {
	key := "config:routing"
	data := map[string]interface{}{
		"config_id":  "previous",
		"percentage": 1.0,
		"updated_at": time.Now().UTC().Unix(),
	}

	jsonData, err := json.Marshal(data)
	if err != nil {
		return err
	}

	return h.redis.Set(ctx, key, jsonData, 0).Err()
}

func respondJSON(w http.ResponseWriter, status int, data interface{}) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	json.NewEncoder(w).Encode(data)
}

func respondError(w http.ResponseWriter, status int, message string) {
	respondJSON(w, status, map[string]string{
		"error": message,
	})
}
