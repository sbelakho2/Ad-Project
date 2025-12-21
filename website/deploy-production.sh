#!/bin/bash
# Website Production Deployment Script
# Prevents build ID mismatch by deploying to the correct directory
#
# Usage: ./deploy-production.sh

set -e

# Configuration
SERVER="root@46.62.252.96"
SSH_KEY="$HOME/.ssh/hetzner-db-mac"
RELEASE_DIR="/opt/adproject/website_release"
LOCAL_DIR="$(cd "$(dirname "$0")" && pwd)"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${YELLOW}======================================${NC}"
echo -e "${YELLOW}  Website Production Deployment${NC}"
echo -e "${YELLOW}======================================${NC}"
echo ""

# Step 1: Verify we're in the website directory
if [[ ! -f "$LOCAL_DIR/package.json" ]] || [[ ! -f "$LOCAL_DIR/next.config.js" ]]; then
    echo -e "${RED}ERROR: Must run from website directory${NC}"
    exit 1
fi

echo -e "${GREEN}✓ Running from website directory${NC}"

# Step 2: Build
echo ""
echo -e "${YELLOW}Step 1: Building production bundle...${NC}"

cd "$LOCAL_DIR"
npm run build

# Verify standalone was created
if [[ ! -d ".next/standalone" ]]; then
    echo -e "${RED}ERROR: Standalone build not created. Check next.config.js has output: 'standalone'${NC}"
    exit 1
fi

BUILD_ID=$(cat .next/BUILD_ID)
echo -e "${GREEN}✓ Build successful: $BUILD_ID${NC}"

# Step 3: Deploy standalone to server
echo ""
echo -e "${YELLOW}Step 2: Deploying to $SERVER:$RELEASE_DIR...${NC}"

# Stop the service first
echo "  Stopping website service..."
ssh -i "$SSH_KEY" "$SERVER" "pm2 stop website || true"

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

# Step 4: Verify build ID matches on server
echo ""
echo -e "${YELLOW}Step 3: Verifying deployment...${NC}"
SERVER_BUILD_ID=$(ssh -i "$SSH_KEY" "$SERVER" "cat $RELEASE_DIR/.next/BUILD_ID")
if [[ "$BUILD_ID" == "$SERVER_BUILD_ID" ]]; then
    echo -e "${GREEN}✓ Build ID matches: $SERVER_BUILD_ID${NC}"
else
    echo -e "${RED}ERROR: Build ID mismatch! Local: $BUILD_ID, Server: $SERVER_BUILD_ID${NC}"
    exit 1
fi

# Step 5: Restart service
echo ""
echo -e "${YELLOW}Step 4: Restarting website service...${NC}"
ssh -i "$SSH_KEY" "$SERVER" "cd $RELEASE_DIR && pm2 delete website 2>/dev/null || true && PORT=3002 NODE_ENV=production pm2 start server.js --name website"
echo -e "${GREEN}✓ Website service restarted${NC}"

# Step 6: Health check
echo ""
echo -e "${YELLOW}Step 5: Running health check...${NC}"
sleep 3
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" https://apexmediation.ee)
if [[ "$HTTP_CODE" == "200" ]]; then
    echo -e "${GREEN}✓ Website is responding (HTTP $HTTP_CODE)${NC}"
else
    echo -e "${RED}WARNING: Website returned HTTP $HTTP_CODE${NC}"
fi

echo ""
echo -e "${GREEN}======================================${NC}"
echo -e "${GREEN}  Deployment Complete!${NC}"
echo -e "${GREEN}======================================${NC}"
echo ""
echo "Website URL: https://apexmediation.ee"
echo "Build ID:    $BUILD_ID"
echo ""
