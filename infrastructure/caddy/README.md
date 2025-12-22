# Caddy (Production Reverse Proxy)

This repo-managed Caddy config is the canonical edge routing for the Hetzner deployment.

## Files

- Caddyfile: `infrastructure/caddy/Caddyfile`

## Required environment variables

- `CADDY_EMAIL` — ACME/Let's Encrypt registration email

## Routes

- `admin.apexmediation.ee` → `control-plane:4000`
- `api.apexmediation.ee` → `backend:8080`
- `console.apexmediation.ee` → `console:3000`
- `apexmediation.ee` / `www.apexmediation.ee` → `website:3002`

Note: `admin.apexmediation.ee` must route ONLY to the control-plane. Do not proxy any admin UI paths to the customer console.
