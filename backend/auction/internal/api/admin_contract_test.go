package api

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"strings"
	"testing"
	"time"

	"github.com/gorilla/mux"

	"github.com/rivalapexmediation/auction/internal/bidders"
)

// buildTestRouter wires only admin routes with middlewares applied as in main.go
func buildTestRouter(h *Handlers) *mux.Router {
	r := mux.NewRouter()
	adminDebug := r.PathPrefix("/v1/debug").Subrouter()
	adminDebug.Use(AdminIPAllowlistMiddleware)
	adminDebug.Use(AdminAuthMiddleware)
	adminDebug.Use(AdminRateLimitMiddleware)
	adminDebug.HandleFunc("/mediation", h.GetMediationDebugEvents).Methods("GET", "OPTIONS")

	adminMetrics := r.PathPrefix("/v1/metrics").Subrouter()
	adminMetrics.Use(AdminIPAllowlistMiddleware)
	adminMetrics.Use(AdminAuthMiddleware)
	adminMetrics.Use(AdminRateLimitMiddleware)
	adminMetrics.HandleFunc("/adapters", h.GetAdapterMetrics).Methods("GET", "OPTIONS")
	adminMetrics.HandleFunc("/adapters/timeseries", h.GetAdapterMetricsTimeSeries).Methods("GET", "OPTIONS")
	adminMetrics.HandleFunc("/slo", h.GetAdapterSLO).Methods("GET", "OPTIONS")
	adminMetrics.HandleFunc("/overview", h.GetObservabilitySnapshot).Methods("GET", "OPTIONS")
	return r
}

// seed some metrics and debugger events
func seedObservability() {
	bidders.SetTimeSeriesAggregator(bidders.NewTimeSeriesAggregator(1*time.Second, 60*time.Second))
	// record some signals for two adapters
	for i := 0; i < 10; i++ {
		bidders.RecordForTest("admob", 5, 1, 1, 3, []float64{10, 20, 50})
		bidders.RecordForTest("applovin", 7, 0, 0, 7, []float64{30, 60, 120})
	}
	// debugger with a few events
	bidders.SetDebugger(bidders.NewInMemoryDebugger(10))
	bidders.CaptureDebugEvent(bidders.DebugEvent{PlacementID: "pl-1", RequestID: "r-1", Adapter: "admob", Outcome: "success", CreatedAt: time.Now()})
	bidders.CaptureDebugEvent(bidders.DebugEvent{PlacementID: "pl-1", RequestID: "r-2", Adapter: "applovin", Outcome: "no_bid", Reason: "status_204", CreatedAt: time.Now()})
}

// helper to set env and return a cleanup
func setenv(k, v string) func() {
	old, had := os.LookupEnv(k)
	_ = os.Setenv(k, v)
	return func() {
		if had {
			_ = os.Setenv(k, old)
		} else {
			_ = os.Unsetenv(k)
		}
	}
}

// Admin smoke: ensure envelopes and shapes are correct with and without security middlewares enabled.
func TestAdminAPI_Contracts(t *testing.T) {
	// Prepare handlers and router
	seedObservability()
	h := NewHandlers(nil, nil, nil) // handlers don't strictly need engine for read-only admin endpoints
	r := buildTestRouter(h)

	// Case 1: no auth/allowlist/ratelimit set (back-compat path)
	t.Run("backward_compatible_no_auth", func(t *testing.T) {
		// /v1/metrics/adapters
		req := httptest.NewRequest("GET", "/v1/metrics/adapters", nil)
		w := httptest.NewRecorder()
		r.ServeHTTP(w, req)
		if w.Code != http.StatusOK {
			t.Fatalf("expected 200, got %d", w.Code)
		}
		body := w.Body.String()
		var m map[string]any
		if err := json.Unmarshal([]byte(body), &m); err != nil {
			t.Fatalf("invalid JSON: %v", err)
		}
		if _, ok := m["schema_version"]; !ok {
			t.Fatalf("missing schema_version in %s", body)
		}
		if ok, ok2 := m["success"].(bool); !ok2 || !ok {
			t.Fatalf("expected success=true, got %v in %s", m["success"], body)
		}

		// legacy timeseries by days
		req = httptest.NewRequest("GET", "/v1/metrics/adapters/timeseries?days=1", nil)
		w = httptest.NewRecorder()
		r.ServeHTTP(w, req)
		if w.Code != http.StatusOK {
			t.Fatalf("expected 200, got %d", w.Code)
		}
		if !strings.Contains(w.Body.String(), "\"data\":") {
			t.Fatalf("expected data field in response")
		}

		// multi-window timeseries
		req = httptest.NewRequest("GET", "/v1/metrics/adapters/timeseries?windows=5m,1h", nil)
		w = httptest.NewRecorder()
		r.ServeHTTP(w, req)
		if w.Code != http.StatusOK {
			t.Fatalf("expected 200, got %d", w.Code)
		}
		if !strings.Contains(w.Body.String(), "\"5m\"") {
			t.Fatalf("expected multi-window keys in response: %s", w.Body.String())
		}

		// SLO endpoint
		req = httptest.NewRequest("GET", "/v1/metrics/slo", nil)
		w = httptest.NewRecorder()
		r.ServeHTTP(w, req)
		if w.Code != http.StatusOK {
			t.Fatalf("expected 200, got %d", w.Code)
		}
		if !strings.Contains(w.Body.String(), "window_1h") {
			t.Fatalf("expected slo windows in response")
		}

		// Debug events
		req = httptest.NewRequest("GET", "/v1/debug/mediation?placement_id=pl-1&n=2", nil)
		w = httptest.NewRecorder()
		r.ServeHTTP(w, req)
		if w.Code != http.StatusOK {
			t.Fatalf("expected 200, got %d", w.Code)
		}
		var dbg map[string]any
		if err := json.Unmarshal(w.Body.Bytes(), &dbg); err != nil {
			t.Fatalf("invalid JSON: %v", err)
		}
		if dbg["success"] != true {
			t.Fatalf("expected success true in debug response")
		}
		// data should be an array of events
		if _, ok := dbg["data"].([]any); !ok {
			t.Fatalf("expected data to be an array in %s", w.Body.String())
		}
	})

	// Case 2: with auth token + allowlist + rate limit
	t.Run("secured_with_middlewares", func(t *testing.T) {
		unset1 := setenv("ADMIN_API_BEARER", "test-token")
		defer unset1()
		unset2 := setenv("ADMIN_IP_ALLOWLIST", "127.0.0.1/32")
		defer unset2()
		unset3 := setenv("ADMIN_RATELIMIT_WINDOW", "10s")
		defer unset3()
		unset4 := setenv("ADMIN_RATELIMIT_BURST", "1")
		defer unset4()

		// Rebuild router after envs so middlewares are enabled
		r = buildTestRouter(h)

		req := httptest.NewRequest("GET", "/v1/metrics/adapters", nil)
		req.Header.Set("Authorization", "Bearer test-token")
		req.RemoteAddr = "127.0.0.1:12345"
		w := httptest.NewRecorder()
		r.ServeHTTP(w, req)
		if w.Code != http.StatusOK {
			t.Fatalf("expected 200 with auth+allowlist, got %d: %s", w.Code, w.Body.String())
		}

		// Validate success envelope with security middlewares enabled
		w = httptest.NewRecorder()
		r.ServeHTTP(w, req)
		if w.Code != http.StatusOK {
			t.Fatalf("expected 200 with auth+allowlist, got %d", w.Code)
		}
		var em map[string]any
		if err := json.Unmarshal(w.Body.Bytes(), &em); err != nil {
			t.Fatalf("invalid JSON: %v", err)
		}
		if _, ok := em["schema_version"]; !ok {
			t.Fatalf("missing schema_version in %s", w.Body.String())
		}
		if ok, ok2 := em["success"].(bool); !ok2 || !ok {
			t.Fatalf("expected success=true, got %v in %s", em["success"], w.Body.String())
		}
	})
}
