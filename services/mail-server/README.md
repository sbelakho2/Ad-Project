# ApexMediation Mail Server

A modular, security-focused mail server written in Rust for the apexmediation.ee domain.

> **⚠️ STANDALONE SERVER - NO PROTONMAIL DEPENDENCY**
>
> This mail server is completely self-contained:
> - **Outbound:** Uses Postfix MTA (direct MX delivery with DKIM)
> - **Inbound:** Our own SMTP server on port 25
> - **Storage:** PostgreSQL + local blob storage
> - **No Protonmail, no third-party mail providers in the mail flow**
>
> DNS MX records point to this server at mail.apexmediation.ee.

## Architecture

```
┌──────────────────────────────────────────────────────────────────────┐
│                         PROTOCOL EDGES                                │
│                                                                       │
│  ┌─────────────┐   ┌─────────────┐   ┌─────────────────────────┐    │
│  │ smtp-edge   │   │ submission  │   │ imap-edge               │    │
│  │ (Port 25)   │   │ (Port 587)  │   │ (Port 993)              │    │
│  │             │   │             │   │                         │    │
│  │ Inbound     │   │ Client      │   │ Mailbox access          │    │
│  │ SMTP        │   │ Sending     │   │ (IMAP4rev1/rev2)        │    │
│  └──────┬──────┘   └──────┬──────┘   └───────────┬─────────────┘    │
│         │                 │                       │                  │
└─────────┼─────────────────┼───────────────────────┼──────────────────┘
          │                 │                       │
          │  gRPC           │  gRPC                 │  gRPC
          ▼                 ▼                       ▼
┌──────────────────────────────────────────────────────────────────────┐
│                         CORE SERVICES                                 │
│                                                                       │
│  ┌─────────────────────────────────────────────────────────────────┐ │
│  │                    mailstore-core                                │ │
│  │                                                                  │ │
│  │  • Message storage (immutable blobs, SHA-256 addressed)         │ │
│  │  • Mailbox management (folders, flags, UIDs)                    │ │
│  │  • User/account management                                      │ │
│  │  • Quota enforcement                                            │ │
│  │  • Search indexing (full-text via Tantivy)                     │ │
│  │  • PostgreSQL metadata backend                                  │ │
│  └─────────────────────────────────────────────────────────────────┘ │
│                                                                       │
│  ┌────────────────────────┐   ┌────────────────────────────────────┐ │
│  │    outbound-queue      │   │        spam-gateway                │ │
│  │                        │   │                                    │ │
│  │  • Retry engine        │   │  • Rspamd HTTP integration         │ │
│  │  • Bounce processing   │   │  • Bayesian learning (spam/ham)    │ │
│  │  • Rate limiting       │   │  • DKIM signing                    │ │
│  │  • DSN generation      │   │  • SPF/DMARC verification          │ │
│  └────────────────────────┘   └────────────────────────────────────┘ │
│                                                                       │
└──────────────────────────────────────────────────────────────────────┘
```

## Crates

| Crate | Purpose |
|-------|---------|
| `mail-proto` | gRPC/Protobuf definitions for inter-service communication |
| `mail-common` | Shared types, utilities, and error handling |
| `mailstore-core` | Central message storage and mailbox management |
| `smtp-edge` | Inbound SMTP server (port 25) |
| `submission` | Authenticated client submission (port 587) |
| `imap-edge` | IMAP4rev1/rev2 mailbox access (port 993) |
| `outbound-queue` | Outbound delivery with retry logic |
| `spam-gateway` | Spam filtering via Rspamd HTTP API |

## Configuration

Configuration is via environment variables or `/etc/apex-mail/config.toml`:

```toml
[server]
hostname = "mail.apexmediation.ee"
domain = "apexmediation.ee"

[database]
url = "postgresql://mail:password@localhost/maildb"
max_connections = 20

[storage]
blob_path = "/var/lib/apex-mail/blobs"
index_path = "/var/lib/apex-mail/index"

[tls]
cert_path = "/etc/letsencrypt/live/mail.apexmediation.ee/fullchain.pem"
key_path = "/etc/letsencrypt/live/mail.apexmediation.ee/privkey.pem"

[rspamd]
url = "http://127.0.0.1:11333"
password = ""  # Optional

[smtp]
# Outbound delivery via local Postfix
relay_host = "localhost"
relay_port = 25

[dkim]
selector = "apex2026"
private_key_path = "/etc/apex-mail/dkim/apex2026.private"
```

## Building

```bash
# Install Rust (if needed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Build all crates
cargo build --release

# Run tests
cargo test
```

## Deployment (Test Server)

```bash
# Deploy to test server
./scripts/deploy-mail-server.sh test
```

## Security Features

- Memory-safe Rust implementation
- TLS 1.3 for all connections
- DKIM signing for outbound mail
- SPF/DMARC verification for inbound
- Rate limiting and abuse prevention
- Sandboxed systemd services

## License

MIT License - ApexMediation 2026
