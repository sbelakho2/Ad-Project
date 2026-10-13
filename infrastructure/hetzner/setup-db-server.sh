#!/bin/bash
# Dogfooding dry-run guard (adversarial shell coverage executes scripts under DOGFOOD_DRY_RUN=1)
if [ "${DOGFOOD_DRY_RUN:-0}" = "1" ]; then
  echo "[dogfood] dry-run: $(basename "$0") side effects skipped"
  exit 0
fi
# =============================================================================
# Hetzner Database Server Setup Script
# =============================================================================
# This script configures the DB server for the Ad-Project
# 
# IMPORTANT: This server must only be accessible from the App Server
# It should NOT be connected to the public internet
#
# Prerequisites:
#   - Fresh Hetzner VPS with Ubuntu 22.04
#   - SSH access configured
#   - Private network configured between App and DB servers
#
# Usage:
#   ssh -i ~/.ssh/hetzner-db-mac -J root@46.62.252.96 root@77.42.44.35
#   ./setup-db-server.sh
# =============================================================================

set -euo pipefail

# Configuration
POSTGRES_VERSION="16"
POSTGRES_PASSWORD="${POSTGRES_PASSWORD:-CHANGE_ME_IMMEDIATELY}"
APP_SERVER_IP="46.62.252.96"
PRIVATE_NETWORK_INTERFACE="ens10"  # Adjust based on Hetzner private network

echo "=============================================="
echo "Hetzner DB Server Setup"
echo "=============================================="

# -----------------------------------------------------------------------------
# Step 1: System Updates
# -----------------------------------------------------------------------------
echo "[1/10] Updating system packages..."
apt-get update && apt-get upgrade -y
apt-get install -y curl wget gnupg2 lsb-release software-properties-common ufw

# -----------------------------------------------------------------------------
# Step 2: Configure Firewall (CRITICAL - Block Public Access)
# -----------------------------------------------------------------------------
echo "[2/10] Configuring firewall for private-only access..."

# Reset firewall to defaults
ufw --force reset

# Default policies - deny all incoming, allow outgoing
ufw default deny incoming
ufw default allow outgoing

# Only allow SSH from app server
ufw allow from ${APP_SERVER_IP} to any port 22 proto tcp comment 'SSH from App Server'

# Only allow PostgreSQL from app server
ufw allow from ${APP_SERVER_IP} to any port 5432 proto tcp comment 'PostgreSQL from App Server'

# Only allow Autopilot API from app server
ufw allow from ${APP_SERVER_IP} to any port 8090 proto tcp comment 'Autopilot API from App Server'

# Enable firewall
ufw --force enable

echo "Firewall configured. Only accepting connections from ${APP_SERVER_IP}"

# -----------------------------------------------------------------------------
# Step 3: Install PostgreSQL 16
# -----------------------------------------------------------------------------
echo "[3/10] Installing PostgreSQL ${POSTGRES_VERSION}..."

# Add PostgreSQL APT repository
curl -fsSL https://www.postgresql.org/media/keys/ACCC4CF8.asc | gpg --dearmor -o /usr/share/keyrings/postgresql-keyring.gpg
echo "deb [signed-by=/usr/share/keyrings/postgresql-keyring.gpg] http://apt.postgresql.org/pub/repos/apt $(lsb_release -cs)-pgdg main" > /etc/apt/sources.list.d/pgdg.list

apt-get update
apt-get install -y postgresql-${POSTGRES_VERSION} postgresql-contrib-${POSTGRES_VERSION}

# Enable pg_stat_statements extension
echo "shared_preload_libraries = 'pg_stat_statements'" >> /etc/postgresql/${POSTGRES_VERSION}/main/conf.d/extensions.conf

# -----------------------------------------------------------------------------
# Step 4: Configure PostgreSQL
# -----------------------------------------------------------------------------
echo "[4/10] Configuring PostgreSQL..."

# Backup original configs
cp /etc/postgresql/${POSTGRES_VERSION}/main/postgresql.conf /etc/postgresql/${POSTGRES_VERSION}/main/postgresql.conf.bak
cp /etc/postgresql/${POSTGRES_VERSION}/main/pg_hba.conf /etc/postgresql/${POSTGRES_VERSION}/main/pg_hba.conf.bak

# Create optimized postgresql.conf
cat > /etc/postgresql/${POSTGRES_VERSION}/main/conf.d/tuning.conf << 'EOF'
# =============================================================================
# PostgreSQL 16 Tuning for Ad-Project
# =============================================================================

# Connection Settings
listen_addresses = '*'
max_connections = 200
superuser_reserved_connections = 3

# Memory Settings (adjust based on server RAM)
shared_buffers = 4GB
effective_cache_size = 12GB
work_mem = 64MB
maintenance_work_mem = 512MB
wal_buffers = 64MB

# Write-Ahead Log (WAL)
wal_level = replica
max_wal_size = 4GB
min_wal_size = 1GB
checkpoint_completion_target = 0.9

# Query Planner
random_page_cost = 1.1
effective_io_concurrency = 200
default_statistics_target = 100

# Parallelism
max_worker_processes = 8
max_parallel_workers_per_gather = 4
max_parallel_workers = 8
max_parallel_maintenance_workers = 4

# Logging
log_min_duration_statement = 1000
log_checkpoints = on
log_connections = on
log_disconnections = on
log_lock_waits = on
log_temp_files = 0
log_autovacuum_min_duration = 0

# Autovacuum
autovacuum = on
autovacuum_max_workers = 4
autovacuum_naptime = 30s
autovacuum_vacuum_threshold = 50
autovacuum_analyze_threshold = 50
autovacuum_vacuum_scale_factor = 0.05
autovacuum_analyze_scale_factor = 0.05

# Statement Timeout (prevent runaway queries)
statement_timeout = 300000

# pg_stat_statements
pg_stat_statements.max = 10000
pg_stat_statements.track = all
EOF

# Configure pg_hba.conf for app server access only
cat > /etc/postgresql/${POSTGRES_VERSION}/main/pg_hba.conf << EOF
# PostgreSQL Client Authentication Configuration File
# TYPE  DATABASE        USER            ADDRESS                 METHOD

# Local connections
local   all             postgres                                peer
local   all             all                                     peer

# IPv4 local connections
host    all             all             127.0.0.1/32            scram-sha-256

# App server connection (ONLY ALLOW THIS IP)
host    all             all             ${APP_SERVER_IP}/32     scram-sha-256

# Reject all other connections
host    all             all             0.0.0.0/0               reject
EOF

# -----------------------------------------------------------------------------
# Step 5: Create Database and Roles
# -----------------------------------------------------------------------------
echo "[5/10] Creating database and roles..."

# Restart PostgreSQL to apply config changes
systemctl restart postgresql

# Set postgres user password and create database
sudo -u postgres psql << EOF
-- Set postgres password
ALTER USER postgres PASSWORD '${POSTGRES_PASSWORD}';

-- Create main database
CREATE DATABASE adproject;

-- Create application role
CREATE ROLE app_service WITH LOGIN PASSWORD '${POSTGRES_PASSWORD}' 
  CONNECTION LIMIT 100
  CREATEDB;

-- Create read-only role for reporting
CREATE ROLE readonly WITH LOGIN PASSWORD '${POSTGRES_PASSWORD}_readonly';

-- Grant connect to database
GRANT CONNECT ON DATABASE adproject TO app_service;
GRANT CONNECT ON DATABASE adproject TO readonly;

-- Connect to adproject database
\c adproject

-- Enable required extensions
CREATE EXTENSION IF NOT EXISTS pg_stat_statements;
CREATE EXTENSION IF NOT EXISTS pgcrypto;
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";
EOF

echo "Database and roles created."

# -----------------------------------------------------------------------------
# Step 6: Install Node.js for Autopilot
# -----------------------------------------------------------------------------
echo "[6/10] Installing Node.js for Autopilot service..."

curl -fsSL https://deb.nodesource.com/setup_20.x | bash -
apt-get install -y nodejs

# Verify installation
node --version
npm --version

# -----------------------------------------------------------------------------
# Step 7: Create Autopilot Service Directory
# -----------------------------------------------------------------------------
echo "[7/10] Setting up Autopilot service..."

# Create directory for autopilot
mkdir -p /opt/autopilot
mkdir -p /var/log/autopilot

# Create systemd service file
cat > /etc/systemd/system/autopilot.service << 'EOF'
[Unit]
Description=Ad-Project Database Autopilot Service
After=network.target postgresql.service
Requires=postgresql.service

[Service]
Type=simple
User=root
WorkingDirectory=/opt/autopilot
ExecStart=/usr/bin/node /opt/autopilot/dist/index.js
Restart=always
RestartSec=10
StandardOutput=append:/var/log/autopilot/stdout.log
StandardError=append:/var/log/autopilot/stderr.log

# Environment
Environment=NODE_ENV=production
EnvironmentFile=/opt/autopilot/.env

[Install]
WantedBy=multi-user.target
EOF

# Create placeholder .env file
cat > /opt/autopilot/.env << EOF
# Autopilot Service Configuration
DATABASE_URL=postgresql://postgres:${POSTGRES_PASSWORD}@localhost:5432/adproject
DATABASE_POOL_SIZE=5
API_HOST=0.0.0.0
API_PORT=8090
ALLOWED_IPS=${APP_SERVER_IP}
HEALTH_CHECK_INTERVAL_SECONDS=60
PARTITION_CHECK_INTERVAL_HOURS=6
RETENTION_CLEANUP_HOUR=3
AGGREGATION_RUN_MINUTES=5
LOG_LEVEL=info
LOG_FORMAT=json
EOF

chmod 600 /opt/autopilot/.env

echo "Autopilot service configured. Deploy code to /opt/autopilot/"

# -----------------------------------------------------------------------------
# Step 8: Setup Log Rotation
# -----------------------------------------------------------------------------
echo "[8/10] Configuring log rotation..."

cat > /etc/logrotate.d/autopilot << 'EOF'
/var/log/autopilot/*.log {
    daily
    missingok
    rotate 14
    compress
    delaycompress
    notifempty
    create 0640 root root
    sharedscripts
    postrotate
        systemctl reload autopilot > /dev/null 2>&1 || true
    endscript
}
EOF

cat > /etc/logrotate.d/postgresql << 'EOF'
/var/log/postgresql/*.log {
    daily
    missingok
    rotate 14
    compress
    delaycompress
    notifempress
    create 0640 postgres postgres
    sharedscripts
}
EOF

# -----------------------------------------------------------------------------
# Step 9: Security Hardening
# -----------------------------------------------------------------------------
echo "[9/10] Applying security hardening..."

# Disable root login with password (only SSH key)
sed -i 's/^PermitRootLogin.*/PermitRootLogin prohibit-password/' /etc/ssh/sshd_config
sed -i 's/^#PasswordAuthentication.*/PasswordAuthentication no/' /etc/ssh/sshd_config
systemctl restart sshd

# Set secure permissions on PostgreSQL directory
chmod 700 /var/lib/postgresql/${POSTGRES_VERSION}/main

# Configure sysctl for better performance
cat >> /etc/sysctl.conf << 'EOF'

# PostgreSQL optimization
vm.overcommit_memory = 2
vm.overcommit_ratio = 80
vm.swappiness = 10
net.core.somaxconn = 1024
net.ipv4.tcp_max_syn_backlog = 2048
EOF

sysctl -p

# -----------------------------------------------------------------------------
# Step 10: Verify Setup
# -----------------------------------------------------------------------------
echo "[10/10] Verifying setup..."

# Check PostgreSQL is running
systemctl status postgresql --no-pager

# Check firewall rules
ufw status verbose

# Check PostgreSQL is listening only on expected interfaces
ss -tlnp | grep postgres

# Test local connection
sudo -u postgres psql -c "SELECT version();"
sudo -u postgres psql -d adproject -c "SELECT 1 as test;"

echo ""
echo "=============================================="
echo "DB Server Setup Complete!"
echo "=============================================="
echo ""
echo "IMPORTANT NEXT STEPS:"
echo "1. Run database migrations: /opt/autopilot/migrations/034_hetzner_schema_reorganization.sql"
echo "2. Deploy Autopilot service to /opt/autopilot/"
echo "3. Start Autopilot: systemctl start autopilot"
echo "4. Enable Autopilot on boot: systemctl enable autopilot"
echo ""
echo "SECURITY REMINDER:"
echo "- This server only accepts connections from ${APP_SERVER_IP}"
echo "- Change the default passwords in /opt/autopilot/.env"
echo "- PostgreSQL password: ${POSTGRES_PASSWORD}"
echo ""
echo "Connection string for App Server:"
echo "postgresql://app_service:${POSTGRES_PASSWORD}@46.62.252.96:5432/adproject"
echo ""
