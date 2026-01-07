-- Add placement configuration storage (JSONB) — backward compatible
ALTER TABLE placements
  ADD COLUMN IF NOT EXISTS config JSONB DEFAULT '{}'::jsonb;
