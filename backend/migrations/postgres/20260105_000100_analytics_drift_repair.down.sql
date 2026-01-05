-- Repair analytics schema drift (rollback)
--
-- Down migrations are not routinely used in production.
-- This rollback removes only the objects that are safe to remove:
-- - staging tables
-- - supporting indexes added by the up migration
--
-- It intentionally does NOT drop columns from analytics tables.

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '60s';

DROP TABLE IF EXISTS analytics_impressions_stage;
DROP TABLE IF EXISTS analytics_clicks_stage;
DROP TABLE IF EXISTS analytics_revenue_events_stage;

DROP INDEX IF EXISTS idx_analytics_impressions_observed_event;
DROP INDEX IF EXISTS idx_analytics_clicks_observed_event;
DROP INDEX IF EXISTS idx_analytics_revenue_events_observed_event;
