#!/bin/bash
# Console Production Deployment Script
# Prevents build ID mismatch by deploying to the correct directory
#
# Usage: ./deploy-production.sh
#
# CRITICAL: This script ensures NEXT_PUBLIC_* variables are baked into
# the build at compile time (required for client-side code).

set -e

# Configuration
SERVER="root@46.62.252.96"
SSH_KEY="$HOME/.ssh/hetzner-db-mac"
RELEASE_DIR="/opt/adproject/console_release"
LOCAL_DIR="$(cd "$(dirname "$0")" && pwd)"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${YELLOW}======================================${NC}"
echo -e "${YELLOW}  Console Production Deployment${NC}"
echo -e "${YELLOW}======================================${NC}"
echo ""

# Step 1: Verify we're in the console directory
if [[ ! -f "$LOCAL_DIR/package.json" ]] || ! grep -q '"name": "@apex/console"' "$LOCAL_DIR/package.json" 2>/dev/null; then
    # Try alternate check
    if [[ ! -f "$LOCAL_DIR/next.config.js" ]]; then
        echo -e "${RED}ERROR: Must run from console directory${NC}"
        exit 1
    fi
fi

echo -e "${GREEN}✓ Running from console directory${NC}"

# Step 2: Build with production env vars baked in
echo ""
echo -e "${YELLOW}Step 1: Building with production environment variables...${NC}"
echo "  NEXT_PUBLIC_API_URL=https://api.apexmediation.ee/api/v1"

cd "$LOCAL_DIR"

# CRITICAL: NEXT_PUBLIC_* vars must be set at BUILD time
NEXT_PUBLIC_API_URL=https://api.apexmediation.ee/api/v1 \
NEXT_PUBLIC_FRAUD_API_URL=https://api.apexmediation.ee/api/v1 \
NEXT_PUBLIC_ANALYTICS_API_URL=https://api.apexmediation.ee/api/v1 \
npm run build

# Verify standalone was created
if [[ ! -d ".next/standalone" ]]; then
    echo -e "${RED}ERROR: Standalone build not created. Check next.config.js has output: 'standalone'${NC}"
    exit 1
fi

BUILD_ID=$(cat .next/BUILD_ID)
echo -e "${GREEN}✓ Build successful: $BUILD_ID${NC}"

# Step 3: Verify API URL is baked into bundle
echo ""
echo -e "${YELLOW}Step 2: Verifying API URL in bundle...${NC}"
if grep -r "api.apexmediation.ee" .next/static/chunks/ > /dev/null 2>&1; then
    echo -e "${GREEN}✓ API URL correctly baked into JS bundle${NC}"
else
    echo -e "${RED}ERROR: API URL not found in bundle! NEXT_PUBLIC_API_URL not baked in.${NC}"
    exit 1
fi

# Step 4: Deploy standalone to server
echo ""
echo -e "${YELLOW}Step 3: Deploying to $SERVER:$RELEASE_DIR...${NC}"

# Stop the service first
echo "  Stopping console service..."
ssh -i "$SSH_KEY" "$SERVER" "pm2 stop console || true"

# Deploy standalone (server code + node_modules)
echo "  Deploying standalone build..."
rsync -avz --delete -e "ssh -i $SSH_KEY" \
    .next/standalone/ \
    "$SERVER:$RELEASE_DIR/"

# Deploy static files (not included in standalone)
echo "  Deploying static assets..."
rsync -avz -e "ssh -i $SSH_KEY" \
    .next/static/ \
    "$SERVER:$RELEASE_DIR/.next/static/"

# Deploy public folder
echo "  Deploying public assets..."
rsync -avz -e "ssh -i $SSH_KEY" \
    public/ \
    "$SERVER:$RELEASE_DIR/public/"

# Step 5: Ensure .env file exists on server
echo ""
echo -e "${YELLOW}Step 4: Ensuring .env file exists...${NC}"
ssh -i "$SSH_KEY" "$SERVER" "cat > $RELEASE_DIR/.env << 'EOF'
NEXTAUTH_SECRET=apex-mediation-super-secret-jwt-key-2024
NEXTAUTH_URL=https://console.apexmediation.ee
AUTH_TRUST_HOST=true
NEXT_PUBLIC_API_URL=https://api.apexmediation.ee/api/v1
NEXT_PUBLIC_FRAUD_API_URL=https://api.apexmediation.ee/api/v1
NEXT_PUBLIC_ANALYTICS_API_URL=https://api.apexmediation.ee/api/v1
EOF"
echo -e "${GREEN}✓ .env file created${NC}"

# Step 6: Verify build ID matches on server
echo ""
echo -e "${YELLOW}Step 5: Verifying deployment...${NC}"
SERVER_BUILD_ID=$(ssh -i "$SSH_KEY" "$SERVER" "cat $RELEASE_DIR/.next/BUILD_ID")
if [[ "$BUILD_ID" == "$SERVER_BUILD_ID" ]]; then
    echo -e "${GREEN}✓ Build ID matches: $SERVER_BUILD_ID${NC}"
else
    echo -e "${RED}ERROR: Build ID mismatch! Local: $BUILD_ID, Server: $SERVER_BUILD_ID${NC}"
    exit 1
fi

# Step 7: Restart service
echo ""
echo -e "${YELLOW}Step 6: Restarting console service...${NC}"
ssh -i "$SSH_KEY" "$SERVER" "pm2 restart console"
echo -e "${GREEN}✓ Console service restarted${NC}"

# Step 8: Health check
echo ""
echo -e "${YELLOW}Step 7: Running health check...${NC}"
sleep 3
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" https://console.apexmediation.ee/login)
if [[ "$HTTP_CODE" == "200" ]]; then
    echo -e "${GREEN}✓ Console is responding (HTTP $HTTP_CODE)${NC}"
else
    echo -e "${RED}WARNING: Console returned HTTP $HTTP_CODE${NC}"
fi

# Verify manifest is accessible
MANIFEST_CHECK=$(curl -s -o /dev/null -w "%{http_code}" "https://console.apexmediation.ee/_next/static/$BUILD_ID/_buildManifest.js")
if [[ "$MANIFEST_CHECK" == "200" ]]; then
    echo -e "${GREEN}✓ Static assets accessible (build: $BUILD_ID)${NC}"
else
    echo -e "${RED}WARNING: Static assets returned HTTP $MANIFEST_CHECK${NC}"
fi

echo ""
echo -e "${GREEN}======================================${NC}"
echo -e "${GREEN}  Deployment Complete!${NC}"
echo -e "${GREEN}======================================${NC}"
echo ""
echo "Console URL: https://console.apexmediation.ee"
echo "Build ID:    $BUILD_ID"
echo ""
