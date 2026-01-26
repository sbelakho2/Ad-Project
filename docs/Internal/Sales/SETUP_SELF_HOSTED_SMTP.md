# Self-Hosted Inbound SMTP (Zero Cost)

This guide explains how to set up the zero-cost, self-hosted SMTP server for handling inbound email replies (e.g., from leads).

**Goal:** Receive emails on `replies.apexmediation.ee` and forward them to the Control Plane API without using third-party services like SendGrid or Cloudflare.

> **⚠️ Architecture Note (January 2026):** The inbound-smtp service runs on the **Test Server** (157.90.126.205:2525), not the App Server.
> It forwards to the local sales-api (localhost:4001) which handles all sales automation.
> The App Server control-plane is strictly a proxy client.

## Architecture

```mermaid
sequenceDiagram
    participant Sender as Lead (Email Client)
    participant FW as Firewall (iptables)
    participant SMTP as inbound-smtp (Test Server:2525)
    participant API as sales-api (Test Server:4001)

    Sender->>FW: SMTP Connect (Port 25)
    FW->>SMTP: Redirect to Port 2525
    SMTP->>SMTP: Parse Email (MIME)
    SMTP->>API: POST /api/autonomous/webhook/sendgrid (JSON)
    active participant API
    API-->>SMTP: 200 OK
    deactivate API
    SMTP-->>Sender: 250 OK (Message Accepted)
```

**Server Locations:**
| Component | Server | Port |
|-----------|--------|------|
| inbound-smtp | Test Server (157.90.126.205) | 2525 (via iptables from 25) |
| sales-api | Test Server (157.90.126.205) | 4001 |
| Control Plane (proxy) | App Server (46.62.252.96) | 4000 |

## Prerequisites

1. **DNS Records**:
   - `A` record for `replies.apexmediation.ee` pointing to your server IP.
   - `MX` record for `apexmediation.ee` (or `replies.apexmediation.ee`) pointing to `replies.apexmediation.ee`.
   
2. **Ports**:
   - Port `25` must be open on your Hetzner firewall / Security Group.

## Installation & Setup

The script is located at `control-plane/scripts/inbound-smtp.ts`.

### 1. Install Dependencies
(Already done in the repo)
```bash
cd control-plane
npm install smtp-server mailparser
npm install --save-dev @types/smtp-server @types/mailparser
```

### 2. Configure Firewall Redirect (Port 25 -> 2525)
Node.js cannot bind to port 25 without root privileges. We run the script on port 2525 and use `iptables` to redirect traffic.

**Run on Server (as root):**
```bash
# Redirect TCP 25 to 2525
iptables -t nat -A PREROUTING -p tcp --dport 25 -j REDIRECT --to-port 2525

# Save rules (persistent across reboots)
apt-get install iptables-persistent
netfilter-persistent save
```

### 3. Start the Service (systemd on Test Server)
The service runs as `inbound-smtp.service` on the Test Server.

```bash
# SSH to Test Server
./scripts/ops/hetzner_ssh.sh test

# Check service status
sudo systemctl status inbound-smtp.service

# Start/restart service
sudo systemctl restart inbound-smtp.service
```

### 4. Verification

**Check Logs (Test Server):**
```bash
./scripts/ops/hetzner_ssh.sh test -- "sudo journalctl -u inbound-smtp.service -n 50"
```
Expected output:
> ✅ Inbound SMTP Server listening on 0.0.0.0:2525
> Forwarding to: http://localhost:4001/api/autonomous/webhook/sendgrid

**Test via Local Curl:**
```bash
curl -v smtp://localhost:2525 --mail-from test@example.com --mail-rcpt test@replies.apexmediation.ee --upload-file - <<EOF
From: test@example.com
To: test@replies.apexmediation.ee
Subject: Self-Hosted Test

This is a test verifying the homespun SMTP server.
EOF
```

## Troubleshooting

- **Connection Refused on Port 25?** Check `iptables -t nat -L -v` on Test Server to ensure the rule exists.
- **Webhook Failed?** Ensure the sales-api is running on Test Server (`./scripts/ops/hetzner_ssh.sh test -- "pm2 status"`).
- **Spam Issues?** This is a basic server. For production, consider adding basic spam filtering logic or IP reputation checks in `inbound-smtp.ts`.
