#!/usr/bin/env bash
# DNS Migration Guide - Remove Protonmail, Use ApexMail Server
# This script shows the required DNS changes (does not make them automatically)
set -euo pipefail

# Server IPs
TEST_SERVER="157.90.126.205"
APP_SERVER="46.62.252.96"

cat << 'EOF'
================================================================================
          DNS MIGRATION: Protonmail → ApexMediation Mail Server
================================================================================

CURRENT DNS RECORDS (with Protonmail):
--------------------------------------
MX:
  10 mail.protonmail.ch
  20 mailsec.protonmail.ch

SPF:
  "v=spf1 include:amazonses.com ~all"
  "v=spf1 include:_spf.protonmail.ch ~all"

DMARC:
  "v=DMARC1; p=quarantine"


REQUIRED DNS CHANGES (remove Protonmail):
-----------------------------------------

1. UPDATE MX RECORD
   Delete:
     10 mail.protonmail.ch
     20 mailsec.protonmail.ch
   
   Add:
     10 mail.apexmediation.ee

2. ADD A RECORD FOR MAIL SERVER
   mail.apexmediation.ee → 157.90.126.205 (test server)
   
   OR for production:
   mail.apexmediation.ee → 46.62.252.96 (app server)

3. UPDATE SPF RECORD
   Delete: "v=spf1 include:_spf.protonmail.ch ~all"
   
   Update to single record:
   "v=spf1 include:amazonses.com ip4:157.90.126.205 -all"
   
   Note: -all (hard fail) is more secure than ~all (soft fail)

4. ADD DKIM RECORD (after generating key)
   apex2026._domainkey.apexmediation.ee → "v=DKIM1; k=rsa; p=<PUBLIC_KEY>"
   
   Generate DKIM key:
   openssl genrsa -out /etc/apex-mail/dkim/apex2026.private 2048
   openssl rsa -in /etc/apex-mail/dkim/apex2026.private -pubout -outform PEM | \
     grep -v "^-" | tr -d '\n'

5. KEEP DMARC (no change needed)
   "v=DMARC1; p=quarantine"

6. SET REVERSE DNS (PTR) on Hetzner
   157.90.126.205 → mail.apexmediation.ee
   
   Do this in Hetzner Cloud Console:
   - Go to your server
   - Click "Networking"
   - Set rDNS to: mail.apexmediation.ee


VERIFICATION COMMANDS:
----------------------
# Check MX records
dig +short MX apexmediation.ee

# Check SPF
dig +short TXT apexmediation.ee | grep spf

# Check DMARC
dig +short TXT _dmarc.apexmediation.ee

# Check DKIM
dig +short TXT apex2026._domainkey.apexmediation.ee

# Check reverse DNS
dig +short -x 157.90.126.205

# Test SMTP connectivity
telnet mail.apexmediation.ee 25


HETZNER FIREWALL RULES:
-----------------------
Ensure these ports are open on the mail server:
- 25/tcp   (SMTP inbound)
- 587/tcp  (Submission)
- 993/tcp  (IMAPS)
- 80/tcp   (Let's Encrypt)
- 443/tcp  (HTTPS/webmail)

================================================================================
EOF

echo ""
echo "Current DNS status for apexmediation.ee:"
echo "========================================="
echo ""
echo "MX Records:"
dig +short MX apexmediation.ee 2>/dev/null || echo "  (DNS lookup failed)"
echo ""
echo "SPF Records:"
dig +short TXT apexmediation.ee 2>/dev/null | grep -i spf || echo "  (No SPF found)"
echo ""
echo "DMARC Record:"
dig +short TXT _dmarc.apexmediation.ee 2>/dev/null || echo "  (No DMARC found)"
echo ""
