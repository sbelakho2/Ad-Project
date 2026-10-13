#!/bin/bash
set -eu

# Rival ApexMediation Console - Development Test Script
# This script starts the development server with mock data

echo "🚀 Starting Rival ApexMediation Console..."
echo ""
echo "📋 Configuration:"
echo "  - Mode: Development with Mock API"
echo "  - URL: http://localhost:3000"
echo "  - Login: demo@rival.com / demo"
echo ""
echo "✅ Features enabled:"
echo "  - Dashboard with revenue metrics"
echo "  - Placements management"
echo "  - Adapters catalog"
echo "  - Analytics with charts"
echo "  - Fraud detection monitoring"
echo "  - Payout history"
echo "  - Settings pages"
echo ""
echo "🔧 Mock API provides realistic data for:"
echo "  - Revenue summaries and time-series"
echo "  - Placement statistics"
echo "  - Adapter performance"
echo "  - Fraud alerts and statistics"
echo "  - Payout history and upcoming payouts"
echo ""
echo "Press Ctrl+C to stop the server"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

cd "$(dirname "$0")/console"
npm run dev
