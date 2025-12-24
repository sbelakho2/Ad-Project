-- Partition Maintenance Function for analytics_impressions
-- Automatically creates partitions for upcoming days

CREATE OR REPLACE FUNCTION create_analytics_impressions_partition(partition_date date)
RETURNS void AS $$
DECLARE
    partition_name text;
    start_date timestamp with time zone;
    end_date timestamp with time zone;
BEGIN
    -- Generate partition name like analytics_impressions_2025_12_24
    partition_name := 'analytics_impressions_' || to_char(partition_date, 'YYYY_MM_DD');
    
    -- Calculate date range
    start_date := partition_date::timestamp with time zone;
    end_date := (partition_date + interval '1 day')::timestamp with time zone;
    
    -- Check if partition already exists
    IF NOT EXISTS (
        SELECT 1 FROM pg_class c
        JOIN pg_namespace n ON n.oid = c.relnamespace
        WHERE c.relname = partition_name AND n.nspname = 'public'
    ) THEN
        -- Create partition
        EXECUTE format(
            'CREATE TABLE %I PARTITION OF analytics_impressions FOR VALUES FROM (%L) TO (%L)',
            partition_name,
            start_date,
            end_date
        );
        
        RAISE NOTICE 'Created partition % for date range % to %', partition_name, start_date, end_date;
    ELSE
        RAISE NOTICE 'Partition % already exists', partition_name;
    END IF;
END;
$$ LANGUAGE plpgsql;

-- Function to ensure partitions exist for the next N days
CREATE OR REPLACE FUNCTION ensure_analytics_impressions_partitions(days_ahead integer DEFAULT 7)
RETURNS void AS $$
DECLARE
    current_date date := CURRENT_DATE;
    i integer;
BEGIN
    FOR i IN 0..days_ahead LOOP
        PERFORM create_analytics_impressions_partition(current_date + i);
    END LOOP;
    
    RAISE NOTICE 'Partition maintenance complete: ensured partitions exist for % days ahead', days_ahead;
END;
$$ LANGUAGE plpgsql;

-- Function to drop old partitions (for data retention)
CREATE OR REPLACE FUNCTION drop_old_analytics_impressions_partitions(days_to_keep integer DEFAULT 30)
RETURNS void AS $$
DECLARE
    partition_record record;
    cutoff_date date;
BEGIN
    cutoff_date := CURRENT_DATE - days_to_keep;
    
    FOR partition_record IN
        SELECT c.relname as partition_name
        FROM pg_class c
        JOIN pg_inherits i ON i.inhrelid = c.oid
        JOIN pg_class p ON p.oid = i.inhparent
        WHERE p.relname = 'analytics_impressions'
        AND c.relname ~ '^analytics_impressions_\d{4}_\d{2}_\d{2}$'
    LOOP
        -- Extract date from partition name (format: analytics_impressions_YYYY_MM_DD)
        DECLARE
            partition_date date;
            date_parts text[];
        BEGIN
            -- Extract date components from partition name
            date_parts := regexp_matches(partition_record.partition_name, '^analytics_impressions_(\d{4})_(\d{2})_(\d{2})$');
            
            IF array_length(date_parts, 1) = 3 THEN
                partition_date := make_date(
                    date_parts[1]::integer,
                    date_parts[2]::integer,
                    date_parts[3]::integer
                );
                
                IF partition_date < cutoff_date THEN
                    EXECUTE format('DROP TABLE IF EXISTS %I', partition_record.partition_name);
                    RAISE NOTICE 'Dropped old partition %', partition_record.partition_name;
                END IF;
            END IF;
        END;
    END LOOP;
    
    RAISE NOTICE 'Partition cleanup complete: dropped partitions older than % days', days_to_keep;
END;
$$ LANGUAGE plpgsql;

-- Create initial partitions (7 days ahead)
SELECT ensure_analytics_impressions_partitions(7);

-- Example usage for maintenance (to be run daily via cron):
-- SELECT ensure_analytics_impressions_partitions(7);  -- Ensure next 7 days exist
-- SELECT drop_old_analytics_impressions_partitions(30);  -- Drop data older than 30 days
