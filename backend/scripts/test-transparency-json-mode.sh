#!/usr/bin/env bash
set -eu
# Dogfooding dry-run guard (adversarial shell coverage executes scripts under DOGFOOD_DRY_RUN=1)
if [ "${DOGFOOD_DRY_RUN:-0}" = "1" ]; then
  echo "[dogfood] dry-run: $(basename "$0") side effects skipped"
  exit 0
fi
#
# Test transparency-verify.ts --json mode with various scenarios
# This script validates structured output format and exit codes
#

set -e

echo "=== Transparency CLI --json Mode Test Suite ==="
echo ""

# Test 1: Usage error (missing --auction)
echo "Test 1: Usage error scenario..."
cd /home/aaron/IdeaProjects/Ad-Project/backend
if npm run verify:transparency -- --publisher test-pub --json 2>&1 | grep -q "Missing --auction"; then
  echo "✅ Usage error message displayed correctly"
else
  echo "❌ Usage error message not displayed"
  exit 1
fi

# Test 2: Exit code validation (npm wraps exit codes, so just check non-zero)
echo ""
echo "Test 2: Exit code for usage error..."
if npm run verify:transparency -- --publisher test-pub --json 2>&1 > /dev/null; then
  echo "❌ Expected non-zero exit code for usage error"
  exit 1
else
  echo "✅ Non-zero exit code returned (npm wraps original exit code 2)"
fi

echo ""
echo "=== Manual Testing Required ==="
echo ""
echo "To test full JSON mode output with live API:"
echo ""
echo "1. Start backing services:"
echo "   sudo docker compose up -d postgres redis"
echo ""
echo "2. Run backend with transparency enabled:"
echo "   cd /home/aaron/IdeaProjects/Ad-Project/backend"
echo "   TRANSPARENCY_ENABLED=1 TRANSPARENCY_API_ENABLED=true npm run dev"
echo ""
echo "3. Trigger auction and get auction_id from logs"
echo ""
echo "4. Test JSON mode with valid auction:"
echo "   npm run verify:transparency -- \\"
echo "     --auction <uuid> \\"
echo "     --publisher <uuid> \\"
echo "     --json"
echo ""
echo "Expected JSON structure:"
echo "{"
echo "  \"status\": \"PASS\","
echo "  \"auction_id\": \"...\","
echo "  \"publisher_id\": \"...\","
echo "  \"timestamp\": \"2025-11-10T12:34:56Z\","
echo "  \"key_id\": \"key-2025-11-10-v2\","
echo "  \"algo\": \"Ed25519\","
echo "  \"canonical\": \"{...}\","
echo "  \"canonical_length\": 1234,"
echo "  \"sample_bps\": 250,"
echo "  \"server_verification\": {"
echo "    \"status\": \"pass\","
echo "    \"reason\": null"
echo "  },"
echo "  \"local_verification\": {"
echo "    \"status\": \"pass\","
echo "    \"error\": null"
echo "  }"
echo "}"
echo ""
echo "=== Automated Tests Complete ==="
