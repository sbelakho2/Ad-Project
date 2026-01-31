-- Migration 061: Add Proof Strength Tracking for Audit Hardening
-- 
-- AUDIT TRANSPARENCY FIX
-- 
-- Problem: When receipt_hash is missing (SDK sync delay or data loss), the proofs
-- system falls back to sha256(request_id). This weakens cryptographic verification
-- since the platform controls request_ids and could theoretically forge entries.
--
-- Solution: Track which proofs used the fallback mechanism so auditors can
-- understand the cryptographic strength of each day's/month's proof.
--
-- Metrics:
--   strong_proofs: Count of entries using actual receipt_hash (network-signed)
--   weak_proofs: Count of entries using sha256(request_id) fallback
--   strength_pct: Percentage of strong proofs (target: >95%)

BEGIN;

-- Step 1: Add proof strength columns to proofs_daily_roots
ALTER TABLE proofs_daily_roots 
ADD COLUMN IF NOT EXISTS strong_proofs INTEGER DEFAULT 0;

ALTER TABLE proofs_daily_roots 
ADD COLUMN IF NOT EXISTS weak_proofs INTEGER DEFAULT 0;

ALTER TABLE proofs_daily_roots 
ADD COLUMN IF NOT EXISTS strength_pct DECIMAL(5,2) DEFAULT 100.00;

-- Step 2: Add proof strength columns to proofs_monthly_digest
ALTER TABLE proofs_monthly_digest 
ADD COLUMN IF NOT EXISTS strong_proofs INTEGER DEFAULT 0;

ALTER TABLE proofs_monthly_digest 
ADD COLUMN IF NOT EXISTS weak_proofs INTEGER DEFAULT 0;

ALTER TABLE proofs_monthly_digest 
ADD COLUMN IF NOT EXISTS strength_pct DECIMAL(5,2) DEFAULT 100.00;

-- Step 3: Add check constraint to ensure strength_pct is valid
ALTER TABLE proofs_daily_roots
ADD CONSTRAINT chk_daily_strength_pct 
CHECK (strength_pct >= 0 AND strength_pct <= 100);

ALTER TABLE proofs_monthly_digest
ADD CONSTRAINT chk_monthly_strength_pct 
CHECK (strength_pct >= 0 AND strength_pct <= 100);

-- Step 4: Add index for querying low-strength proof days (for audit alerts)
CREATE INDEX IF NOT EXISTS idx_proofs_daily_low_strength 
ON proofs_daily_roots(strength_pct) 
WHERE strength_pct < 95;

-- Step 5: Add comments for documentation
COMMENT ON COLUMN proofs_daily_roots.strong_proofs IS 
'Count of proofs using actual receipt_hash from network (cryptographically strong)';

COMMENT ON COLUMN proofs_daily_roots.weak_proofs IS 
'Count of proofs using sha256(request_id) fallback (potentially forgeable)';

COMMENT ON COLUMN proofs_daily_roots.strength_pct IS 
'Percentage of strong proofs. Target >95%. Values <95% should be flagged for audit.';

COMMENT ON COLUMN proofs_monthly_digest.strong_proofs IS 
'Aggregated count of strong proofs across all days in the month';

COMMENT ON COLUMN proofs_monthly_digest.weak_proofs IS 
'Aggregated count of weak proofs across all days in the month';

COMMENT ON COLUMN proofs_monthly_digest.strength_pct IS 
'Average proof strength percentage for the month. Values <95% indicate potential audit concerns.';

-- Step 6: Create a view for audit reports
CREATE OR REPLACE VIEW vw_proof_strength_audit AS
SELECT 
  day,
  merkle_root,
  strong_proofs,
  weak_proofs,
  (strong_proofs + weak_proofs) AS total_proofs,
  strength_pct,
  CASE 
    WHEN strength_pct >= 99 THEN 'EXCELLENT'
    WHEN strength_pct >= 95 THEN 'GOOD'
    WHEN strength_pct >= 80 THEN 'WARNING'
    ELSE 'CRITICAL'
  END AS strength_grade,
  CASE 
    WHEN strength_pct < 95 THEN 
      'Low proof strength: ' || weak_proofs || ' proofs used request_id fallback'
    ELSE NULL
  END AS audit_note
FROM proofs_daily_roots
ORDER BY day DESC;

COMMENT ON VIEW vw_proof_strength_audit IS 
'Audit view showing daily proof strength metrics. 
CRITICAL/WARNING grades indicate proofs that may be less cryptographically rigorous.';

COMMIT;
