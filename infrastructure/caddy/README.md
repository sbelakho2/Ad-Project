# Caddy edge

One host serves `apex-api` on `127.0.0.1:8787`; the other serves the built
Zola site from `/srv/site/public`. Set `APEX_DOMAIN` and `SITE_DOMAIN` in the
environment before starting Caddy. Validate with:

```sh
caddy validate --config infrastructure/caddy/Caddyfile --adapter caddyfile
```
