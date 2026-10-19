# Console single sign-on and session model

This document describes the current customer console: how a publisher arrives from the marketing site, receives a one-time sign-in link, exchanges it for a browser session, and acts inside the console. The old Next.js and Express description is superseded; the [historical integration document](../Architecture/WEBSITE_DASHBOARD_AUTH_INTEGRATION.md) is kept for the record.

## Public hosts

The edge serves three public hostnames plus one internal control host. The mapping lives in `infrastructure/caddy/Caddyfile`.

| Host | What it serves |
|---|---|
| Marketing (site) | Static Zola pages, plus POST `/free-audit` and `/audit-requested/*` proxied to apex-api |
| API | The whole origin proxied to apex-api, except `/metrics`, which answers 404 |
| Console | The whole origin proxied to apex-api, except `/metrics`, which answers 404 |
| Control | Internal only; apex-control binds loopback on the control host and is not on the public edge |

The console host owns its root path. GET `/` is the authenticated overview, `/console/*` holds the console pages, and `/assets/*`, `/favicon*`, `/watch*` and POST `/free-audit` reach the same service. The marketing host no longer proxies `/console`; it keeps the static audit form and the confirmation page.

## The one-time link journey

1. A visitor submits the free-audit form. The site host proxies POST `/free-audit` to apex-api.
2. The service stores the lead and resolves the workspace from the email domain.
3. A one-time link row is appended to `signin.jsonl` and the absolute link is emailed to a recorded contact.
4. GET `/console/signin?token=<token>` renders an inert page. It never consumes the token, so a mail scanner that follows the link cannot spend it.
5. The page posts the token to POST `/console/signin`. That call consumes the row and answers `303` with `Location: /`.
6. The session cookie opens GET `/` as the overview for the session's publisher.

The token is 32 lowercase hex characters (128 random bits). The raw token appears only in the email and in memory. The durable row keeps its SHA256 hash.

## `APEX_CONSOLE_URL` contract

`APEX_CONSOLE_URL` is the public console origin the emailed link starts with. It is required to issue links.

- The value must be an absolute `http` or `https` URL with a host and no query or fragment.
- `http` is accepted only for loopback dev hosts: `localhost`, a `*.localhost` name (RFC 6761 reserves the namespace) and `127.0.0.1`. Every other host requires `https`.
- Trailing slashes are normalized away.
- A configured value that fails validation refuses startup with a typed error naming the variable.
- An unset or blank value disables links. Startup logs `sign-in links are disabled: APEX_CONSOLE_URL is not configured`.

With a valid base, every link is exactly `{base}/console/signin?token=<32 hex>`. With the base unset, a free audit still stores the lead and answers `303` to `/audit-requested/?ref=...`. A proven workspace is recorded, but no sign-in row is created and no email is sent. The service records the typed `MissingBase` refusal (`missing_base`) in the operator log.

## `APEX_SUPPORT_URL` contract

`APEX_SUPPORT_URL` is the Help destination of the authenticated console pages. It follows the same absolute URL rules as `APEX_CONSOLE_URL`. The default is `https://apexmediation.ee/support/`. An invalid configured value refuses startup.

Console pages render the configured absolute URL in the top bar. The marketing shell used by the free-audit and watch forms keeps the on-site relative `/support/` link.

## Session cookie

The session cookie is `apex_ledger_session`. It carries an opaque 128-bit session id and no publisher data.

| Attribute | Value |
|---|---|
| `Path` | `/` |
| `HttpOnly` | always |
| `SameSite` | `Lax` |
| `Secure` | set when the request arrived over `https` |
| `Domain` | absent, so the cookie is host-only |
| `Max-Age` | 900 seconds |

The `Path=/` scope is what makes the post-sign-in GET `/` reach the overview. A cookie scoped to `/console` would never be sent to `/`.

Sessions live in memory. One session expires after 15 minutes of idleness and every successful read renews the window. The store keeps at most 1024 sessions and evicts the least recently used entry first. A process restart ends every session.

## CSRF

Every session carries its own 128-bit CSRF token. Mutating console forms include it in the hidden `csrf_token` field, and the handler compares it in constant time. The protected actions include session end, theme changes, connection changes, proposal decisions and settings changes. A missing or wrong token answers `400`.

The sign-in POST is the exception. It runs before a session exists, and the one-time link token is itself the capability being consumed.

## Ledger token exchange

The change log has a separate access panel at `/console/ledger`. An operator issues each publisher a token in `APEX_LEDGER_TOKENS_FILE`.

POST `/console/ledger` accepts `publisher` and `token`. On a match it opens the same browser session described above and answers `303` to `/console/ledger` with the session cookie. The publisher's bearer token is never stored in the cookie and never rendered. A wrong pair answers `401`; a deployment with no token file answers `503 ledger_auth_not_configured`.

Every console page resolves its publisher from the live session. No page falls back to a deployment-wide publisher.

## Workspaces and recorded contacts

A workspace is keyed by the email domain that proved ownership through its public `app-ads.txt` (`OWNERDOMAIN=` or `CONTACT=`). The studio name is only a display label on the lead. Two studios behind one proven domain share one workspace.

The workspace entry lives in the ledger token file. A sidecar named `<tokens file>.workspaces.json` records the owner email and the contact list. A new workspace records its creating address as owner and as its only contact.

An existing workspace accepts a sign-in link only for an address already in its contact list. A free-audit submission never extends that list. An operator-created token has no contacts and can never receive a self-serve link. A refused address produces a typed workspace refusal, and no link row is created.

## Expiry, used links and replay

`APEX_SIGNIN_TTL_HOURS` sets the link lifetime. The default is 72 hours and the accepted range is 1 to 720; any other value refuses startup.

- GET on a live link renders the confirmation page and leaves the row untouched.
- POST consumes the row once and mints the session.
- POST on an expired, used or unknown token answers `200` with the refusal page and sets no cookie.
- A second POST of the same token is the same refusal.
- Consuming a link appends a hash-only `consumed` marker. Readers fold the marker into the row's `used_at`.
- Single use is enforced while the store lock is held. Sessions already minted live out their own idle window.

## Sign-out

POST `/console/ledger/session/end` requires a live session and its CSRF token. It closes the session and answers `303` to `/console/ledger` with `apex_ledger_session=; Path=/; Max-Age=0; HttpOnly; SameSite=Lax`. An expired or unknown session is already ended, so the route still answers `303` and clears the cookie.

## Ledger token endpoint

GET `/v1/ledger/{publisher}` is the machine read of a publisher's change log. It requires `Authorization: Bearer <publisher token>` and answers `401` without a valid token. A deployment with no token file answers `503`. The console session cookie is not accepted on this endpoint, and the ledger bearer is not accepted as a console session.

## Operator plane

The operator control plane is separate from the publisher console. It runs as apex-control on the internal control host and binds loopback; the public edge has no block for it. Operators sign in with an operator id, a password verified against an Argon2id hash, and a time-based one-time authenticator code. The operator session is separate from `apex_ledger_session`, and a publisher console cookie is not accepted there. See the [control plane overview](../../Architecture/CONTROL_PLANE.md). The retired Next.js two-factor design is kept in the [historical two-factor document](../Security/TWO_FACTOR_AUTH.md).

## Proposal review and approvals

The cycle publishes a proposal file at
`$APEX_PROPOSALS_ROOT/<proposal_id>.json` whenever a change is held for
approval. The envelope records the owning publisher, the exact runner
proposal, the snapshot reference, the policy version, the approval window and
the planned test settings. The console lists and opens only files whose
`publisher` equals the signed-in workspace; another workspace's id answers
`404` and no decision can be recorded against it.

A decision is written beside the proposal as `<proposal_id>.approval.json`
with the decision, the acting publisher, the change hash (SHA-256 over the
stored proposal bytes), the snapshot and the policy version. The next cycle
reads the decision and applies the change only when every binding matches;
approvals do not bypass any other policy gate. A decline is final for that
exact change: it is skipped with `approval_declined` even after the
time-based approval window ends. Approving records the decision and the
following cycle is what writes to the platform, so an approval is never a
silent instant mutation.

## Workspace ownership

A self-serve workspace is keyed by the email domain's publisher slug. The
first successful claim records the owner address, the contact set and the
domain that proved ownership in the workspace sidecar. Every later request
must match both a recorded contact and the recorded domain; a different
domain whose slug collides with the key is refused exactly like an
unrecorded address. Existing records written before the binding (no recorded
domain) keep working for their recorded contacts and are not rewritten.

An operator-provisioned ledger token is not by itself a workspace. It can be
claimed once by a submission whose app-ads.txt proof passes; that claim
records the owner and domain. Once claimed, no further address or domain is
added. An unproven submission leaves the token unclaimed and sends no link.

A provisioned workspace that already holds runner data (a ledger record or a
snapshot) cannot be claimed by self-serve unless the operator declares the
owning domain in `APEX_WORKSPACE_DOMAINS`. This prevents a domain whose slug
collides with the key from claiming the data by being first. New, empty
workspaces still claim through the app-ads.txt proof without a declaration.
