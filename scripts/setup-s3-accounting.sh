#!/usr/bin/env bash

# S3 Accounting Bucket Setup
# Creates S3 bucket with Object Lock for 7-year document retention (Estonian Accounting Act compliance)

set -euo pipefail
# Dogfooding dry-run guard (adversarial shell coverage executes scripts under DOGFOOD_DRY_RUN=1)
if [ "${DOGFOOD_DRY_RUN:-0}" = "1" ]; then
  echo "[dogfood] dry-run: $(basename "$0") side effects skipped"
  exit 0
fi

DRY_RUN=${DRY_RUN:-0}
YES=${YES:-0}

print_usage() {
  cat <<USAGE
Provision or update the Accounting S3 bucket with Object Lock (COMPLIANCE, 7 years).

Usage:
  $(basename "$0") [--dry-run] [--yes]

Environment:
  S3_ACCOUNTING_BUCKET   Bucket name (default rivalapexmediation-accounting)
  AWS_REGION             Region (default eu-north-1)
  AWS_PROFILE            Optional AWS profile

Flags:
  --dry-run   Print planned AWS calls, do not execute.
  --yes       Do not prompt for irreversible actions.

WARNING: Enabling COMPLIANCE retention is IRREVERSIBLE for the retention window.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run) DRY_RUN=1; shift ;;
    --yes|-y) YES=1; shift ;;
    -h|--help) print_usage; exit 0 ;;
    *) echo "Unknown argument: $1" >&2; print_usage; exit 2 ;;
  esac
done

BUCKET_NAME="${S3_ACCOUNTING_BUCKET:-rivalapexmediation-accounting}"
AWS_REGION="${AWS_REGION:-eu-north-1}"  # Stockholm (closest to Estonia)

echo "=================================================="
echo "Setting up S3 Accounting Bucket"
echo "=================================================="
echo ""
echo "Bucket: $BUCKET_NAME"
echo "Region: $AWS_REGION"
echo "Profile: ${AWS_PROFILE:-default}"
echo ""

run() {
  echo "+ $*"
  if [[ "$DRY_RUN" -eq 1 ]]; then return 0; fi
  "$@"
}

# Check if bucket exists
if run aws s3api head-bucket --bucket "$BUCKET_NAME" 2>/dev/null; then
  echo "⚠️  Bucket already exists: $BUCKET_NAME"
  if [[ "$YES" -ne 1 ]]; then
    read -p "Do you want to update its configuration? (y/n) " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
      exit 0
    fi
  fi
  BUCKET_EXISTS=true
else
  BUCKET_EXISTS=false
fi

if [ "$BUCKET_EXISTS" = false ]; then
  echo "📦 Creating bucket with Object Lock enabled..."
  run aws s3api create-bucket \
    --bucket "$BUCKET_NAME" \
    --region "$AWS_REGION" \
    --create-bucket-configuration LocationConstraint="$AWS_REGION" \
    --object-lock-enabled-for-bucket

  echo "✅ Bucket created"
fi

echo "🔒 Enabling versioning (required for Object Lock)..."
run aws s3api put-bucket-versioning \
  --bucket "$BUCKET_NAME" \
  --versioning-configuration Status=Enabled

echo "🔐 Configuring Object Lock default retention (7 years)..."
CONFIRM_CODE="lock-$BUCKET_NAME"
if [[ "$YES" -ne 1 && "$DRY_RUN" -ne 1 ]]; then
  cat <<WARN
WARNING: You are about to enable S3 Object Lock in COMPLIANCE mode for seven (7) years.
This **cannot** be disabled or shortened by any IAM user once applied. Every object and
version uploaded to $BUCKET_NAME will be undeletable until the retention period expires.
Type the confirmation token below to continue or press Ctrl+C to abort.
  Token: $CONFIRM_CODE
Documentation: docs/Internal/Deployment/ACCOUNTING_IMPLEMENTATION_STATUS.md (Object Lock section)
WARN
  read -p "Enter confirmation token: " CONFIRM
  if [[ "$CONFIRM" != "$CONFIRM_CODE" ]]; then
    echo "Aborted."; exit 0
  fi
fi
run aws s3api put-object-lock-configuration \
  --bucket "$BUCKET_NAME" \
  --object-lock-configuration '{
    "ObjectLockEnabled": "Enabled",
    "Rule": {
      "DefaultRetention": {
        "Mode": "COMPLIANCE",
        "Years": 7
      }
    }
  }'

echo "🔒 Configuring encryption..."
run aws s3api put-bucket-encryption \
  --bucket "$BUCKET_NAME" \
  --server-side-encryption-configuration '{
    "Rules": [{
      "ApplyServerSideEncryptionByDefault": {
        "SSEAlgorithm": "AES256"
      },
      "BucketKeyEnabled": true
    }]
  }'

echo "🚫 Blocking public access..."
run aws s3api put-public-access-block \
  --bucket "$BUCKET_NAME" \
  --public-access-block-configuration \
    BlockPublicAcls=true,\
IgnorePublicAcls=true,\
BlockPublicPolicy=true,\
RestrictPublicBuckets=true

echo "📋 Configuring lifecycle policy..."
cat > /tmp/lifecycle-policy.json <<EOF
{
  "Rules": [
    {
      "Id": "InvoiceRetention",
      "Status": "Enabled",
      "Filter": {
        "Prefix": "invoices/"
      },
      "NoncurrentVersionExpiration": {
        "NoncurrentDays": 90
      }
    },
    {
      "Id": "VATReportRetention",
      "Status": "Enabled",
      "Filter": {
        "Prefix": "vat-reports/"
      },
      "NoncurrentVersionExpiration": {
        "NoncurrentDays": 90
      }
    },
    {
      "Id": "ReceiptRetention",
      "Status": "Enabled",
      "Filter": {
        "Prefix": "receipts/"
      },
      "NoncurrentVersionExpiration": {
        "NoncurrentDays": 90
      }
    }
  ]
}
EOF

run aws s3api put-bucket-lifecycle-configuration \
  --bucket "$BUCKET_NAME" \
  --lifecycle-configuration file:///tmp/lifecycle-policy.json

rm /tmp/lifecycle-policy.json

echo "🏷️  Adding tags..."
run aws s3api put-bucket-tagging \
  --bucket "$BUCKET_NAME" \
  --tagging 'TagSet=[
    {Key=Project,Value=RivalApexMediation},
    {Key=Purpose,Value=Accounting},
    {Key=Compliance,Value=EstonianAccountingAct},
    {Key=Retention,Value=7Years}
  ]'

echo ""
if [[ "$DRY_RUN" -eq 1 ]]; then
  echo "(dry-run) Skipped execution; printed planned steps above."
else
  echo "✅ S3 bucket configured successfully!"
fi
echo ""
echo "Bucket details:"
echo "  Name: $BUCKET_NAME"
echo "  Region: $AWS_REGION"
echo "  Versioning: Enabled"
echo "  Object Lock: COMPLIANCE mode, 7 years"
echo "  Encryption: AES256"
echo "  Public Access: Blocked"
echo ""
echo "Folder structure:"
echo "  invoices/YYYY/INV-XXXXXX.pdf"
echo "  invoices/YYYY/INV-XXXXXX.xml"
echo "  vat-reports/YYYY/QX-VAT-Report.pdf"
echo "  receipts/YYYY/MM/receipt-XXX.pdf"
echo "  annual-reports/YYYY/annual-report.xbrl"
echo ""
echo "⚠️  IMPORTANT: Objects in COMPLIANCE mode cannot be deleted"
echo "    even by root account until retention period expires!"
echo ""
echo "Add to .env:"
echo "  S3_ACCOUNTING_BUCKET=$BUCKET_NAME"
echo "  AWS_REGION=$AWS_REGION"
echo ""
