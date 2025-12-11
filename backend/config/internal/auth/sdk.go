package auth

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"net/http"
	"strings"

	"github.com/gorilla/mux"
	"github.com/jackc/pgx/v5/pgxpool"
	"golang.org/x/crypto/bcrypt"
)

type ctxKey struct{}

// SDKContext is attached to request context for SDK-authenticated calls.
type SDKContext struct {
	KeyID       string
	PublisherID string
	AppID       string
	Environment string
}

func FromContext(ctx context.Context) (SDKContext, bool) {
	v := ctx.Value(ctxKey{})
	if v == nil {
		return SDKContext{}, false
	}
	sdk, ok := v.(SDKContext)
	return sdk, ok
}

func requireAPIKey(r *http.Request) string {
	h := strings.TrimSpace(r.Header.Get("Authorization"))
	if len(h) >= 7 && strings.EqualFold(h[:7], "Bearer ") {
		k := strings.TrimSpace(h[7:])
		if k != "" {
			return k
		}
	}
	return strings.TrimSpace(r.Header.Get("X-Api-Key"))
}

func sha256Hex(s string) string {
	h := sha256.Sum256([]byte(s))
	return hex.EncodeToString(h[:])
}

type keyRow struct {
	id          string
	publisherID string
	appID       string
	environment string
	secretHash  string
	revokedAt   *string
}

func lookupKey(ctx context.Context, pool *pgxpool.Pool, digest string) (*keyRow, error) {
	row := pool.QueryRow(ctx,
		`SELECT id::text, publisher_id::text, app_id::text, environment, secret, revoked_at::text
		   FROM app_api_keys
		  WHERE secret_digest = $1
		  LIMIT 1`,
		digest,
	)

	var out keyRow
	if err := row.Scan(&out.id, &out.publisherID, &out.appID, &out.environment, &out.secretHash, &out.revokedAt); err != nil {
		return nil, err
	}
	return &out, nil
}

// Middleware enforces the per-app key for SDK endpoints.
// If appIDParam is non-empty, it also enforces the path param matches the key's app.
func Middleware(pool *pgxpool.Pool, appIDParam string) mux.MiddlewareFunc {
	return func(next http.Handler) http.Handler {
		return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			key := requireAPIKey(r)
			if key == "" {
				http.Error(w, `{"error":"Missing API key"}`, http.StatusUnauthorized)
				return
			}

			digest := sha256Hex(key)
			k, err := lookupKey(r.Context(), pool, digest)
			if err != nil || k == nil || k.revokedAt != nil {
				http.Error(w, `{"error":"Invalid API key"}`, http.StatusUnauthorized)
				return
			}

			if err := bcrypt.CompareHashAndPassword([]byte(k.secretHash), []byte(key)); err != nil {
				http.Error(w, `{"error":"Invalid API key"}`, http.StatusUnauthorized)
				return
			}

			if appIDParam != "" {
				vars := mux.Vars(r)
				if want := strings.TrimSpace(vars[appIDParam]); want != "" && want != k.appID {
					http.Error(w, `{"error":"API key not valid for requested app"}`, http.StatusForbidden)
					return
				}
			}

			ctx := context.WithValue(r.Context(), ctxKey{}, SDKContext{
				KeyID:       k.id,
				PublisherID: k.publisherID,
				AppID:       k.appID,
				Environment: k.environment,
			})
			next.ServeHTTP(w, r.WithContext(ctx))
		})
	}
}

var ErrMissingContext = errors.New("missing sdk context")

func RequireContext(ctx context.Context) (SDKContext, error) {
	sdk, ok := FromContext(ctx)
	if !ok {
		return SDKContext{}, ErrMissingContext
	}
	return sdk, nil
}
