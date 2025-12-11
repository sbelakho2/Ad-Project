package api

import (
	"encoding/json"
	"net/http"
	"strconv"
	"strings"
	"time"

	"github.com/gorilla/mux"
	"github.com/jackc/pgx/v5/pgxpool"
	log "github.com/sirupsen/logrus"

	"github.com/rivalapexmediation/auction/internal/auth"
	"github.com/rivalapexmediation/auction/internal/bidders"
	"github.com/rivalapexmediation/auction/internal/bidding"
	"github.com/rivalapexmediation/auction/internal/waterfall"
)

// Handlers contains HTTP handlers
type Handlers struct {
	auctionEngine    *bidding.AuctionEngine
	waterfallManager *waterfall.WaterfallManager
	pgPool           *pgxpool.Pool
}

// NewHandlers creates new HTTP handlers
func NewHandlers(ae *bidding.AuctionEngine, wm *waterfall.WaterfallManager, pgPool *pgxpool.Pool) *Handlers {
	return &Handlers{
		auctionEngine:    ae,
		waterfallManager: wm,
		pgPool:           pgPool,
	}
}

// HealthCheck returns service health
func (h *Handlers) HealthCheck(w http.ResponseWriter, r *http.Request) {
	respondJSON(w, http.StatusOK, map[string]string{
		"status":  "healthy",
		"service": "auction",
	})
}

// RunAuction handles auction requests
func (h *Handlers) RunAuction(w http.ResponseWriter, r *http.Request) {
	var req bidding.BidRequest

	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		respondError(w, http.StatusBadRequest, "Invalid request")
		return
	}

	// Validate request
	if req.RequestID == "" || req.PlacementID == "" {
		respondError(w, http.StatusBadRequest, "Missing required fields")
		return
	}

	sdk, err := auth.RequireContext(r.Context())
	if err != nil {
		respondError(w, http.StatusUnauthorized, "Missing API key")
		return
	}

	// Ensure placement belongs to the key's app.
	if ok, err := auth.PlacementBelongsToApp(r.Context(), h.pgPool, req.PlacementID, sdk.AppID); err != nil || !ok {
		respondError(w, http.StatusForbidden, "Placement not valid for this app")
		return
	}

	// Run auction
	result, err := h.auctionEngine.RunAuction(r.Context(), req)
	if err != nil {
		log.WithError(err).Error("Auction failed")
		respondError(w, http.StatusInternalServerError, "Auction failed")
		return
	}

	respondJSON(w, http.StatusOK, result)
}

// ReceiveBid handles S2S bid responses
func (h *Handlers) ReceiveBid(w http.ResponseWriter, r *http.Request) {
	var bid bidding.BidResponse

	if err := json.NewDecoder(r.Body).Decode(&bid); err != nil {
		respondError(w, http.StatusBadRequest, "Invalid bid")
		return
	}

	// Process bid (store in Redis, etc.)
	// This would be handled by a bid queue in production

	respondJSON(w, http.StatusOK, map[string]string{
		"status": "received",
		"bid_id": bid.BidID,
	})
}

// GetWaterfall returns waterfall configuration
func (h *Handlers) GetWaterfall(w http.ResponseWriter, r *http.Request) {
	vars := mux.Vars(r)
	placementID := vars["placement"]

	sdk, err := auth.RequireContext(r.Context())
	if err != nil {
		respondError(w, http.StatusUnauthorized, "Missing API key")
		return
	}

	if ok, err := auth.PlacementBelongsToApp(r.Context(), h.pgPool, placementID, sdk.AppID); err != nil || !ok {
		respondError(w, http.StatusForbidden, "Placement not valid for this app")
		return
	}

	config, err := h.waterfallManager.GetWaterfall(r.Context(), placementID)
	if err != nil {
		respondError(w, http.StatusInternalServerError, "Failed to fetch waterfall")
		return
	}

	respondJSON(w, http.StatusOK, config)
}

// GetMediationDebugEvents returns last-N mediation debugger events (sanitized) for a placement.
// Query params:
//
//	placement_id: string (optional; empty returns events stored under unknown bucket)
//	n: int (optional; default 50, max 200)
func (h *Handlers) GetMediationDebugEvents(w http.ResponseWriter, r *http.Request) {
	q := r.URL.Query()
	placementID := q.Get("placement_id")
	n := 50
	if ns := q.Get("n"); ns != "" {
		if v, err := strconv.Atoi(ns); err == nil && v > 0 {
			if v > 200 {
				v = 200
			}
			n = v
		}
	}
	events := bidders.GetLastDebugEvents(placementID, n)
	respondAdminJSON(w, http.StatusOK, map[string]any{
		"success": true,
		"data":    events,
	})
}

// GetAdapterMetrics returns a read-only snapshot of per-adapter metrics (requests, outcomes, percentiles).
// Route: GET /v1/metrics/adapters
func (h *Handlers) GetAdapterMetrics(w http.ResponseWriter, r *http.Request) {
	snaps := bidders.GetAdapterMetricsSnapshot()
	respondAdminJSON(w, http.StatusOK, map[string]any{
		"success": true,
		"data":    snaps,
	})
}

// GetAdapterMetricsTimeSeries returns time-series buckets per adapter.
// Routes:
//   - GET /v1/metrics/adapters/timeseries?days=7 (legacy shape: []AdapterSeriesSnapshot)
//   - GET /v1/metrics/adapters/timeseries?windows=5m,1h,24h (new additive shape: map[string][]AdapterSeriesSnapshot)
func (h *Handlers) GetAdapterMetricsTimeSeries(w http.ResponseWriter, r *http.Request) {
	q := r.URL.Query()
	if ws := strings.TrimSpace(q.Get("windows")); ws != "" {
		// Multi-window mode (additive, backward-compatible)
		parts := strings.Split(ws, ",")
		out := map[string]any{}
		for _, p := range parts {
			wstr := strings.TrimSpace(p)
			if wstr == "" {
				continue
			}
			dur, err := time.ParseDuration(wstr)
			if err != nil {
				// Accept shorthands: 1h, 24h, 5m are parseable; ignore invalid tokens
				continue
			}
			snaps := bidders.GetTimeSeriesSnapshot(dur)
			out[wstr] = snaps
		}
		respondAdminJSON(w, http.StatusOK, map[string]any{"success": true, "data": out})
		return
	}
	// Legacy single-window by days (default 7, max 14)
	days := 7
	if s := q.Get("days"); s != "" {
		if v, err := strconv.Atoi(s); err == nil && v > 0 && v <= 14 {
			days = v
		}
	}
	maxAge := time.Duration(days) * 24 * time.Hour
	snaps := bidders.GetTimeSeriesSnapshot(maxAge)
	respondAdminJSON(w, http.StatusOK, map[string]any{
		"success": true,
		"data":    snaps,
	})
}

// GetAdapterSLO returns current SLO status per adapter for 1h and 24h windows.
// Route: GET /v1/metrics/slo
func (h *Handlers) GetAdapterSLO(w http.ResponseWriter, r *http.Request) {
	status1h := bidders.EvaluateSLO(1 * time.Hour)
	status24h := bidders.EvaluateSLO(24 * time.Hour)
	respondJSON(w, http.StatusOK, map[string]any{
		"success": true,
		"data": map[string]any{
			"window_1h":  status1h,
			"window_24h": status24h,
		},
	})
}

// GetObservabilitySnapshot aggregates SLO status with optional last-N debugger events for a placement.
// Route: GET /v1/metrics/overview?placement_id=&n=
func (h *Handlers) GetObservabilitySnapshot(w http.ResponseWriter, r *http.Request) {
	q := r.URL.Query()
	placementID := q.Get("placement_id")
	n := 50
	if ns := q.Get("n"); ns != "" {
		if v, err := strconv.Atoi(ns); err == nil && v > 0 {
			if v > 200 {
				v = 200
			}
			n = v
		}
	}
	status1h := bidders.EvaluateSLO(1 * time.Hour)
	status24h := bidders.EvaluateSLO(24 * time.Hour)
	var events []bidders.DebugEvent
	if placementID != "" {
		events = bidders.GetLastDebugEvents(placementID, n)
	}
	respondJSON(w, http.StatusOK, map[string]any{
		"success": true,
		"data": map[string]any{
			"slo": map[string]any{
				"window_1h":  status1h,
				"window_24h": status24h,
			},
			"debugger": map[string]any{
				"placement_id": placementID,
				"events":       events,
			},
		},
	})
}

// Helper functions

// respondAdminJSON writes an envelope with schema_version for admin endpoints
func respondAdminJSON(w http.ResponseWriter, statusCode int, payload map[string]any) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(statusCode)
	if payload == nil {
		payload = map[string]any{}
	}
	payload["schema_version"] = 1
	if _, ok := payload["success"]; !ok {
		payload["success"] = true
	}
	_ = json.NewEncoder(w).Encode(payload)
}

func respondJSON(w http.ResponseWriter, statusCode int, data interface{}) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(statusCode)
	json.NewEncoder(w).Encode(data)
}

func respondError(w http.ResponseWriter, statusCode int, message string) {
	respondJSON(w, statusCode, map[string]string{
		"error": message,
	})
}
