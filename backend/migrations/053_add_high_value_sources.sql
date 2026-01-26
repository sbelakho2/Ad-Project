-- Migration: 053_add_high_value_sources.sql
-- Adds high-intent sources for the autonomous sales system

-- 1. "Similar App" sources (Competitor lookalikes)
-- Targeting apps similar to popular games (high ad revenue potential)
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status)
VALUES 
  ('itunes', 'Casual Games - US', 'match 3', 'Games', 'us', 'active'),
  ('itunes', 'Hypercasual - US', 'runner game', 'Games', 'us', 'active'),
  ('itunes', 'Puzzle Games - US', 'puzzle', 'Games', 'us', 'active')
ON CONFLICT DO NOTHING;

-- 2. "Keyword" sources (Monetization intent)
-- Apps that use keywords suggesting they are ad-supported or looking for growth
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status)
VALUES 
  ('itunes', 'Ad-Supported Apps', 'free game with ads', 'Games', 'us', 'active'),
  ('itunes', 'Multiplayer Games', 'online multiplayer', 'Games', 'us', 'active')
ON CONFLICT DO NOTHING;
