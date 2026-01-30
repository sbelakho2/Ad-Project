-- Migration: Add Missing FK Indexes
-- Date: 2025-01-30
-- Description: Add indexes on foreign key columns that were missing indexes.
-- These indexes improve JOIN and DELETE performance on FK relationships.

-- fraud_alerts.placement_id - needed for JOINs and CASCADE deletes
CREATE INDEX IF NOT EXISTS idx_fraud_alerts_placement ON fraud_alerts(placement_id);

-- payouts.transaction_id - needed for JOINs with payment_transactions
CREATE INDEX IF NOT EXISTS idx_payouts_transaction ON payouts(transaction_id);

-- team_members.invited_by - self-referential FK, helps with team hierarchy queries
CREATE INDEX IF NOT EXISTS idx_team_members_invited_by ON team_members(invited_by);

-- audit_logs.user_id - needed for user activity queries
CREATE INDEX IF NOT EXISTS idx_audit_logs_user ON audit_logs(user_id);

-- Also add commonly queried columns that might benefit from indexes
-- These are optional but improve common query patterns:

-- payouts.period_start and period_end - for date range queries
CREATE INDEX IF NOT EXISTS idx_payouts_period ON payouts(period_start, period_end);

-- fraud_alerts.created_at filter + publisher combo
CREATE INDEX IF NOT EXISTS idx_fraud_alerts_publisher_created ON fraud_alerts(publisher_id, created_at DESC);

COMMENT ON INDEX idx_fraud_alerts_placement IS 'FK index for placement_id references';
COMMENT ON INDEX idx_payouts_transaction IS 'FK index for transaction_id references';
COMMENT ON INDEX idx_team_members_invited_by IS 'FK index for invited_by self-reference';
COMMENT ON INDEX idx_audit_logs_user IS 'FK index for user_id references';
