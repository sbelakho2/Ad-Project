# Caddy edge

The public edge layout, run next to apex-api on the console host. Set the
host names and the access-log path in the environment before starting Caddy:

- `SITE_DOMAIN` (default `www.apexmediation.ee`): the marketing origin. It
  serves the built Zola site from `/srv/site/public` and proxies only
  `POST /free-audit` and `/audit-requested/*` to `apex-api` on
  `127.0.0.1:8787`. `GET /free-audit` and every other page stay static, and
  `/console` no longer has a route there.
- `APEX_DOMAIN` (default `api.apexmediation.ee`): the API origin, proxied
  whole to `apex-api` on `127.0.0.1:8787`.
- `CONSOLE_DOMAIN` (default `console.apexmediation.ee`): the console origin,
  proxied whole to `apex-api` on `127.0.0.1:8787`. Its overview is `/`,
  `/console/*` the console pages, and `/assets/*`, `/favicon*`, `/watch*`
  and `POST /free-audit` all belong to the same service. Set
  `APEX_CONSOLE_URL` to `https://<CONSOLE_DOMAIN>` so emailed sign-in links
  land on this origin.
- `APEX_ACCESS_LOG` (default `/var/log/caddy/apex-access.json`): the JSON
  access log on the marketing origin for `apex-ingest`. The directory must
  exist and be writable by the Caddy process
  (`install -d -o caddy -g caddy /var/log/caddy` on a Debian host).

The operator origin is not on this edge. The control plane runs on the
control host, binds loopback there and carries no public host block
(`infrastructure/README.md`, "Hosting split").

Routing table:

| host | request | destination |
|---|---|---|
| `SITE_DOMAIN` | `POST /free-audit` | `apex-api` (`127.0.0.1:8787`), method and body preserved |
| `SITE_DOMAIN` | `/audit-requested/*` | `apex-api` |
| `SITE_DOMAIN` | everything else (GET pages, `/console*`, `/fonts/*`, assets) | static `file_server` from `/srv/site/public` |
| `APEX_DOMAIN` | everything except `/metrics` | `apex-api` |
| `APEX_DOMAIN` | `/metrics` | 404 |
| `CONSOLE_DOMAIN` | everything except `/metrics` | `apex-api`; `/` is the console overview |
| `CONSOLE_DOMAIN` | `/metrics` | 404 |

No route reaches ClickHouse or the control plane: the only upstream in the
config is `apex-api`, and the contract test fails on any other dial, on any
`8790` mention and on a returning operator host block.

Validate and run the routing contract (the contract adapts the file with fixed
test domains and asserts the table above):

```sh
caddy validate --config infrastructure/caddy/Caddyfile --adapter caddyfile
sh infrastructure/caddy/tests/contract.sh
```

CI runs the two commands above after `apk add --no-cache python3`; the
`caddy validate` step sets `APEX_ACCESS_LOG=/tmp/apex-access.json` so
validation never needs the production log directory.

## Access log and privacy

The marketing origin writes one JSON record per request. Records carry the
standard Caddy fields `apex-ingest` reads: `ts`, `status` and
`request.uri`/`request.headers` (with the query string, which the parser
reduces to allow-listed parameter **keys** and the campaign code only). The
parser never stores a query value, a client address, a raw user agent or the
raw path; at-rest handling of the raw file is documented in
`infrastructure/README.md`.
