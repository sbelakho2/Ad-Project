-- Migration: 051_rename_contacts_table.sql
-- Renames sales_contacts to sa_contacts for consistency with V2 schema
-- This fixes the mismatch where code references sa_contacts but table was sales_contacts

-- ============================================================================
-- 1. RENAME MAIN TABLE
-- ============================================================================

-- Check if sa_contacts already exists (idempotent migration)
DO $$
BEGIN
  -- Only rename if sales_contacts exists and sa_contacts doesn't
  IF EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name = 'sales_contacts')
     AND NOT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name = 'sa_contacts') THEN
    ALTER TABLE sales_contacts RENAME TO sa_contacts;
    RAISE NOTICE 'Renamed sales_contacts to sa_contacts';
  ELSIF EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name = 'sa_contacts') THEN
    RAISE NOTICE 'sa_contacts already exists, skipping rename';
  ELSE
    -- Create the table if neither exists
    CREATE TABLE sa_contacts (
      id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
      company_id UUID,
      prospect_id UUID,
      
      -- Contact info
      email VARCHAR(255),
      email_verified BOOLEAN DEFAULT false,
      email_verification_status VARCHAR(30) DEFAULT 'unverified',
      
      first_name VARCHAR(100),
      last_name VARCHAR(100),
      full_name VARCHAR(255),
      
      -- Professional info
      job_title VARCHAR(255),
      department VARCHAR(100),
      seniority VARCHAR(50),
      
      -- Contact channels
      linkedin_url TEXT,
      github_username VARCHAR(100),
      twitter_handle VARCHAR(100),
      
      -- Scoring
      contact_score INTEGER DEFAULT 0,
      reachability_score INTEGER DEFAULT 0,
      score INTEGER DEFAULT 0,
      
      -- Status
      status VARCHAR(30) DEFAULT 'discovered',
      opted_out_at TIMESTAMP,
      
      -- Discovery
      discovery_source VARCHAR(100),
      discovered_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
      
      -- Sales V2 additions
      source_id UUID,
      app_store VARCHAR(50),
      app_id VARCHAR(255),
      app_genre VARCHAR(100),
      icebreaker TEXT,
      website_meta_description TEXT,
      app_ads_txt_content TEXT,
      website TEXT,
      
      created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
      updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
      
      UNIQUE(email)
    );
    
    CREATE INDEX IF NOT EXISTS idx_sa_contacts_email ON sa_contacts(email);
    CREATE INDEX IF NOT EXISTS idx_sa_contacts_status ON sa_contacts(status);
    CREATE INDEX IF NOT EXISTS idx_sa_contacts_score ON sa_contacts(score DESC);
    CREATE INDEX IF NOT EXISTS idx_sa_contacts_source ON sa_contacts(source_id);
    
    RAISE NOTICE 'Created new sa_contacts table';
  END IF;
END $$;

-- ============================================================================
-- 2. ADD MISSING COLUMNS (Idempotent)
-- ============================================================================

-- Ensure website column exists (needed for scoring)
ALTER TABLE sa_contacts ADD COLUMN IF NOT EXISTS website TEXT;

-- Add company_name if missing (might be full_name in older schema)
DO $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM information_schema.columns 
                 WHERE table_name = 'sa_contacts' AND column_name = 'company_name') THEN
    ALTER TABLE sa_contacts ADD COLUMN company_name VARCHAR(255);
    -- Try to populate from full_name if it exists
    IF EXISTS (SELECT 1 FROM information_schema.columns 
               WHERE table_name = 'sa_contacts' AND column_name = 'full_name') THEN
      UPDATE sa_contacts SET company_name = full_name WHERE company_name IS NULL;
    END IF;
  END IF;
END $$;

-- Ensure all V2 columns exist
ALTER TABLE sa_contacts ADD COLUMN IF NOT EXISTS source_id UUID;
ALTER TABLE sa_contacts ADD COLUMN IF NOT EXISTS app_store VARCHAR(50);
ALTER TABLE sa_contacts ADD COLUMN IF NOT EXISTS app_id VARCHAR(255);
ALTER TABLE sa_contacts ADD COLUMN IF NOT EXISTS app_genre VARCHAR(100);
ALTER TABLE sa_contacts ADD COLUMN IF NOT EXISTS icebreaker TEXT;
ALTER TABLE sa_contacts ADD COLUMN IF NOT EXISTS website_meta_description TEXT;
ALTER TABLE sa_contacts ADD COLUMN IF NOT EXISTS app_ads_txt_content TEXT;
ALTER TABLE sa_contacts ADD COLUMN IF NOT EXISTS score INTEGER DEFAULT 0;
ALTER TABLE sa_contacts ADD COLUMN IF NOT EXISTS email_verified BOOLEAN DEFAULT false;
ALTER TABLE sa_contacts ADD COLUMN IF NOT EXISTS email_verification_status VARCHAR(30) DEFAULT 'unverified';
ALTER TABLE sa_contacts ADD COLUMN IF NOT EXISTS updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP;

-- ============================================================================
-- 3. CREATE INDEXES (Idempotent)
-- ============================================================================

CREATE INDEX IF NOT EXISTS idx_sa_contacts_email ON sa_contacts(email);
CREATE INDEX IF NOT EXISTS idx_sa_contacts_status ON sa_contacts(status);
CREATE INDEX IF NOT EXISTS idx_sa_contacts_score ON sa_contacts(score DESC);
CREATE INDEX IF NOT EXISTS idx_sa_contacts_source ON sa_contacts(source_id);
CREATE INDEX IF NOT EXISTS idx_sa_contacts_verified ON sa_contacts(email_verified);

-- ============================================================================
-- 4. ADD COMMENT
-- ============================================================================

COMMENT ON TABLE sa_contacts IS 'Sales automation contacts - leads discovered from app stores and other sources';
