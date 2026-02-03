#!/usr/bin/env bash
# Send a test email via Resend API
# Usage: ./send-test-email.sh <to_email>
set -euo pipefail

TO_EMAIL="${1:-}"
if [[ -z "$TO_EMAIL" ]]; then
  echo "Usage: $0 <to_email>"
  echo "Example: $0 sadokaaronbelakhoua@gmail.com"
  exit 1
fi

# Load env if available
if [[ -f /opt/adproject/.env ]]; then
  source /opt/adproject/.env 2>/dev/null || true
fi

RESEND_API_KEY="${RESEND_API_KEY:-}"
if [[ -z "$RESEND_API_KEY" ]]; then
  echo "Error: RESEND_API_KEY not set"
  exit 1
fi

FROM_EMAIL="${FROM_EMAIL:-ApexMediation Mail Server <noreply@apexmediation.ee>}"
SUBJECT="Test Email from ApexMediation Mail Server"
TIMESTAMP=$(date -u +"%Y-%m-%d %H:%M:%S UTC")

# HTML email body
HTML_BODY=$(cat <<EOF
<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <style>
        body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; line-height: 1.6; color: #333; }
        .container { max-width: 600px; margin: 0 auto; padding: 20px; }
        .header { background: linear-gradient(135deg, #667eea 0%, #764ba2 100%); color: white; padding: 30px; border-radius: 8px 8px 0 0; }
        .content { background: #f9fafb; padding: 30px; border: 1px solid #e5e7eb; }
        .footer { background: #1f2937; color: #9ca3af; padding: 20px; border-radius: 0 0 8px 8px; font-size: 12px; }
        .success { background: #10b981; color: white; padding: 10px 20px; border-radius: 4px; display: inline-block; }
        table { width: 100%; margin: 20px 0; border-collapse: collapse; }
        td { padding: 8px; }
        .label { background: #e5e7eb; font-weight: bold; }
    </style>
</head>
<body>
    <div class="container">
        <div class="header">
            <h1 style="margin: 0;">🚀 ApexMediation Mail Server</h1>
            <p style="margin: 10px 0 0 0; opacity: 0.9;">Test Email</p>
        </div>
        <div class="content">
            <p class="success">✓ Email Delivery Successful!</p>
            <h2>Hello!</h2>
            <p>This is a test email from the <strong>ApexMediation Mail Server</strong> running on the test server.</p>
            <table>
                <tr><td class="label">Sent at:</td><td>${TIMESTAMP}</td></tr>
                <tr><td class="label">From:</td><td>${FROM_EMAIL}</td></tr>
                <tr><td class="label">To:</td><td>${TO_EMAIL}</td></tr>
                <tr><td class="label">Server:</td><td>Test Server (157.90.126.205)</td></tr>
            </table>
            <p>If you received this email, the mail server is working correctly!</p>
        </div>
        <div class="footer">
            <p>ApexMediation Mail Server • <a href="https://apexmediation.ee" style="color: #60a5fa;">apexmediation.ee</a></p>
        </div>
    </div>
</body>
</html>
EOF
)

TEXT_BODY=$(cat <<EOF
Hello!

This is a test email from the ApexMediation Mail Server.

Sent at: ${TIMESTAMP}
From: ${FROM_EMAIL}
To: ${TO_EMAIL}
Server: Test Server (157.90.126.205)

If you received this email, the mail server is working correctly!

--
ApexMediation Mail Server
https://apexmediation.ee
EOF
)

# Escape JSON
HTML_ESCAPED=$(echo "$HTML_BODY" | python3 -c 'import json,sys; print(json.dumps(sys.stdin.read()))')
TEXT_ESCAPED=$(echo "$TEXT_BODY" | python3 -c 'import json,sys; print(json.dumps(sys.stdin.read()))')

# Create JSON payload
PAYLOAD=$(cat <<EOF
{
  "from": "${FROM_EMAIL}",
  "to": ["${TO_EMAIL}"],
  "subject": "${SUBJECT}",
  "html": ${HTML_ESCAPED},
  "text": ${TEXT_ESCAPED}
}
EOF
)

echo "📧 Sending test email to ${TO_EMAIL}..."
echo ""

RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "https://api.resend.com/emails" \
  -H "Authorization: Bearer ${RESEND_API_KEY}" \
  -H "Content-Type: application/json" \
  -d "${PAYLOAD}")

HTTP_CODE=$(echo "$RESPONSE" | tail -1)
BODY=$(echo "$RESPONSE" | sed '$d')

if [[ "$HTTP_CODE" == "200" ]]; then
  EMAIL_ID=$(echo "$BODY" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("id","unknown"))')
  echo "✅ Email sent successfully!"
  echo "   Email ID: ${EMAIL_ID}"
  echo "   To: ${TO_EMAIL}"
  echo "   Subject: ${SUBJECT}"
else
  echo "❌ Failed to send email (HTTP ${HTTP_CODE})"
  echo "   Response: ${BODY}"
  exit 1
fi
