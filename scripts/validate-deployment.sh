#!/usr/bin/env bash
# Pre-Deployment Validation Script
# Verifies all requirements from PRE_DEPLOYMENT_CHECKLIST.md are met
# Run this script before deploying to production

# Note: Don't exit on errors - we want to show all failures by default
# Allow opt-in strict mode via STRICT=1
if [[ "${STRICT:-0}" -eq 1 ]]; then
  set -euo pipefail
# Dogfooding dry-run guard (adversarial shell coverage executes scripts under DOGFOOD_DRY_RUN=1)
if [ "${DOGFOOD_DRY_RUN:-0}" = "1" ]; then
  echo "[dogfood] dry-run: $(basename "$0") side effects skipped"
  exit 0
fi
else
  set +e
fi

EXPECTED_MIGRATIONS_DEFAULT=1
EXPECTED_MIGRATIONS_MIN="${EXPECTED_MIGRATIONS_MIN:-$EXPECTED_MIGRATIONS_DEFAULT}"

usage() {
    cat <<USAGE
Pre-deployment validator.

Usage:
    $(basename "$0") [--expected-migrations-min N]

Env overrides:
    STRICT=1                     Exit on first error (default: show all)
    EXPECTED_MIGRATIONS_MIN=N    Same as --expected-migrations-min
USAGE
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --expected-migrations-min)
            EXPECTED_MIGRATIONS_MIN="$2"
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "Unknown argument: $1" >&2
            usage
            exit 2
            ;;
    esac
done

if ! [[ "$EXPECTED_MIGRATIONS_MIN" =~ ^[0-9]+$ ]]; then
    echo "Expected migrations minimum must be an integer (got '$EXPECTED_MIGRATIONS_MIN')." >&2
    exit 2
fi

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Counters
PASSED=0
FAILED=0
WARNINGS=0

# Functions
print_header() {
    echo -e "\n${BLUE}========================================${NC}"
    echo -e "${BLUE}$1${NC}"
    echo -e "${BLUE}========================================${NC}\n"
}

check_pass() {
    echo -e "${GREEN}✅ PASS:${NC} $1"
    ((PASSED++))
}

check_fail() {
    echo -e "${RED}❌ FAIL:${NC} $1"
    ((FAILED++))
}

check_warn() {
    echo -e "${YELLOW}⚠️  WARN:${NC} $1"
    ((WARNINGS++))
}

check_info() {
    echo -e "${BLUE}ℹ️  INFO:${NC} $1"
}

# Start validation
echo -e "${BLUE}"
cat << "EOF"
╔═══════════════════════════════════════════════════════╗
║   ApexMediation Pre-Deployment Validation v1.0        ║
║   Checking production readiness...                    ║
╚═══════════════════════════════════════════════════════╝
EOF
echo -e "${NC}"

# ============================================
# Section 1: Database Validation
# ============================================
print_header "1. DATABASE VALIDATION"

# Check if database is accessible
check_info "Checking database connection..."
if [ -z "$DATABASE_URL" ]; then
    check_warn "DATABASE_URL not set in environment"
else
    check_pass "DATABASE_URL is configured"
fi

# Check migrations (parameterized)
check_info "Verifying database migrations..."
if [ -d "backend/migrations" ]; then
    MIGRATION_COUNT=$(find backend/migrations -name "*.sql" | wc -l | tr -d ' ')
    if [ "$MIGRATION_COUNT" -ge "$EXPECTED_MIGRATIONS_MIN" ]; then
        check_pass "Found $MIGRATION_COUNT migration(s) (>= expected minimum $EXPECTED_MIGRATIONS_MIN)"
    else
        check_fail "Found $MIGRATION_COUNT migration(s), below expected minimum $EXPECTED_MIGRATIONS_MIN"
    fi
else
    check_fail "backend/migrations/ directory not found"
fi

# ============================================
# Section 2: Build Validation
# ============================================
print_header "2. BUILD VALIDATION"

# Check if node_modules exists
if [ -d "backend/node_modules" ]; then
    check_pass "Backend dependencies installed"
else
    check_fail "Backend node_modules not found. Run: cd backend && npm install"
fi

if [ -d "console/node_modules" ]; then
    check_pass "Console dependencies installed"
else
    check_fail "Console node_modules not found. Run: cd console && npm install"
fi

# Check TypeScript compilation
check_info "Checking TypeScript compilation..."
if cd backend && npm run build > /dev/null 2>&1; then
    check_pass "Backend builds successfully"
    cd ..
else
    check_fail "Backend build failed"
    cd ..
fi

# ============================================
# Section 3: AI Cost Controls
# ============================================
print_header "3. AI COST CONTROL VALIDATION"

# Check .env file
if [ -f "backend/.env" ]; then
    check_pass "backend/.env file exists"
    
    # Check OpenAI API key
    if grep -q "OPENAI_API_KEY=" backend/.env; then
        if grep "OPENAI_API_KEY=sk-" backend/.env > /dev/null 2>&1; then
            check_pass "OpenAI API key configured"
        else
            check_fail "OpenAI API key not properly set in backend/.env"
        fi
    else
        check_fail "OPENAI_API_KEY not found in backend/.env"
    fi
    
    # Check feature flags (all should be false initially)
    FLAGS=("ENABLE_AI_AUTOMATION" "ENABLE_SALES_AI_OPTIMIZATION" "ENABLE_GROWTH_AI_ANALYTICS" "ENABLE_SELF_EVOLVING_AI")
    for FLAG in "${FLAGS[@]}"; do
        if grep "$FLAG=false" backend/.env > /dev/null 2>&1; then
            check_pass "$FLAG is disabled (correct for initial deployment)"
        elif grep "$FLAG=true" backend/.env > /dev/null 2>&1; then
            check_warn "$FLAG is enabled - ensure this is intentional for staged rollout"
        else
            check_fail "$FLAG not found in backend/.env"
        fi
    done
else
    check_fail "backend/.env file not found"
fi

# Check cost control documentation
if [ -f "infrastructure/runbooks/AI_COST_CONTROLS.md" ]; then
    check_pass "AI cost control runbook exists"
else
    check_fail "AI cost control runbook not found"
fi

# Check Terraform module
if [ -f "infrastructure/terraform/modules/ai-cost-controls/main.tf" ]; then
    check_pass "Terraform AI cost control module exists"
else
    check_fail "Terraform module not found"
fi

# Check monitoring alerts
if [ -f "monitoring/alerts.yml" ]; then
    if grep -q "ai_cost_controls" monitoring/alerts.yml; then
        check_pass "AI cost control alerts configured in monitoring/alerts.yml"
    else
        check_fail "AI cost control alerts not found in monitoring/alerts.yml"
    fi
else
    check_fail "monitoring/alerts.yml not found"
fi

# ============================================
# Section 4: Documentation Validation
# ============================================
print_header "4. DOCUMENTATION VALIDATION"

REQUIRED_DOCS=(
    "PRE_DEPLOYMENT_CHECKLIST.md"
    "DEPLOYMENT_READINESS_SUMMARY.md"
    "docs/production-deployment.md"
    "infrastructure/runbooks/AI_COST_CONTROLS.md"
    "infrastructure/terraform/modules/ai-cost-controls/README.md"
)

for DOC in "${REQUIRED_DOCS[@]}"; do
    if [ -f "$DOC" ]; then
        check_pass "Documentation exists: $DOC"
    else
        check_fail "Missing documentation: $DOC"
    fi
done

# ============================================
# Section 5: Service Validation
# ============================================
print_header "5. SERVICE VALIDATION"

# Check if critical services exist
SERVICES=(
    "backend/services/sales/InfluenceBasedSalesService.ts"
    "backend/services/automation/AutomatedGrowthEngine.ts"
    "backend/services/automation/SelfEvolvingSystemService.ts"
)

for SERVICE in "${SERVICES[@]}"; do
    if [ -f "$SERVICE" ]; then
        check_pass "Service exists: $(basename $SERVICE)"
        
        # Check for feature flag implementation
        if grep -q "aiEnabled\|enableAIOptimization" "$SERVICE"; then
            check_pass "  └─ Feature flag implemented"
        else
            check_warn "  └─ Feature flag not found (may not be required)"
        fi
    else
        check_fail "Missing service: $SERVICE"
    fi
done

# ============================================
# Section 6: Security Validation
# ============================================
print_header "6. SECURITY VALIDATION"

# Check .env.example exists
if [ -f "backend/.env.example" ]; then
    check_pass ".env.example template exists"
else
    check_warn ".env.example not found (recommended for new deployments)"
fi

# Check .gitignore excludes secrets
if [ -f ".gitignore" ]; then
    if grep -q ".env" .gitignore; then
        check_pass ".env files excluded from git"
    else
        check_fail ".env not in .gitignore - SECURITY RISK!"
    fi
else
    check_warn ".gitignore not found"
fi

# ============================================
# Section 7: Infrastructure Validation
# ============================================
print_header "7. INFRASTRUCTURE VALIDATION"

# Check Kubernetes manifests
if [ -d "infrastructure/k8s" ]; then
    check_pass "Kubernetes manifests directory exists"
    
    K8S_FILE_COUNT=$(find infrastructure/k8s -name "*.yaml" -o -name "*.yml" | wc -l)
    check_info "Found $K8S_FILE_COUNT Kubernetes manifest files"
else
    check_warn "infrastructure/k8s/ not found (may not be using Kubernetes)"
fi

# Check monitoring setup
if [ -d "infrastructure/monitoring" ]; then
    check_pass "Monitoring configuration directory exists"
else
    check_warn "infrastructure/monitoring/ not found"
fi

# ============================================
# Section 8: Final Summary
# ============================================
print_header "VALIDATION SUMMARY"

echo -e "Results:"
echo -e "  ${GREEN}✅ Passed:${NC}   $PASSED"
echo -e "  ${RED}❌ Failed:${NC}   $FAILED"
echo -e "  ${YELLOW}⚠️  Warnings:${NC} $WARNINGS"
echo ""

if [ $FAILED -eq 0 ]; then
    echo -e "${GREEN}╔═══════════════════════════════════════════════════════╗${NC}"
    echo -e "${GREEN}║   ✅ VALIDATION PASSED - READY FOR DEPLOYMENT        ║${NC}"
    echo -e "${GREEN}╚═══════════════════════════════════════════════════════╝${NC}"
    echo ""
    echo -e "Next steps:"
    echo -e "  1. Review ${BLUE}PRE_DEPLOYMENT_CHECKLIST.md${NC}"
    echo -e "  2. Follow ${BLUE}docs/production-deployment.md${NC}"
    echo -e "  3. Apply Terraform: ${BLUE}terraform apply${NC}"
    echo -e "  4. Deploy to Kubernetes: ${BLUE}kubectl apply -f infrastructure/k8s/${NC}"
    echo ""
    exit 0
else
    echo -e "${RED}╔═══════════════════════════════════════════════════════╗${NC}"
    echo -e "${RED}║   ❌ VALIDATION FAILED - FIX ERRORS BEFORE DEPLOY    ║${NC}"
    echo -e "${RED}╚═══════════════════════════════════════════════════════╝${NC}"
    echo ""
    echo -e "Fix the following issues:"
    echo -e "  • Review failed checks above"
    echo -e "  • Ensure all dependencies installed"
    echo -e "  • Verify environment variables configured"
    echo -e "  • Complete all items in ${BLUE}PRE_DEPLOYMENT_CHECKLIST.md${NC}"
    echo ""
    exit 1
fi
