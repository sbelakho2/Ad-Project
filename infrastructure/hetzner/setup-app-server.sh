#!/bin/bash
# =============================================================================
# Hetzner App Server Setup Script
# =============================================================================
# This script configures the App Server for the Ad-Project
# 
# This server hosts:
#   - Backend API (Node.js/Express)
#   - Console (Next.js)
#   - Redis (cache and queues)
#   - Nginx (reverse proxy)
#
# Prerequisites:
#   - Fresh Hetzner VPS with Ubuntu 22.04
#   - SSH access configured
#   - Private network configured between App and DB servers
#   - Domain name pointing to this server (for SSL)
#
# Usage:
#   ssh -i ~/.ssh/hetzner-db-mac root@46.62.252.96
#   ./setup-app-server.sh
# =============================================================================

set -euo pipefail

# Configuration
DB_SERVER_IP="77.42.44.35"
DB_PASSWORD="${DB_PASSWORD:-CHANGE_ME_IMMEDIATELY}"
REDIS_PASSWORD="${REDIS_PASSWORD:-CHANGE_ME_REDIS}"
DOMAIN="${DOMAIN:-api.yourapp.com}"
CONSOLE_DOMAIN="${CONSOLE_DOMAIN:-console.yourapp.com}"

echo "=============================================="
echo "Hetzner App Server Setup"
echo "=============================================="

# -----------------------------------------------------------------------------
# Step 1: System Updates
# -----------------------------------------------------------------------------
echo "[1/12] Updating system packages..."
apt-get update && apt-get upgrade -y
apt-get install -y curl wget gnupg2 lsb-release software-properties-common ufw git

# -----------------------------------------------------------------------------
# Step 2: Configure Firewall
# -----------------------------------------------------------------------------
echo "[2/12] Configuring firewall..."

ufw --force reset
ufw default deny incoming
ufw default allow outgoing

# SSH
ufw allow 22/tcp comment 'SSH'

# HTTP/HTTPS
ufw allow 80/tcp comment 'HTTP'
ufw allow 443/tcp comment 'HTTPS'

# Backend API (internal - via reverse proxy)
# ufw allow 8080/tcp comment 'Backend API'

# Console (internal - via Nginx)
# ufw allow 3000/tcp comment 'Console'

ufw --force enable

echo "Firewall configured."

# -----------------------------------------------------------------------------
# Step 3: Install Docker
# -----------------------------------------------------------------------------
echo "[3/12] Installing Docker..."

# Remove old versions
apt-get remove -y docker docker-engine docker.io containerd runc 2>/dev/null || true

# Install Docker
curl -fsSL https://get.docker.com -o get-docker.sh
sh get-docker.sh
rm get-docker.sh

# Install Docker Compose
curl -L "https://github.com/docker/compose/releases/download/v2.23.0/docker-compose-$(uname -s)-$(uname -m)" -o /usr/local/bin/docker-compose
chmod +x /usr/local/bin/docker-compose

# Verify installation
docker --version
docker-compose --version

# -----------------------------------------------------------------------------
# Step 4: Install Node.js
# -----------------------------------------------------------------------------
echo "[4/12] Installing Node.js..."

curl -fsSL https://deb.nodesource.com/setup_20.x | bash -
apt-get install -y nodejs

node --version
npm --version

# Install pnpm
npm install -g pnpm

# -----------------------------------------------------------------------------
# Step 5: Install Redis
# -----------------------------------------------------------------------------
echo "[5/12] Installing Redis..."

apt-get install -y redis-server

# Configure Redis
cat > /etc/redis/redis.conf << EOF
# Redis Configuration for Ad-Project

bind 127.0.0.1
port 6379
daemonize yes
supervised systemd
pidfile /run/redis/redis-server.pid
loglevel notice
logfile /var/log/redis/redis-server.log

# Security
requirepass ${REDIS_PASSWORD}

# Memory
maxmemory 2gb
maxmemory-policy allkeys-lru

# Persistence
save 900 1
save 300 10
save 60 10000
stop-writes-on-bgsave-error yes
rdbcompression yes
dbfilename dump.rdb
dir /var/lib/redis

# AOF
appendonly yes
appendfilename "appendonly.aof"
appendfsync everysec
no-appendfsync-on-rewrite no
auto-aof-rewrite-percentage 100
auto-aof-rewrite-min-size 64mb
EOF

systemctl restart redis-server
systemctl enable redis-server

# Verify Redis
redis-cli -a "${REDIS_PASSWORD}" ping

# -----------------------------------------------------------------------------
# Step 6: Install Nginx (legacy; production uses Caddy)
# -----------------------------------------------------------------------------
echo "[6/12] Installing Nginx..."

apt-get install -y nginx

# Remove default site
rm -f /etc/nginx/sites-enabled/default

# Create main config
cat > /etc/nginx/nginx.conf << 'EOF'
user www-data;
worker_processes auto;
pid /run/nginx.pid;
include /etc/nginx/modules-enabled/*.conf;

events {
    worker_connections 2048;
    multi_accept on;
    use epoll;
}

http {
    # Basic Settings
    sendfile on;
    tcp_nopush on;
    tcp_nodelay on;
    keepalive_timeout 65;
    types_hash_max_size 2048;
    server_tokens off;

    include /etc/nginx/mime.types;
    default_type application/octet-stream;

    # Logging
    access_log /var/log/nginx/access.log;
    error_log /var/log/nginx/error.log;

    # Gzip
    gzip on;
    gzip_vary on;
    gzip_proxied any;
    gzip_comp_level 6;
    gzip_types text/plain text/css text/xml application/json application/javascript application/xml application/rss+xml;

    # Rate Limiting
    limit_req_zone $binary_remote_addr zone=api:10m rate=100r/s;
    limit_conn_zone $binary_remote_addr zone=conn:10m;

    # Upstream servers
    upstream backend {
        server 127.0.0.1:8080;
        keepalive 32;
    }

    upstream console {
        server 127.0.0.1:3000;
        keepalive 32;
    }

    # Include site configs
    include /etc/nginx/sites-enabled/*;
}
EOF

# Create backend API config
cat > /etc/nginx/sites-available/backend << EOF
server {
    listen 80;
    server_name ${DOMAIN};

    location / {
        limit_req zone=api burst=50 nodelay;
        limit_conn conn 50;

        proxy_pass http://backend;
        proxy_http_version 1.1;
        proxy_set_header Upgrade \$http_upgrade;
        proxy_set_header Connection 'upgrade';
        proxy_set_header Host \$host;
        proxy_set_header X-Real-IP \$remote_addr;
        proxy_set_header X-Forwarded-For \$proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto \$scheme;
        proxy_cache_bypass \$http_upgrade;
        proxy_read_timeout 300s;
        proxy_connect_timeout 75s;
    }

    # SDK endpoints - higher rate limits
    location /sdk/ {
        limit_req zone=api burst=200 nodelay;
        
        proxy_pass http://backend;
        proxy_http_version 1.1;
        proxy_set_header Host \$host;
        proxy_set_header X-Real-IP \$remote_addr;
        proxy_set_header X-Forwarded-For \$proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto \$scheme;
    }

    # Health check endpoint
    location /health {
        proxy_pass http://backend;
        access_log off;
    }
}
EOF

# Create console config
cat > /etc/nginx/sites-available/console << EOF
server {
    listen 80;
    server_name ${CONSOLE_DOMAIN};

    location / {
        proxy_pass http://console;
        proxy_http_version 1.1;
        proxy_set_header Upgrade \$http_upgrade;
        proxy_set_header Connection 'upgrade';
        proxy_set_header Host \$host;
        proxy_set_header X-Real-IP \$remote_addr;
        proxy_set_header X-Forwarded-For \$proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto \$scheme;
        proxy_cache_bypass \$http_upgrade;
    }
}
EOF

# Enable sites
ln -sf /etc/nginx/sites-available/backend /etc/nginx/sites-enabled/
ln -sf /etc/nginx/sites-available/console /etc/nginx/sites-enabled/

# Test and reload
nginx -t
systemctl restart nginx
systemctl enable nginx

# -----------------------------------------------------------------------------
# Step 7: Setup Application Directory
# -----------------------------------------------------------------------------
echo "[7/12] Setting up application directories..."

mkdir -p /opt/adproject/{backend,console,console-run,logs}
mkdir -p /var/log/adproject

# Create environment files
cat > /opt/adproject/backend/.env << EOF
# Backend Environment Configuration
NODE_ENV=production
PORT=8080

# Database
DATABASE_URL=postgresql://app_service:${DB_PASSWORD}@${DB_SERVER_IP}:5432/adproject
DATABASE_POOL_SIZE=20
DATABASE_STATEMENT_TIMEOUT=30000

# Redis
REDIS_URL=redis://:${REDIS_PASSWORD}@127.0.0.1:6379

# Autopilot API (for Infrastructure Control Panel)
AUTOPILOT_API_URL=http://${DB_SERVER_IP}:8090

# JWT
JWT_SECRET=CHANGE_ME_TO_LONG_RANDOM_STRING

# Logging
LOG_LEVEL=info
EOF

cat > /opt/adproject/console/.env << EOF
# Console Environment Configuration
NODE_ENV=production
PORT=3000

# API URL
NEXT_PUBLIC_API_URL=https://${DOMAIN}

# Autopilot API URL (for Admin Control Panel)
NEXT_PUBLIC_AUTOPILOT_API_URL=http://${DB_SERVER_IP}:8090
EOF

chmod 600 /opt/adproject/backend/.env
chmod 600 /opt/adproject/console/.env

# -----------------------------------------------------------------------------
# Step 8: Create Systemd Services
# -----------------------------------------------------------------------------
echo "[8/12] Creating systemd services..."

# Backend service
cat > /etc/systemd/system/adproject-backend.service << 'EOF'
[Unit]
Description=Ad-Project Backend API
After=network.target redis.service

[Service]
Type=simple
User=root
WorkingDirectory=/opt/adproject/backend
ExecStart=/usr/bin/node /opt/adproject/backend/dist/index.js
Restart=always
RestartSec=10
StandardOutput=append:/var/log/adproject/backend.log
StandardError=append:/var/log/adproject/backend-error.log
EnvironmentFile=/opt/adproject/backend/.env

# Limits
LimitNOFILE=65535

[Install]
WantedBy=multi-user.target
EOF

# Console service
cat > /etc/systemd/system/adproject-console.service << 'EOF'
[Unit]
Description=Ad-Project Console
After=network.target

[Service]
Type=simple
User=root
WorkingDirectory=/opt/adproject/console-run
ExecStart=/usr/bin/node /opt/adproject/console-run/server.js
Restart=always
RestartSec=10
StandardOutput=append:/var/log/adproject/console.log
StandardError=append:/var/log/adproject/console-error.log
EnvironmentFile=/opt/adproject/console/.env

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload

# -----------------------------------------------------------------------------
# Step 9: Setup Log Rotation
# -----------------------------------------------------------------------------
echo "[9/12] Configuring log rotation..."

cat > /etc/logrotate.d/adproject << 'EOF'
/var/log/adproject/*.log {
    daily
    missingok
    rotate 14
    compress
    delaycompress
    notifempty
    create 0640 root root
    sharedscripts
}
EOF

# -----------------------------------------------------------------------------
# Step 10: Install Certbot for SSL
# -----------------------------------------------------------------------------
echo "[10/12] Installing Certbot for SSL..."

apt-get install -y certbot python3-certbot-nginx

echo "SSL certificates can be obtained with:"
echo "  certbot --nginx -d ${DOMAIN} -d ${CONSOLE_DOMAIN}"

# -----------------------------------------------------------------------------
# Step 11: Security Hardening
# -----------------------------------------------------------------------------
echo "[11/12] Applying security hardening..."

# SSH hardening
sed -i 's/^PermitRootLogin.*/PermitRootLogin prohibit-password/' /etc/ssh/sshd_config
sed -i 's/^#PasswordAuthentication.*/PasswordAuthentication no/' /etc/ssh/sshd_config
systemctl restart sshd

# Kernel tuning
cat >> /etc/sysctl.conf << 'EOF'

# Network optimization
net.core.somaxconn = 65535
net.ipv4.tcp_max_syn_backlog = 65535
net.ipv4.ip_local_port_range = 1024 65535
net.ipv4.tcp_tw_reuse = 1
net.ipv4.tcp_fin_timeout = 15
net.core.netdev_max_backlog = 65535
net.ipv4.tcp_keepalive_time = 300
net.ipv4.tcp_keepalive_probes = 5
net.ipv4.tcp_keepalive_intvl = 15

# File descriptors
fs.file-max = 2097152
EOF

sysctl -p

# Increase file limits
cat >> /etc/security/limits.conf << 'EOF'
* soft nofile 65535
* hard nofile 65535
EOF

# -----------------------------------------------------------------------------
# Step 12: Create Deployment Script
# -----------------------------------------------------------------------------
echo "[12/12] Creating deployment scripts..."

cat > /opt/adproject/deploy.sh << 'EOF'
#!/bin/bash
set -euo pipefail

echo "=== Ad-Project Deployment ==="

cd /opt/adproject

# Pull latest code (assuming git repo)
# git pull origin main

# Backend deployment
echo "Building backend..."
cd /opt/adproject/backend
pnpm install --frozen-lockfile
pnpm build

# Console deployment
echo "Building console..."
cd /opt/adproject/console
pnpm install --frozen-lockfile
pnpm build

# Stage the Next.js standalone output into a runtime folder that matches the Docker layout.
# Important: copy dotfolders (e.g. `.next`) from `.next/standalone`.
rm -rf /opt/adproject/console-run
mkdir -p /opt/adproject/console-run/.next
cp -R .next/standalone/. /opt/adproject/console-run/
rm -rf /opt/adproject/console-run/.next/static
cp -R .next/static /opt/adproject/console-run/.next/static
cp -R public /opt/adproject/console-run/public

# Restart services
echo "Restarting services..."
systemctl restart adproject-backend
systemctl restart adproject-console

echo "=== Deployment Complete ==="
systemctl status adproject-backend --no-pager
systemctl status adproject-console --no-pager
EOF

chmod +x /opt/adproject/deploy.sh

# Create health check script
cat > /opt/adproject/healthcheck.sh << 'EOF'
#!/bin/bash
echo "=== Health Check ==="
echo ""
echo "Services:"
systemctl is-active adproject-backend && echo "✓ Backend: running" || echo "✗ Backend: stopped"
systemctl is-active adproject-console && echo "✓ Console: running" || echo "✗ Console: stopped"
systemctl is-active redis-server && echo "✓ Redis: running" || echo "✗ Redis: stopped"
systemctl is-active nginx && echo "✓ Nginx: running" || echo "✗ Nginx: stopped"
echo ""
echo "DB Connection:"
curl -s http://127.0.0.1:8080/health | jq . 2>/dev/null || echo "Backend health check failed"
echo ""
echo "Autopilot Connection:"
curl -s http://46.62.252.96:8090/status | jq . 2>/dev/null || echo "Autopilot connection failed"
EOF

chmod +x /opt/adproject/healthcheck.sh

# -----------------------------------------------------------------------------
# Final Output
# -----------------------------------------------------------------------------
echo ""
echo "=============================================="
echo "App Server Setup Complete!"
echo "=============================================="
echo ""
echo "SERVICES CONFIGURED:"
echo "  - Nginx (reverse proxy) - ports 80/443"
echo "  - Redis (cache/queues) - port 6379 (localhost only)"
echo "  - Backend API - port 3001 (via Nginx)"
echo "  - Console - port 3000 (via Nginx)"
echo ""
echo "NEXT STEPS:"
echo "1. Deploy code to /opt/adproject/backend and /opt/adproject/console"
echo "2. Run: /opt/adproject/deploy.sh"
echo "3. Enable services: systemctl enable adproject-backend adproject-console"
echo "4. Setup SSL: certbot --nginx -d ${DOMAIN} -d ${CONSOLE_DOMAIN}"
echo ""
echo "ENVIRONMENT FILES:"
echo "  - /opt/adproject/backend/.env"
echo "  - /opt/adproject/console/.env"
echo ""
echo "HEALTH CHECK:"
echo "  /opt/adproject/healthcheck.sh"
echo ""
echo "DB SERVER CONNECTION:"
echo "  ${DB_SERVER_IP}:5432 (PostgreSQL)"
echo "  ${DB_SERVER_IP}:8090 (Autopilot API)"
echo ""
