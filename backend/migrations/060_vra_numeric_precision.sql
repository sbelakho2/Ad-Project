-- Migration: 060_vra_numeric_precision.sql
-- Purpose: Upgrade amount and revenue columns from float8/NUMERIC(12,2) to NUMERIC(18,6)
--          for native database-level precision (eliminating floating-point representation errors).
--
-- Context:
--   - The application uses Decimal.js for financial calculations, but the database
--     stores some values as float8 (double precision) which loses precision.
--   - NUMERIC(18,6) provides:
--       - 18 total significant digits
--       - 6 decimal places (micro-dollar precision: $0.000001)
--       - Range: -999,999,999,999.999999 to +999,999,999,999.999999 (trillions)
--   - This matches the NUMERIC(18,6) already used in analytics_revenue_events and
--     newer VRA tables, ensuring consistency.
--
-- Impact: 
--   - Zero downtime migration (ADD COLUMN + copy + DROP is avoided for production safety)
--   - Uses ALTER COLUMN TYPE which may require a brief table-level lock on large tables
--   - Backwards compatible: existing values are cast with ROUND() to avoid truncation warnings
--
-- Affected Tables (legacy float8 or NUMERIC(12,2) columns):
--   - revenue_events.revenue (NUMERIC(12,2) -> NUMERIC(18,6))
--   - revenue_events.amount (NUMERIC(12,2) -> NUMERIC(18,6))
--   - payment_transactions.amount (NUMERIC(12,2) -> NUMERIC(18,6))
--   - recon_expected.expected_value (DECIMAL(15,6) already, but verify)
--   - vra_reconciliations.variance_amount (BIGINT micros, consider adding USD decimal view)

-- Start transaction
BEGIN;

-- ============================================================================
-- 1. revenue_events table upgrades
-- ============================================================================

-- Check if column exists and upgrade type only if needed
DO $$
BEGIN
    -- Upgrade revenue column if it exists and is not already NUMERIC(18,6)
    IF EXISTS (
        SELECT 1 FROM information_schema.columns 
        WHERE table_name = 'revenue_events' 
        AND column_name = 'revenue'
        AND (numeric_precision IS NULL OR numeric_precision < 18 OR numeric_scale < 6)
    ) THEN
        EXECUTE 'ALTER TABLE revenue_events ALTER COLUMN revenue TYPE NUMERIC(18,6) USING ROUND(revenue::numeric, 6)';
        RAISE NOTICE 'Upgraded revenue_events.revenue to NUMERIC(18,6)';
    END IF;

    -- Upgrade amount column if it exists and is not already NUMERIC(18,6)
    IF EXISTS (
        SELECT 1 FROM information_schema.columns 
        WHERE table_name = 'revenue_events' 
        AND column_name = 'amount'
        AND (numeric_precision IS NULL OR numeric_precision < 18 OR numeric_scale < 6)
    ) THEN
        EXECUTE 'ALTER TABLE revenue_events ALTER COLUMN amount TYPE NUMERIC(18,6) USING ROUND(amount::numeric, 6)';
        RAISE NOTICE 'Upgraded revenue_events.amount to NUMERIC(18,6)';
    END IF;
END $$;

-- ============================================================================
-- 2. payment_transactions table upgrades (if exists)
-- ============================================================================

DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM information_schema.columns 
        WHERE table_name = 'payment_transactions' 
        AND column_name = 'amount'
        AND (numeric_precision IS NULL OR numeric_precision < 18 OR numeric_scale < 6)
    ) THEN
        EXECUTE 'ALTER TABLE payment_transactions ALTER COLUMN amount TYPE NUMERIC(18,6) USING ROUND(amount::numeric, 6)';
        RAISE NOTICE 'Upgraded payment_transactions.amount to NUMERIC(18,6)';
    END IF;
END $$;

-- ============================================================================
-- 3. Core VRA tables - ensure consistency
-- ============================================================================

-- recon_expected: expected_value might be DECIMAL(15,6) from migration, upgrade to NUMERIC(18,6)
DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM information_schema.columns 
        WHERE table_name = 'recon_expected' 
        AND column_name = 'expected_value'
        AND (numeric_precision IS NULL OR numeric_precision < 18)
    ) THEN
        EXECUTE 'ALTER TABLE recon_expected ALTER COLUMN expected_value TYPE NUMERIC(18,6) USING ROUND(expected_value::numeric, 6)';
        RAISE NOTICE 'Upgraded recon_expected.expected_value to NUMERIC(18,6)';
    END IF;
END $$;

-- recon_statements_norm: paid, gross, ivt_adjustments, original_amount
DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM information_schema.columns 
        WHERE table_name = 'recon_statements_norm' 
        AND column_name = 'paid'
        AND (numeric_precision IS NULL OR numeric_precision < 18)
    ) THEN
        EXECUTE 'ALTER TABLE recon_statements_norm ALTER COLUMN paid TYPE NUMERIC(18,6) USING ROUND(paid::numeric, 6)';
        EXECUTE 'ALTER TABLE recon_statements_norm ALTER COLUMN gross TYPE NUMERIC(18,6) USING ROUND(gross::numeric, 6)';
        EXECUTE 'ALTER TABLE recon_statements_norm ALTER COLUMN ivt_adjustments TYPE NUMERIC(18,6) USING ROUND(ivt_adjustments::numeric, 6)';
        EXECUTE 'ALTER TABLE recon_statements_norm ALTER COLUMN original_amount TYPE NUMERIC(18,6) USING ROUND(original_amount::numeric, 6)';
        RAISE NOTICE 'Upgraded recon_statements_norm monetary columns to NUMERIC(18,6)';
    END IF;
END $$;

-- vra_reconciliation_runs: expected_usd, paid_usd, variance_usd
DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM information_schema.columns 
        WHERE table_name = 'vra_reconciliation_runs' 
        AND column_name = 'expected_usd'
        AND (numeric_precision IS NULL OR numeric_precision < 18)
    ) THEN
        EXECUTE 'ALTER TABLE vra_reconciliation_runs ALTER COLUMN expected_usd TYPE NUMERIC(18,6) USING ROUND(expected_usd::numeric, 6)';
        EXECUTE 'ALTER TABLE vra_reconciliation_runs ALTER COLUMN paid_usd TYPE NUMERIC(18,6) USING ROUND(paid_usd::numeric, 6)';
        EXECUTE 'ALTER TABLE vra_reconciliation_runs ALTER COLUMN variance_usd TYPE NUMERIC(18,6) USING ROUND(variance_usd::numeric, 6)';
        RAISE NOTICE 'Upgraded vra_reconciliation_runs monetary columns to NUMERIC(18,6)';
    END IF;
END $$;

-- vra_sync_history: revenue_usd_total
DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM information_schema.columns 
        WHERE table_name = 'vra_sync_history' 
        AND column_name = 'revenue_usd_total'
        AND (numeric_precision IS NULL OR numeric_precision < 18)
    ) THEN
        EXECUTE 'ALTER TABLE vra_sync_history ALTER COLUMN revenue_usd_total TYPE NUMERIC(18,6) USING ROUND(revenue_usd_total::numeric, 6)';
        RAISE NOTICE 'Upgraded vra_sync_history.revenue_usd_total to NUMERIC(18,6)';
    END IF;
END $$;

-- vra_alerts: amount_usd
DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM information_schema.columns 
        WHERE table_name = 'vra_alerts' 
        AND column_name = 'amount_usd'
        AND (numeric_precision IS NULL OR numeric_precision < 18)
    ) THEN
        EXECUTE 'ALTER TABLE vra_alerts ALTER COLUMN amount_usd TYPE NUMERIC(18,6) USING ROUND(amount_usd::numeric, 6)';
        RAISE NOTICE 'Upgraded vra_alerts.amount_usd to NUMERIC(18,6)';
    END IF;
END $$;

-- ============================================================================
-- 4. recon_deltas table - critical for VRA accuracy
-- ============================================================================

-- Check if recon_deltas exists and has an amount column that needs upgrading
DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM information_schema.columns 
        WHERE table_name = 'recon_deltas' 
        AND column_name = 'amount'
    ) THEN
        -- Check current type
        IF EXISTS (
            SELECT 1 FROM information_schema.columns 
            WHERE table_name = 'recon_deltas' 
            AND column_name = 'amount'
            AND (data_type = 'double precision' OR data_type = 'real' 
                 OR (data_type = 'numeric' AND (numeric_precision IS NULL OR numeric_precision < 18)))
        ) THEN
            EXECUTE 'ALTER TABLE recon_deltas ALTER COLUMN amount TYPE NUMERIC(18,6) USING ROUND(amount::numeric, 6)';
            RAISE NOTICE 'Upgraded recon_deltas.amount to NUMERIC(18,6)';
        END IF;
    END IF;
END $$;

-- ============================================================================
-- 5. Analytics tables - ensure consistency with revenue tracking
-- ============================================================================

-- analytics_revenue_events should already be NUMERIC(18,6) but verify
DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM information_schema.columns 
        WHERE table_name = 'analytics_revenue_events' 
        AND column_name = 'revenue_usd'
        AND (numeric_precision IS NULL OR numeric_precision < 18)
    ) THEN
        EXECUTE 'ALTER TABLE analytics_revenue_events ALTER COLUMN revenue_usd TYPE NUMERIC(18,6) USING ROUND(revenue_usd::numeric, 6)';
        RAISE NOTICE 'Upgraded analytics_revenue_events.revenue_usd to NUMERIC(18,6)';
    END IF;

    IF EXISTS (
        SELECT 1 FROM information_schema.columns 
        WHERE table_name = 'analytics_revenue_events' 
        AND column_name = 'revenue_original'
        AND (numeric_precision IS NULL OR numeric_precision < 18)
    ) THEN
        EXECUTE 'ALTER TABLE analytics_revenue_events ALTER COLUMN revenue_original TYPE NUMERIC(18,6) USING ROUND(revenue_original::numeric, 6)';
        RAISE NOTICE 'Upgraded analytics_revenue_events.revenue_original to NUMERIC(18,6)';
    END IF;
END $$;

-- ============================================================================
-- 6. Add comment documenting the standard
-- ============================================================================

COMMENT ON SCHEMA public IS 
'Ad Project database schema. Monetary columns use NUMERIC(18,6) standard for micro-dollar precision. 
Updated by migration 060_vra_numeric_precision.sql.';

-- ============================================================================
-- 7. Create validation view for monitoring precision issues
-- ============================================================================

CREATE OR REPLACE VIEW vra_precision_audit AS
SELECT 
    table_name,
    column_name,
    data_type,
    numeric_precision,
    numeric_scale,
    CASE 
        WHEN data_type = 'numeric' AND numeric_precision >= 18 AND numeric_scale >= 6 THEN 'OK'
        WHEN data_type IN ('double precision', 'real') THEN 'NEEDS_MIGRATION'
        WHEN data_type = 'numeric' AND (numeric_precision < 18 OR numeric_scale < 6) THEN 'INSUFFICIENT_PRECISION'
        WHEN data_type = 'bigint' AND column_name LIKE '%micros' THEN 'OK_MICROS'
        ELSE 'REVIEW'
    END AS status
FROM information_schema.columns
WHERE table_schema = 'public'
  AND (column_name LIKE '%amount%' 
       OR column_name LIKE '%revenue%' 
       OR column_name LIKE '%paid%'
       OR column_name LIKE '%gross%'
       OR column_name LIKE '%variance%')
  AND table_name NOT LIKE 'pg_%'
ORDER BY status DESC, table_name, column_name;

COMMIT;

-- ============================================================================
-- Post-migration verification query (run manually)
-- ============================================================================
-- SELECT * FROM vra_precision_audit WHERE status NOT IN ('OK', 'OK_MICROS');
