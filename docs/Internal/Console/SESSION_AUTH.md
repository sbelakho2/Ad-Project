Console Session Auth and CSRF Model

This document describes how the Console web app authenticates to the Backend using secure cookie sessions and CSRF protection.

Overview
- Session transport: httpOnly, secure cookies issued by the Backend on login/register/refresh.
- Token names: configurable via env (ACCESS_TOKEN_COOKIE_NAME, REFRESH_TOKEN_COOKIE_NAME); defaults: access_token, refresh_token.
- CSRF protection: double-submit cookie strategy using a readable cookie (default XSRF-TOKEN) and an X-CSRF-Token header on mutating requests.
- CORS: strict allowlist; withCredentials: true is required on client requests.

Backend endpoints
- Issue CSRF token: GET /api/v1/auth/csrf (sets XSRF-TOKEN cookie and returns { token }).
- Session user: GET /api/v1/auth/me (requires valid access token; reads cookie or Bearer).
- Logout: POST /api/v1/auth/logout (revokes refresh tokens and clears cookies).
- Auth flows (CSRF-exempt by design): POST /api/v1/auth/{login|register|refresh}.

Console client behavior
- Axios instances are created with withCredentials: true so cookies are sent automatically.
- A request interceptor injects X-CSRF-Token for mutating methods (POST/PUT/PATCH/DELETE):
  - If the CSRF cookie is missing, the client first calls /auth/csrf to obtain one.
  - Then it reads the XSRF-TOKEN cookie and sets X-CSRF-Token header.
- On 401 responses, the client redirects to /login.

RBAC
- Backend exposes authorize([...]) middleware; revenue routes are protected with publisher|admin roles.
- Console uses a simple useSession() hook to fetch /auth/me and can gate UI via helper functions in console/src/lib/rbac.ts.
- Server-side RBAC enforcement via layouts (settings/layout.tsx, billing/layout.tsx) prevents direct URL access bypass.
- See RBAC_SYSTEM.md for detailed role permissions and implementation.

Two-Factor Authentication (2FA)
- TOTP-based 2FA with backup codes
- User management via Settings → Security & 2FA
- Backend service: backend/src/services/twofa.service.ts
- See TWO_FACTOR_AUTH.md in Security folder for implementation details.

Configuration
- Backend:
  - CORS_ALLOWLIST: comma-separated origins allowed to call the API with credentials.
  - COOKIE_SECURE, COOKIE_SAMESITE, COOKIE_DOMAIN: cookie attributes.
  - ACCESS_TOKEN_COOKIE_NAME, REFRESH_TOKEN_COOKIE_NAME: cookie names.
  - CSRF_COOKIE_NAME: name of the readable CSRF cookie (default XSRF-TOKEN).
- Console:
  - NEXT_PUBLIC_API_URL: base URL for API (e.g., http://localhost:4000/api/v1).
  - NEXT_PUBLIC_CSRF_COOKIE_NAME: overrides the name the client looks for (should match backend CSRF_COOKIE_NAME).

File references
- Backend: backend/src/middleware/csrf.ts, backend/src/middleware/auth.ts, backend/src/controllers/auth.controller.ts, backend/src/routes/auth.routes.ts, backend/src/routes/revenue.routes.ts, backend/src/utils/openapi.ts.
- Console: console/src/lib/api-client.ts, console/src/lib/csrf.ts, console/src/lib/useSession.ts, console/src/lib/rbac.ts, console/src/app/providers.tsx.
