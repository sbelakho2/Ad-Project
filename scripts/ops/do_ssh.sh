#!/usr/bin/env bash
# DEPRECATED: This script was for DigitalOcean. Use hetzner_ssh.sh instead.
#
# Production has moved to Hetzner:
#   App Server: 46.62.252.96 (apexmediation.ee)
#   DB Server:  77.42.44.35
#
# Usage:
#   ./scripts/ops/hetzner_ssh.sh        # SSH to app server (default)
#   ./scripts/ops/hetzner_ssh.sh db     # SSH to database server

echo "[DEPRECATED] do_ssh.sh is obsolete. Use hetzner_ssh.sh instead:" >&2
echo "  ./scripts/ops/hetzner_ssh.sh        # SSH to app server" >&2
echo "  ./scripts/ops/hetzner_ssh.sh db     # SSH to database server" >&2
exit 1
