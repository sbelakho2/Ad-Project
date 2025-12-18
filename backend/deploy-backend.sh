#!/usr/bin/env bash
# DEPRECATION NOTICE: legacy Fly.io deployment script (do not use)
#
# This script is intentionally disabled. The production platform runs on Hetzner:
# - App Server: 46.62.252.96 (apexmediation.ee)
# - DB Server: 77.42.44.35 (PostgreSQL 16)
# - Cache: Redis on app server with password authentication
# - TLS: Caddy with auto Let's Encrypt (HTTPS for all domains)
#
# What to use instead:
# - Follow `docs/Architecture/HETZNER_POSTGRES_DESIGN.md`
# - Follow `docs/Internal/Deployment/HETZNER_PRODUCTION_CHECKLIST.md`
# - Use docker-compose on the app server
# - SSH access: ssh -i ~/.ssh/hetzner-db-mac root@<IP>
#
# If you need a CI/CD deploy, add a Hetzner-specific GitHub Action or runbook.

echo "[DEPRECATED] backend/deploy-backend.sh is disabled. Use Hetzner deployment docs." 1>&2
echo "See: docs/Architecture/HETZNER_POSTGRES_DESIGN.md and docs/Internal/Deployment/HETZNER_PRODUCTION_CHECKLIST.md" 1>&2
exit 2
