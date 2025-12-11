package auth

import (
	"context"

	"github.com/jackc/pgx/v5"
	"github.com/jackc/pgx/v5/pgxpool"
)

// PlacementBelongsToApp ensures the requested placement is scoped to the SDK key's app.
func PlacementBelongsToApp(ctx context.Context, pool *pgxpool.Pool, placementID string, appID string) (bool, error) {
	row := pool.QueryRow(ctx, `SELECT 1 FROM placements WHERE id = $1 AND app_id = $2 LIMIT 1`, placementID, appID)
	var one int
	if err := row.Scan(&one); err != nil {
		if err == pgx.ErrNoRows {
			return false, nil
		}
		return false, err
	}
	return true, nil
}
