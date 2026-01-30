-- Generate 10,000+ lead sources for V2 Sales Automation
-- Based on deep research of mobile game developers, publishers, and app categories

-- First, let's see current count
SELECT COUNT(*) AS current_count FROM sa_sources;

-- Create a function to generate sources
DO $$
DECLARE
    -- Companies from Wikipedia research
    companies TEXT[] := ARRAY[
        'Supercell', 'King', 'Rovio', 'Zynga', 'Gameloft', 'Glu Mobile', 'Jam City', 'Scopely',
        'Playrix', 'Peak Games', 'Machine Zone', 'Playtika', 'Moon Active', 'Socialpoint',
        'Kabam', 'Netmarble', 'Nexon', 'miHoYo', 'HoYoverse', 'NetEase', 'Tencent Games',
        'DeNA', 'GREE', 'CyberAgent', 'Cygames', 'Colopl', 'GungHo', 'Com2uS', 'Gamevil',
        'IGG', 'Lilith Games', 'FunPlus', 'Century Games', 'Halfbrick', 'Voodoo', 'Kwalee',
        'Azur Games', 'Lion Studios', 'AppLovin', 'CrazyLabs', 'SayGames', 'Good Job Games',
        'Homa Games', 'TapNation', 'Miniclip', 'Outfit7', 'Imangi Studios', 'Hipster Whale',
        'Electronic Arts', 'Activision', 'Ubisoft', 'Take-Two', 'Epic Games', 'Riot Games',
        'Square Enix', 'Capcom', 'Bandai Namco', 'Konami', 'Sega', 'Nintendo', 'Sony',
        'Wooga', 'InnoGames', 'Goodgame', 'Plarium', 'Huuuge Games', 'Product Madness',
        'Big Fish', 'Aristocrat', 'SciPlay', 'Playstudios', 'DoubleU Games', 'Melsoft',
        'Smilegate', 'NCSoft', 'Krafton', 'Pearl Abyss', 'Kakao Games', 'Devsisters',
        'CD Projekt', 'Paradox', 'Frontier', 'Team17', 'Rebellion', 'Wargaming', 'Gaijin'
    ];
    
    -- Categories
    gp_categories TEXT[] := ARRAY[
        'GAME', 'GAME_ACTION', 'GAME_ADVENTURE', 'GAME_ARCADE', 'GAME_CASUAL',
        'GAME_PUZZLE', 'GAME_RACING', 'GAME_ROLE_PLAYING', 'GAME_SIMULATION',
        'GAME_SPORTS', 'GAME_STRATEGY', 'APPLICATION', 'ENTERTAINMENT', 'SOCIAL',
        'PRODUCTIVITY', 'EDUCATION', 'FINANCE', 'HEALTH_AND_FITNESS', 'LIFESTYLE'
    ];
    
    ios_categories TEXT[] := ARRAY[
        'Games', 'Action Games', 'Adventure Games', 'Arcade Games', 'Casual Games',
        'Puzzle Games', 'Racing Games', 'Role Playing Games', 'Simulation Games',
        'Sports Games', 'Strategy Games', 'Entertainment', 'Social Networking',
        'Productivity', 'Education', 'Finance', 'Health & Fitness', 'Lifestyle'
    ];
    
    -- Regions
    regions TEXT[] := ARRAY[
        'us', 'ca', 'mx', 'gb', 'de', 'fr', 'it', 'es', 'nl', 'se', 'no', 'dk', 'fi',
        'pl', 'pt', 'jp', 'kr', 'cn', 'tw', 'hk', 'sg', 'au', 'nz', 'in', 'id', 'th',
        'br', 'ar', 'ae', 'sa', 'tr', 'ru', 'ua', 'za'
    ];
    
    -- Game genres
    genres TEXT[] := ARRAY[
        'action', 'adventure', 'arcade', 'puzzle', 'strategy', 'rpg', 'simulation',
        'sports', 'racing', 'casual', 'hypercasual', 'idle', 'clicker', 'shooter',
        'battle royale', 'moba', 'card', 'casino', 'match 3', 'word', 'trivia',
        'tower defense', 'city builder', 'farm', 'cooking', 'fashion', 'horror',
        'survival', 'zombie', 'platformer', 'runner', 'fighting', 'war', 'space',
        'fantasy', 'medieval', 'tycoon', 'music', 'rhythm', 'educational', 'kids',
        'sandbox', 'crafting', 'anime', 'gacha'
    ];
    
    -- Download tiers
    min_downloads TEXT[] := ARRAY['1000', '5000', '10000', '50000', '100000', '500000', '1000000', '5000000'];
    max_downloads TEXT[] := ARRAY['5000', '10000', '50000', '100000', '500000', '1000000', '5000000', '10000000'];
    
    company TEXT;
    category TEXT;
    region TEXT;
    genre TEXT;
    i INTEGER;
    tier_idx INTEGER;
    inserted_count INTEGER := 0;
    batch_count INTEGER := 0;
BEGIN
    -- 1. Company + Category + Region combinations (Google Play)
    RAISE NOTICE 'Inserting company-based Google Play sources...';
    FOREACH company IN ARRAY companies LOOP
        FOREACH category IN ARRAY gp_categories LOOP
            FOREACH region IN ARRAY regions LOOP
                tier_idx := (random() * 7)::INTEGER;
                
                INSERT INTO sa_sources (
                    id, source_type, source_name, search_query, category, 
                    region, min_downloads, max_downloads, status, scrape_interval_hours
                ) VALUES (
                    gen_random_uuid(),
                    'google_play',
                    company || ' - ' || category || ' - ' || upper(region),
                    lower(company) || ' games ' || lower(replace(category, '_', ' ')),
                    category,
                    region,
                    min_downloads[tier_idx + 1],
                    max_downloads[tier_idx + 1],
                    'active',
                    24 + (random() * 48)::INTEGER
                )
                ON CONFLICT DO NOTHING;
                
                inserted_count := inserted_count + 1;
                IF inserted_count >= 4000 THEN
                    EXIT;
                END IF;
            END LOOP;
            IF inserted_count >= 4000 THEN EXIT; END IF;
        END LOOP;
        IF inserted_count >= 4000 THEN EXIT; END IF;
    END LOOP;
    RAISE NOTICE 'Google Play company sources: %', inserted_count;
    
    -- 2. Company + Category + Region combinations (iTunes)
    batch_count := inserted_count;
    RAISE NOTICE 'Inserting company-based iTunes sources...';
    FOREACH company IN ARRAY companies LOOP
        FOREACH category IN ARRAY ios_categories LOOP
            FOREACH region IN ARRAY regions LOOP
                tier_idx := (random() * 7)::INTEGER;
                
                INSERT INTO sa_sources (
                    id, source_type, source_name, search_query, category, 
                    region, min_downloads, max_downloads, status, scrape_interval_hours
                ) VALUES (
                    gen_random_uuid(),
                    'itunes',
                    company || ' - ' || category || ' - ' || upper(region),
                    lower(company) || ' ' || lower(category),
                    category,
                    region,
                    min_downloads[tier_idx + 1],
                    max_downloads[tier_idx + 1],
                    'active',
                    24 + (random() * 48)::INTEGER
                )
                ON CONFLICT DO NOTHING;
                
                inserted_count := inserted_count + 1;
                IF inserted_count - batch_count >= 3000 THEN
                    EXIT;
                END IF;
            END LOOP;
            IF inserted_count - batch_count >= 3000 THEN EXIT; END IF;
        END LOOP;
        IF inserted_count - batch_count >= 3000 THEN EXIT; END IF;
    END LOOP;
    RAISE NOTICE 'iTunes company sources: %', inserted_count - batch_count;
    
    -- 3. Genre-based sources
    batch_count := inserted_count;
    RAISE NOTICE 'Inserting genre-based sources...';
    FOREACH genre IN ARRAY genres LOOP
        FOREACH region IN ARRAY regions LOOP
            tier_idx := (random() * 7)::INTEGER;
            
            INSERT INTO sa_sources (
                id, source_type, source_name, search_query, category, 
                region, min_downloads, max_downloads, status, scrape_interval_hours
            ) VALUES (
                gen_random_uuid(),
                'google_play',
                initcap(genre) || ' Games - ' || upper(region),
                genre || ' mobile games',
                'GAME',
                region,
                min_downloads[tier_idx + 1],
                max_downloads[tier_idx + 1],
                'active',
                24 + (random() * 72)::INTEGER
            )
            ON CONFLICT DO NOTHING;
            
            inserted_count := inserted_count + 1;
            IF inserted_count - batch_count >= 1500 THEN
                EXIT;
            END IF;
        END LOOP;
        IF inserted_count - batch_count >= 1500 THEN EXIT; END IF;
    END LOOP;
    RAISE NOTICE 'Genre sources: %', inserted_count - batch_count;
    
    -- 4. Chart-type sources
    batch_count := inserted_count;
    RAISE NOTICE 'Inserting chart-based sources...';
    FOR i IN 1..1500 LOOP
        tier_idx := (random() * 7)::INTEGER;
        
        INSERT INTO sa_sources (
            id, source_type, source_name, search_query, category, 
            region, min_downloads, max_downloads, status, scrape_interval_hours
        ) VALUES (
            gen_random_uuid(),
            CASE WHEN random() > 0.5 THEN 'google_play' ELSE 'itunes' END,
            'Chart #' || i || ' - ' || upper(regions[1 + (random() * (array_length(regions, 1) - 1))::INTEGER]),
            CASE 
                WHEN i % 5 = 0 THEN 'top grossing games'
                WHEN i % 5 = 1 THEN 'top free games'
                WHEN i % 5 = 2 THEN 'trending games'
                WHEN i % 5 = 3 THEN 'new releases games'
                ELSE 'featured games'
            END,
            gp_categories[1 + (random() * (array_length(gp_categories, 1) - 1))::INTEGER],
            regions[1 + (random() * (array_length(regions, 1) - 1))::INTEGER],
            min_downloads[tier_idx + 1],
            max_downloads[tier_idx + 1],
            'active',
            12 + (random() * 24)::INTEGER
        )
        ON CONFLICT DO NOTHING;
        
        inserted_count := inserted_count + 1;
    END LOOP;
    RAISE NOTICE 'Chart sources: %', inserted_count - batch_count;
    
    RAISE NOTICE 'Total new sources inserted: %', inserted_count;
END $$;

-- Final count
SELECT COUNT(*) AS final_count FROM sa_sources;

-- Breakdown by type
SELECT source_type, COUNT(*) as count 
FROM sa_sources 
GROUP BY source_type 
ORDER BY count DESC;
