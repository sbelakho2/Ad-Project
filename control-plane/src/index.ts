/**
 * CONTROL PLANE - MAIN ENTRY POINT
 *
 * Separate control plane service for system administration.
 * Runs on its own port (4000) with its own hostname (admin.apexmediation.ee)
 *
 * Security layers:
 * 1. Operator JWT authentication - separate from customer auth
 * 2. Tiered action permissions - confirmation required for dangerous ops
 * 3. Full audit logging with 7-year retention
 */

import 'dotenv/config';
import express, { Request, Response, NextFunction } from 'express';
import helmet from 'helmet';
import cors from 'cors';
import compression from 'compression';
import cookieParser from 'cookie-parser';
import rateLimit from 'express-rate-limit';

import { loadConfig } from './config';
import { logger, logAudit } from './logger';
import { initializePools, closePools, getReadOnlyPool, queryReadOnly } from './database';
import routes from './routes';
import { initializeSalesScheduler, stopSalesScheduler } from './services/sales-scheduler';
import { ensureCsrfCookie, validateCsrfToken, CSRF_COOKIE_NAME, CSRF_HEADER_NAME } from './csrf';
// NOTE: Orchestrator removed - app server is strictly a proxy client
// All sales automation work runs on test server (157.90.126.205:4001)

// Load and validate configuration
const config = loadConfig();

const app = express();

// ============================================
// SECURITY MIDDLEWARE
// ============================================

// Helmet for security headers (P1 Fix #6: Enhanced security headers)
app.use(helmet({
  contentSecurityPolicy: {
    directives: {
      defaultSrc: ["'self'"],
      scriptSrc: ["'self'", "'unsafe-inline'"],
      scriptSrcAttr: ["'unsafe-inline'"],
      styleSrc: ["'self'", "'unsafe-inline'"],
      imgSrc: ["'self'", 'data:'],
    },
  },
  crossOriginOpenerPolicy: { policy: 'same-origin' },
  crossOriginResourcePolicy: { policy: 'same-origin' },
  referrerPolicy: { policy: 'strict-origin-when-cross-origin' },
  hsts: {
    maxAge: 31536000,
    includeSubDomains: true,
    preload: true,
  },
  xssFilter: true,
  noSniff: true,
  frameguard: { action: 'deny' },
}));

// CORS - only allow same origin (admin.* subdomain)
app.use(cors({
  origin: config.CORS_ORIGIN || false,
  credentials: true,
}));

// Cookie parsing for CSRF tokens
app.use(cookieParser());

// Trust proxy for accurate IP detection behind Caddy/nginx
app.set('trust proxy', true);

// ============================================
// RATE LIMITING (P1 Security Fix #2)
// ============================================

// Global rate limiter - protects all control-plane endpoints
// Control-plane is operator-only, so limits are reasonable but protective
const globalRateLimiter = rateLimit({
  windowMs: 15 * 60 * 1000, // 15 minutes
  max: 500, // 500 requests per 15-minute window per IP
  standardHeaders: true,
  legacyHeaders: false,
  message: {
    error: 'Too many requests',
    message: 'Rate limit exceeded for control-plane API',
    retryAfterMinutes: 15,
  },
  skip: (req: Request) => {
    // Skip health checks to avoid counting monitoring traffic
    return req.path === '/health' || req.path === '/health/ready';
  },
});

app.use(globalRateLimiter);

// Strict rate limiter for sensitive operations (dangerous actions)
export const sensitiveOperationRateLimiter = rateLimit({
  windowMs: 60 * 60 * 1000, // 1 hour
  max: 10, // 10 dangerous operations per hour
  standardHeaders: true,
  legacyHeaders: false,
  message: {
    error: 'Too many sensitive operations',
    message: 'Rate limit exceeded for dangerous operations. Wait before retrying.',
    retryAfterMinutes: 60,
  },
});

// ============================================
// STANDARD MIDDLEWARE
// ============================================

// Compression
app.use(compression());

// JSON body parsing
app.use(express.json({ limit: '1mb' }));

// ============================================
// CSRF PROTECTION
// ============================================

// Ensure CSRF cookie is set on all HTML page loads
app.use(ensureCsrfCookie);

// Validate CSRF token on all state-changing requests
// Exempt API routes that use bearer token auth (already protected)
app.use((req: Request, res: Response, next: NextFunction) => {
  const stateMutatingMethods = ['POST', 'PUT', 'PATCH', 'DELETE'];
  
  // Skip CSRF for safe methods
  if (!stateMutatingMethods.includes(req.method)) {
    return next();
  }
  
  // Skip CSRF for API routes that use Authorization header (bearer token auth)
  // These are already protected by verifying the JWT token
  const authHeader = req.headers.authorization;
  if (authHeader && authHeader.startsWith('Bearer ')) {
    return next();
  }
  
  // Skip CSRF for login route - user doesn't have a token yet
  // Login is protected by rate limiting and device confirmation instead
  if (req.path === '/api/v1/auth/login') {
    return next();
  }
  
  // Skip CSRF for ApexMail API calls with valid internal API key
  // This allows service-to-service communication without CSRF tokens
  const apexMailKey = req.headers['x-apexmail-key'] as string | undefined;
  const expectedKey = process.env.APEXMAIL_API_KEY;
  if (
    req.path.startsWith('/api/apexmail/') && 
    apexMailKey && 
    expectedKey && 
    apexMailKey === expectedKey
  ) {
    return next();
  }
  
  // Skip CSRF for ApexMail API calls from localhost (internal service calls)
  // This is safe because the service is only exposed on localhost/internal network
  const clientIp = req.ip || req.socket.remoteAddress || '';
  if (
    req.path.startsWith('/api/apexmail/') && 
    (clientIp === '127.0.0.1' || clientIp === '::1' || clientIp === '::ffff:127.0.0.1')
  ) {
    return next();
  }
  
  // Apply CSRF validation for cookie-based auth (form submissions, etc)
  return validateCsrfToken(req, res, next);
});

// ============================================
// MINIMAL OPERATOR UI (same-origin)
// ============================================

function sendHtml(res: Response, html: string): void {
  res.setHeader('content-type', 'text/html; charset=utf-8');
  res.setHeader('cache-control', 'no-store, no-cache, must-revalidate, proxy-revalidate');
  res.setHeader('pragma', 'no-cache');
  res.setHeader('expires', '0');
  res.status(200).send(html);
}

function escapeHtml(input: string): string {
  return input
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#39;');
}

app.get('/', (req: Request, res: Response) => {
  res.redirect('/admin');
});

app.get('/admin/login', (req: Request, res: Response) => {
  const title = 'Operator Login';
  sendHtml(
    res,
    `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>${escapeHtml(title)}</title>
    <style>
      :root {
        color-scheme: light;
        --rams-chassis: #F2F2F2;
        --rams-module: #E6E6E6;
        --rams-panel: #D9D9D9;
        --rams-line: #CCCCCC;
        --rams-muted: #999999;
        --rams-foreground: #1A1A1A;
        --rams-orange: #FFBE00;
        --rams-green: #2D8C3C;
        --rams-red: #D62D2D;
        --rams-steel: #4A90E2;
        --nav-stack-height: 96px;
      }
      *, *::before, *::after { box-sizing: border-box; }
      body {
        margin: 0;
        font-family: "Geist Sans", Inter, system-ui, -apple-system, Segoe UI, Roboto, Helvetica, Arial, sans-serif;
        background: var(--rams-chassis);
        color: var(--rams-foreground);
        text-rendering: optimizeLegibility;
        -webkit-font-smoothing: antialiased;
        -moz-osx-font-smoothing: grayscale;
        font-feature-settings: "kern" 1, "liga" 1;
      }
      .wrap { min-height: 100vh; display: grid; place-items: center; padding: 24px; }
      .card {
        width: 100%;
        max-width: 440px;
        background: var(--rams-module);
        border: 1px solid var(--rams-line);
        border-radius: 2px;
        padding: 20px;
      }
      h1 { margin: 0 0 6px; font-size: 20px; letter-spacing: 0.02em; }
      p { margin: 0 0 18px; color: var(--rams-muted); font-size: 12px; line-height: 16px; }
      label { display: block; font-size: 10px; color: var(--rams-muted); margin: 12px 0 6px; text-transform: uppercase; letter-spacing: 0.08em; font-weight: 700; }
      input {
        width: 100%;
        padding: 10px 12px;
        border-radius: 2px;
        border: 1px solid var(--rams-line);
        background: var(--rams-panel);
        color: var(--rams-foreground);
        box-shadow: inset 0 1px 0 rgba(0,0,0,0.04);
      }
      input:focus { outline: 2px solid var(--rams-orange); outline-offset: 2px; }
      button {
        width: 100%;
        margin-top: 16px;
        padding: 10px 12px;
        border-radius: 2px;
        border: 1px solid var(--rams-line);
        background: var(--rams-orange);
        color: var(--rams-foreground);
        font-weight: 700;
        cursor: pointer;
        text-transform: uppercase;
        letter-spacing: 0.08em;
        font-size: 12px;
      }
      button:disabled { opacity: .6; cursor: not-allowed; }
      .err {
        display:none;
        margin-top: 12px;
        padding: 10px 12px;
        border-radius: 2px;
        border: 1px solid var(--rams-red);
        background: rgba(214,45,45,.08);
        color: var(--rams-red);
        font-size: 12px;
      }
      .foot { margin-top: 16px; color: var(--rams-muted); font-size: 10px; text-transform: uppercase; letter-spacing: 0.08em; }
      .muted { color: var(--rams-muted); }
    </style>
  </head>
  <body>
    <div class="wrap">
      <div class="card">
        <h1>Operator Access</h1>
        <p>Control Plane login (operator-only). Customer accounts do not apply.</p>

        <form id="login-form">
          <label for="username">Username / Email</label>
          <input id="username" name="username" autocomplete="username" placeholder="operator or you@apexmediation.ee" required />

          <label for="password">Password</label>
          <input id="password" name="password" type="password" autocomplete="current-password" required />

          <div id="error" class="err"></div>

          <button id="submit" type="submit">Sign in</button>
        </form>

        <div class="foot">
          <div>All operator actions are logged.</div>
        </div>
      </div>
    </div>

    <script src="/static/admin-login.js"></script>
  </body>
</html>`
  );
});

app.get('/admin', (req: Request, res: Response) => {
  const title = 'Control Plane';
  res.setHeader('cache-control', 'no-store, max-age=0, must-revalidate');
  res.setHeader('pragma', 'no-cache');
  res.setHeader('expires', '0');
  sendHtml(
    res,
    `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>${escapeHtml(title)}</title>
    <style>
      :root {
        color-scheme: light;
        --rams-chassis: #F2F2F2;
        --rams-module: #E6E6E6;
        --rams-panel: #D9D9D9;
        --rams-line: #CCCCCC;
        --rams-muted: #999999;
        --rams-foreground: #1A1A1A;
        --rams-orange: #FFBE00;
        --rams-green: #2D8C3C;
        --rams-red: #D62D2D;
        --rams-steel: #4A90E2;
      }
      body {
        margin: 0;
        font-family: "Geist Sans", Inter, system-ui, -apple-system, Segoe UI, Roboto, Helvetica, Arial, sans-serif;
        background: var(--rams-chassis);
        color: var(--rams-foreground);
        display: flex;
        min-height: 100vh;
        overflow: hidden;
        text-rendering: optimizeLegibility;
        -webkit-font-smoothing: antialiased;
        -moz-osx-font-smoothing: grayscale;
        font-feature-settings: "kern" 1, "liga" 1;
      }

      .frame {
        flex: 1;
        height: 100vh;
        display: grid;
        grid-template-rows: 32px auto 1fr;
      }
      .status-bar {
        height: 32px;
        background: var(--rams-panel);
        border-bottom: 1px solid var(--rams-line);
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding: 0 12px;
        font-size: 10px;
        text-transform: uppercase;
        letter-spacing: 0.12em;
        font-weight: 700;
        color: var(--rams-muted);
        position: static;
      }
      .status-bar .status-pill { color: var(--rams-green); }
      .status-bar a { color: var(--rams-red); text-decoration: none; }

      .main { display: contents; }
      .header {
        border-bottom: 1px solid var(--rams-line);
        background: var(--rams-module);
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding: 12px 16px;
        gap: 16px;
        flex-wrap: wrap;
        position: static;
      }
      .brand { font-size: 12px; font-weight: 800; letter-spacing: 0.12em; text-transform: uppercase; }
      .nav { display: flex; gap: 8px; flex-wrap: wrap; }
      .nav-item {
        padding: 6px 10px;
        border: 1px solid transparent;
        cursor: pointer;
        color: var(--rams-muted);
        font-size: 11px;
        display: inline-flex;
        text-decoration: none;
        text-transform: uppercase;
        letter-spacing: 0.08em;
        font-weight: 700;
      }
      .nav-item:hover { border-color: var(--rams-line); background: var(--rams-panel); color: var(--rams-foreground); }
      .nav-item.active { border-color: var(--rams-orange); color: var(--rams-foreground); }

      .content {
        flex: 1;
        min-height: 0;
        padding: 16px 16px 160px;
        overflow-y: auto;
        overscroll-behavior: contain;
        scroll-padding-bottom: calc(160px + env(safe-area-inset-bottom));
      }
      .view { padding-bottom: 48px; }
      .view .card:last-child { margin-bottom: 48px; }
      #view-data-explorer { padding-bottom: 160px; }
      #view-data-explorer .card { overflow: visible; }
      #view-data-explorer table { margin-bottom: 24px; }
      .scroll-spacer { height: 160px; }

      .grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(260px, 1fr)); gap: 16px; margin-bottom: 16px; }
      .card { background: var(--rams-module); border: 1px solid var(--rams-line); border-radius: 2px; padding: 16px; }
      .card-header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 12px; gap: 8px; }
      .card-title { font-size: 10px; font-weight: 800; color: var(--rams-muted); text-transform: uppercase; letter-spacing: 0.12em; }

      table { width: 100%; border-collapse: collapse; font-size: 12px; }
      th, td { padding: 8px 10px; border-bottom: 1px solid var(--rams-line); text-align: left; }
      th { color: var(--rams-muted); font-weight: 700; text-transform: uppercase; letter-spacing: 0.08em; font-size: 10px; }
      tr:hover { background: var(--rams-panel); }

      .pill { padding: 2px 8px; border-radius: 2px; border: 1px solid var(--rams-line); font-size: 10px; text-transform: uppercase; letter-spacing: 0.08em; }
      .pill-success { border-color: var(--rams-green); color: var(--rams-green); }
      .pill-warning { border-color: var(--rams-orange); color: var(--rams-orange); }
      .pill-danger { border-color: var(--rams-red); color: var(--rams-red); }

      .btn { padding: 6px 12px; border-radius: 2px; border: 1px solid var(--rams-line); background: var(--rams-module); color: var(--rams-foreground); cursor: pointer; font-size: 11px; text-transform: uppercase; letter-spacing: 0.08em; font-weight: 700; }
      .btn:hover { background: var(--rams-panel); }
      .btn-primary { background: var(--rams-orange); border-color: var(--rams-orange); }
      .btn-primary:hover { filter: brightness(0.95); }
      pre { margin: 0; font-family: "Geist Mono", ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace; font-size: 11px; overflow: auto; }

      .hidden { display: none !important; }

      /* Sales Hub Sub-Navigation (Sensei-Rams Module Bar) */
      .sales-sub-nav { padding: 8px 14px; min-width: 90px; text-align: center; text-transform: uppercase; letter-spacing: 0.08em; font-size: 11px; }
      .sales-sub-nav.active { background: var(--rams-orange); border-color: var(--rams-orange); }
      .sales-subview { transition: opacity 0.15s ease; }
      .sales-subview.hidden { opacity: 0; pointer-events: none; }
    </style>
  </head>
  <body>
    <div class="frame">
      <div class="status-bar">
        <div>STATION: SENSEI-01 │ OS: 3.0.1 │ BUILD: ${new Date().toISOString()}</div>
        <div><span class="status-pill">STATUS: OPERATIONAL</span> │ <a href="/admin/logout">SIGN OUT</a></div>
      </div>
      <div class="main">
        <div class="header">
          <div class="brand">APEX CONTROL PLANE</div>
          <nav class="nav">
            <a class="nav-item active" data-view="dashboard" href="#dashboard">Dashboard</a>
            <a class="nav-item" data-view="sales-hub" href="#sales-hub">Sales Hub</a>
            <a class="nav-item" data-view="compliance" href="#compliance">Compliance</a>
            <a class="nav-item" data-view="data-explorer" href="#data-explorer">Data Explorer</a>
            <a class="nav-item" data-view="analytics" href="#analytics">Business Analytics</a>
            <a class="nav-item" data-view="adv-analytics" href="#adv-analytics">Advanced Analytics</a>
            <a class="nav-item" data-view="reconciliation" href="#reconciliation">Reconciliation</a>
            <a class="nav-item" data-view="telemetry" href="#telemetry">System Telemetry</a>
            <a class="nav-item" data-view="partitions" href="#partitions">Partitions</a>
          </nav>
        </div>
        
        <div class="content">
          <div style="display:flex; align-items:center; justify-content:space-between; margin-bottom:12px; gap:12px;">
            <div id="view-title" class="card-title">Dashboard</div>
            <div id="global-status" class="pill pill-success">System Online</div>
          </div>
        <!-- Dashboard View -->
        <div id="view-dashboard" class="view">
          <div class="grid">
            <div class="card">
              <div class="card-title">Overall Health</div>
              <div id="health-status" style="font-size: 24px; font-weight: bold; margin-top: 10px;">Loading...</div>
            </div>
            <div class="card">
              <div class="card-title">Database Sessions</div>
              <div id="db-sessions" style="font-size: 24px; font-weight: bold; margin-top: 10px;">-</div>
            </div>
            <div class="card">
              <div class="card-title">App Storage</div>
              <div id="storage-app" style="font-size: 20px; font-weight: bold; margin-top: 10px;">-</div>
              <div id="storage-app-detail" style="font-size: 12px; color: #94a3b8; margin-top: 6px;">-</div>
            </div>
            <div class="card">
              <div class="card-title">DB Storage</div>
              <div id="storage-db" style="font-size: 20px; font-weight: bold; margin-top: 10px;">-</div>
              <div id="storage-db-detail" style="font-size: 12px; color: #94a3b8; margin-top: 6px;">-</div>
            </div>
            <div class="card">
              <div class="card-title">S3 Storage</div>
              <div id="storage-s3" style="font-size: 20px; font-weight: bold; margin-top: 10px;">-</div>
              <div id="storage-s3-detail" style="font-size: 12px; color: #94a3b8; margin-top: 6px;">-</div>
            </div>
          </div>
          <div class="card">
            <div class="card-header"><span class="card-title">Recent System Events</span></div>
            <pre id="recent-events">Loading system telemetry...</pre>
          </div>
          <div class="card">
            <div class="card-header"><span class="card-title">Actionable Insights</span></div>
            <div id="dashboard-insights" style="font-size: 13px; color: #94a3b8;">-</div>
          </div>
        </div>

        <!-- ============================================================ -->
        <!-- SALES HUB - Consolidated Sales Automation (Sensei-Rams v3.0) -->
        <!-- ============================================================ -->
        <div id="view-sales-hub" class="view hidden">
          <!-- Sub-Navigation Module -->
          <div class="card" style="margin-bottom: 16px; padding: 8px 12px; background: var(--rams-panel);">
            <div style="display: flex; gap: 4px; flex-wrap: wrap;">
              <button class="btn sales-sub-nav active" data-subview="controls" data-sales-action="show-subview">CONTROLS</button>
              <button class="btn sales-sub-nav" data-subview="suppression" data-sales-action="show-subview">SUPPRESSION</button>
              <button class="btn sales-sub-nav" data-subview="v1-sequences" data-sales-action="show-subview">V1 SEQUENCES</button>
              <button class="btn sales-sub-nav" data-subview="v2-discovery" data-sales-action="show-subview">V2 DISCOVERY</button>
              <button class="btn sales-sub-nav" data-subview="v2-messaging" data-sales-action="show-subview">V2 MESSAGING</button>
              <button class="btn sales-sub-nav" data-subview="inbox" data-sales-action="show-subview">INBOX</button>
              <button class="btn sales-sub-nav" data-subview="metrics" data-sales-action="show-subview">METRICS</button>
            </div>
          </div>

          <!-- ==================== CONTROLS SUBVIEW ==================== -->
          <div id="sales-subview-controls" class="sales-subview">
            <!-- Global Kill Switch - CRITICAL CONTROL -->
            <div class="card" style="margin-bottom: 16px;">
              <div class="card-header">
                <div style="display: flex; align-items: center; gap: 12px;">
                  <span class="card-title" style="color: var(--rams-red);">GLOBAL KILL SWITCH</span>
                  <span id="kill-switch-badge" class="pill pill-success">SENDS ENABLED</span>
                </div>
                <button id="kill-switch-btn" class="btn" style="background: var(--rams-red); border-color: var(--rams-red); color: white;" data-sales-action="toggle-kill-switch">EMERGENCY STOP</button>
              </div>
              <div style="font-size: 12px; color: var(--rams-muted); margin-top: 8px;">
                When activated, ALL outbound email sending is blocked across V1 and V2 systems. Use in emergencies only.
              </div>
              <div id="kill-switch-status" style="margin-top: 12px; font-size: 11px; font-family: monospace;"></div>
            </div>

            <!-- Safe Mode Status -->
            <div class="card" style="margin-bottom: 16px;">
              <div class="card-header">
                <div style="display: flex; align-items: center; gap: 12px;">
                  <span class="card-title">SAFE MODE STATUS</span>
                  <span id="safe-mode-badge" class="pill pill-success">NORMAL</span>
                </div>
                <button class="btn" data-sales-action="refresh-safe-mode">Refresh</button>
              </div>
              <div class="grid" style="margin-top: 12px;">
                <div style="text-align: center; padding: 12px; background: var(--rams-panel); border-radius: 2px;">
                  <div style="font-size: 10px; color: var(--rams-muted); text-transform: uppercase; letter-spacing: 0.1em;">Bounce Rate (7d)</div>
                  <div id="safe-mode-bounce" style="font-size: 24px; font-weight: bold; margin-top: 4px;">0.0%</div>
                </div>
                <div style="text-align: center; padding: 12px; background: var(--rams-panel); border-radius: 2px;">
                  <div style="font-size: 10px; color: var(--rams-muted); text-transform: uppercase; letter-spacing: 0.1em;">Complaint Rate (7d)</div>
                  <div id="safe-mode-complaint" style="font-size: 24px; font-weight: bold; margin-top: 4px;">0.00%</div>
                </div>
                <div style="text-align: center; padding: 12px; background: var(--rams-panel); border-radius: 2px;">
                  <div style="font-size: 10px; color: var(--rams-muted); text-transform: uppercase; letter-spacing: 0.1em;">Throttle Level</div>
                  <div id="safe-mode-throttle" style="font-size: 24px; font-weight: bold; margin-top: 4px;">100%</div>
                </div>
                <div style="text-align: center; padding: 12px; background: var(--rams-panel); border-radius: 2px;">
                  <div style="font-size: 10px; color: var(--rams-muted); text-transform: uppercase; letter-spacing: 0.1em;">Emails Sent (7d)</div>
                  <div id="safe-mode-sent" style="font-size: 24px; font-weight: bold; margin-top: 4px;">0</div>
                </div>
              </div>
            </div>

            <!-- System Toggles -->
            <div class="grid">
              <div class="card">
                <div class="card-header">
                  <span class="card-title">V1 OUTBOUND</span>
                  <span id="v1-enabled-badge" class="pill pill-success">ENABLED</span>
                </div>
                <div style="font-size: 12px; color: var(--rams-muted);">Sequence-based cold outreach (Cialdini)</div>
                <button id="v1-toggle-btn" class="btn" style="margin-top: 12px; width: 100%;" data-sales-action="toggle-v1">Disable V1</button>
              </div>
              <div class="card">
                <div class="card-header">
                  <span class="card-title">V2 AUTONOMOUS</span>
                  <span id="v2-enabled-badge" class="pill pill-success">ENABLED</span>
                </div>
                <div style="font-size: 12px; color: var(--rams-muted);">Thompson Sampler + ML scoring</div>
                <button id="v2-toggle-btn" class="btn" style="margin-top: 12px; width: 100%;" data-sales-action="toggle-v2">Disable V2</button>
              </div>
            </div>

            <!-- Audit Log -->
            <div class="card" style="margin-top: 16px;">
              <div class="card-header">
                <span class="card-title">CONTROL AUDIT LOG</span>
                <button class="btn" data-sales-action="refresh-audit-log">Refresh</button>
              </div>
              <div id="control-audit-log" style="max-height: 200px; overflow-y: auto; font-size: 11px; font-family: monospace;">
                Loading audit log...
              </div>
            </div>
          </div>

          <!-- ==================== SUPPRESSION SUBVIEW ==================== -->
          <div id="sales-subview-suppression" class="sales-subview hidden">
            <!-- Suppression Summary -->
            <div class="grid">
              <div class="card">
                <div class="card-title">HARD BOUNCES</div>
                <div id="supp-hard-bounces" style="font-size: 24px; font-weight: bold; margin-top: 10px;">-</div>
                <div style="font-size: 11px; color: var(--rams-muted);">Permanent failures</div>
              </div>
              <div class="card">
                <div class="card-title">UNSUBSCRIBES</div>
                <div id="supp-unsubscribes" style="font-size: 24px; font-weight: bold; margin-top: 10px;">-</div>
                <div style="font-size: 11px; color: var(--rams-muted);">Opt-out requests</div>
              </div>
              <div class="card">
                <div class="card-title">SPAM COMPLAINTS</div>
                <div id="supp-complaints" style="font-size: 24px; font-weight: bold; margin-top: 10px;">-</div>
                <div style="font-size: 11px; color: var(--rams-muted);">Marked as spam</div>
              </div>
              <div class="card">
                <div class="card-title">MANUAL BLOCKS</div>
                <div id="supp-manual" style="font-size: 24px; font-weight: bold; margin-top: 10px;">-</div>
                <div style="font-size: 11px; color: var(--rams-muted);">Operator-added</div>
              </div>
            </div>

            <!-- Add to Suppression -->
            <div class="card" style="margin-top: 16px;">
              <div class="card-header">
                <span class="card-title">ADD TO SUPPRESSION LIST</span>
              </div>
              <div style="display: flex; gap: 8px; margin-top: 8px; flex-wrap: wrap;">
                <input type="email" id="supp-add-email" placeholder="email@domain.com" style="flex: 1; min-width: 200px; padding: 8px 12px; border: 1px solid var(--rams-line); border-radius: 2px; background: var(--rams-panel);" />
                <select id="supp-add-type" style="padding: 8px 12px; border: 1px solid var(--rams-line); border-radius: 2px; background: var(--rams-panel);">
                  <option value="manual_block">Manual Block</option>
                  <option value="competitor">Competitor</option>
                  <option value="gdpr_erasure">GDPR Erasure</option>
                </select>
                <input type="text" id="supp-add-reason" placeholder="Reason (required)" style="flex: 1; min-width: 150px; padding: 8px 12px; border: 1px solid var(--rams-line); border-radius: 2px; background: var(--rams-panel);" />
                <button class="btn btn-primary" data-sales-action="add-suppression">Add</button>
              </div>
            </div>

            <!-- Recent Suppressions -->
            <div class="card" style="margin-top: 16px;">
              <div class="card-header">
                <span class="card-title">RECENT SUPPRESSIONS</span>
                <button class="btn" data-sales-action="refresh-suppression">Refresh</button>
              </div>
              <div id="suppression-list-table" style="max-height: 400px; overflow-y: auto;">
                Loading suppression list...
              </div>
            </div>
          </div>

          <!-- ==================== V1 SEQUENCES SUBVIEW ==================== -->
          <div id="sales-subview-v1-sequences" class="sales-subview hidden">
            <!-- V1 System Status -->
            <div class="card" style="margin-bottom: 16px; border: 2px solid var(--rams-line);">
              <div class="card-header">
                <div style="display: flex; align-items: center; gap: 12px;">
                  <span class="card-title">V1 SYSTEM STATUS</span>
                  <span id="v1-system-badge" class="pill">-</span>
                </div>
                <button id="v1-toggle-main-btn" class="btn" data-sales-action="toggle-v1-system">Toggle</button>
              </div>
              <div style="font-size: 12px; color: var(--rams-muted); margin-top: 8px;">
                Cialdini sequence-based outbound. When disabled, emails will NOT be sent.
              </div>
            </div>

            <!-- V1 KPIs -->
            <div class="grid">
              <div class="card">
                <div class="card-title">Contacts</div>
                <div id="sales-contacts-count" style="font-size: 24px; font-weight: bold; margin-top: 10px;">-</div>
                <div id="sales-contacts-breakdown" style="font-size: 12px; color: var(--rams-muted); margin-top: 5px;"></div>
              </div>
              <div class="card">
                <div class="card-title">Active Enrollments</div>
                <div id="sales-enrollments-count" style="font-size: 24px; font-weight: bold; margin-top: 10px;">-</div>
                <div id="sales-enrollments-breakdown" style="font-size: 12px; color: var(--rams-muted); margin-top: 5px;"></div>
              </div>
              <div class="card">
                <div class="card-title">Emails Sent</div>
                <div id="sales-emails-count" style="font-size: 24px; font-weight: bold; margin-top: 10px;">-</div>
                <div id="sales-emails-rates" style="font-size: 12px; color: var(--rams-muted); margin-top: 5px;"></div>
              </div>
              <div class="card">
                <div class="card-title">Today's Limit</div>
                <div id="sales-limit" style="font-size: 24px; font-weight: bold; margin-top: 10px;">-</div>
                <div id="sales-limit-detail" style="font-size: 12px; color: var(--rams-muted); margin-top: 5px;"></div>
              </div>
            </div>

            <!-- V1 Actions -->
            <div class="card" style="margin: 16px 0;">
              <div class="card-header">
                <span class="card-title">Outreach Actions</span>
                <div>
                  <button class="btn" data-sales-action="run-outreach-dry">Dry Run</button>
                  <button class="btn btn-primary" data-sales-action="run-outreach">Send Pending</button>
                </div>
              </div>
              <div id="sales-action-result" style="font-size: 12px; color: var(--rams-muted);">Ready.</div>
            </div>

            <!-- Sequences -->
            <div class="card" style="margin-bottom: 16px;">
              <div class="card-header">
                <span class="card-title">Email Sequences</span>
                <button class="btn btn-primary" data-sales-action="new-sequence">+ New Sequence</button>
              </div>
              <div id="sales-sequences-table">Loading sequences...</div>
            </div>

            <!-- V1 Contacts -->
            <div class="card" style="margin-bottom: 16px;">
              <div class="card-header">
                <span class="card-title">Contacts</span>
                <div>
                  <select id="contact-status-filter" data-sales-action="filter-contacts" style="padding: 6px 10px; background: var(--rams-panel); border: 1px solid var(--rams-line); border-radius: 2px; margin-right: 8px;">
                    <option value="">All Status</option>
                    <option value="new">New</option>
                    <option value="contacted">Contacted</option>
                    <option value="replied">Replied</option>
                    <option value="meeting_scheduled">Meeting Scheduled</option>
                    <option value="converted">Converted</option>
                    <option value="opted_out">Opted Out</option>
                  </select>
                  <input type="text" id="contact-search" placeholder="Search..." data-sales-action="search-contacts" style="padding: 6px 10px; background: var(--rams-panel); border: 1px solid var(--rams-line); border-radius: 2px; margin-right: 8px; width: 120px;" />
                  <button class="btn btn-primary" data-sales-action="add-contact">+ Add</button>
                </div>
              </div>
              <div id="sales-contacts-table">Loading contacts...</div>
            </div>

            <!-- V1 Enrollments -->
            <div class="card" style="margin-bottom: 16px;">
              <div class="card-header">
                <span class="card-title">Enrollments</span>
                <button class="btn btn-primary" data-sales-action="enroll-contacts">Enroll Contacts</button>
              </div>
              <div id="sales-enrollments-table">Loading enrollments...</div>
            </div>

            <!-- V1 Recent Emails -->
            <div class="card">
              <div class="card-header"><span class="card-title">Recent Emails</span></div>
              <div id="sales-emails-table">Loading emails...</div>
            </div>
          </div>

          <!-- ==================== V2 DISCOVERY SUBVIEW ==================== -->
          <div id="sales-subview-v2-discovery" class="sales-subview hidden">
            <!-- Worker Status -->
            <div class="card" style="margin-bottom: 16px; border: 2px solid var(--rams-line);">
              <div class="card-header">
                <div style="display: flex; align-items: center; gap: 12px;">
                  <span class="card-title">WORKER STATUS (Test Server)</span>
                  <span id="intel-worker-badge" class="pill">-</span>
                </div>
                <button class="btn" data-sales-action="check-worker-health">Check Health</button>
              </div>
              <div id="intel-worker-info" style="font-size: 12px; color: var(--rams-muted); margin-top: 8px;">Click 'Check Health' to verify connection.</div>
            </div>

            <!-- Discovery Stats -->
            <div class="grid">
              <div class="card">
                <div class="card-title">Discovery Sources</div>
                <div id="intel-sources-count" style="font-size: 24px; font-weight: bold; margin-top: 10px;">-</div>
                <div id="intel-sources-detail" style="font-size: 12px; color: var(--rams-muted); margin-top: 5px;"></div>
              </div>
              <div class="card">
                <div class="card-title">Scored Contacts</div>
                <div id="intel-contacts-scored" style="font-size: 24px; font-weight: bold; margin-top: 10px;">-</div>
                <div id="intel-contacts-detail" style="font-size: 12px; color: var(--rams-muted); margin-top: 5px;"></div>
              </div>
              <div class="card">
                <div class="card-title">Avg Lead Score</div>
                <div id="intel-avg-score" style="font-size: 24px; font-weight: bold; margin-top: 10px;">-</div>
                <div id="intel-score-detail" style="font-size: 12px; color: var(--rams-muted); margin-top: 5px;"></div>
              </div>
              <div class="card">
                <div class="card-title">Thompson Sampler</div>
                <div id="intel-bandit-arms" style="font-size: 24px; font-weight: bold; margin-top: 10px;">-</div>
                <div id="intel-bandit-detail" style="font-size: 12px; color: var(--rams-muted); margin-top: 5px;"></div>
              </div>
            </div>

            <!-- Job Controls -->
            <div class="card" style="margin: 16px 0;">
              <div class="card-header">
                <span class="card-title">Job Controls</span>
                <div>
                  <button class="btn" data-sales-action="run-discovery">Run Discovery</button>
                  <button class="btn" data-sales-action="run-scoring">Run Scoring</button>
                  <button class="btn btn-primary" data-sales-action="refresh-intel">Refresh</button>
                </div>
              </div>
              <div id="intel-job-result" style="font-size: 12px; color: var(--rams-muted);">Ready to run jobs.</div>
            </div>

            <!-- Lead Sources -->
            <div class="card" style="margin-bottom: 16px;">
              <div class="card-header">
                <span class="card-title">Lead Sources</span>
                <button class="btn btn-primary" data-sales-action="add-source">+ Add Source</button>
              </div>
              <div id="intel-sources-table">Loading sources...</div>
            </div>

            <!-- Hot Leads -->
            <div class="card">
              <div class="card-header"><span class="card-title">Hot Leads (Score ≥ 70)</span></div>
              <div id="intel-hot-leads">Loading...</div>
            </div>
          </div>

          <!-- ==================== V2 MESSAGING SUBVIEW ==================== -->
          <div id="sales-subview-v2-messaging" class="sales-subview hidden">
            <!-- Compose Stats -->
            <div class="grid">
              <div class="card">
                <div class="card-title">Messages Composed</div>
                <div id="v2-messages-composed" style="font-size: 24px; font-weight: bold; margin-top: 10px;">-</div>
              </div>
              <div class="card">
                <div class="card-title">Messages Sent</div>
                <div id="v2-messages-sent" style="font-size: 24px; font-weight: bold; margin-top: 10px;">-</div>
              </div>
              <div class="card">
                <div class="card-title">Open Rate</div>
                <div id="v2-open-rate" style="font-size: 24px; font-weight: bold; margin-top: 10px;">-</div>
              </div>
              <div class="card">
                <div class="card-title">Reply Rate</div>
                <div id="v2-reply-rate" style="font-size: 24px; font-weight: bold; margin-top: 10px;">-</div>
              </div>
            </div>

            <!-- Compose Actions -->
            <div class="card" style="margin: 16px 0;">
              <div class="card-header">
                <span class="card-title">Compose Actions</span>
                <div>
                  <button class="btn" data-sales-action="compose-batch">Compose Batch</button>
                  <button class="btn btn-primary" data-sales-action="send-batch">Send Batch</button>
                </div>
              </div>
              <div id="v2-compose-result" style="font-size: 12px; color: var(--rams-muted);">Ready.</div>
            </div>

            <!-- Spintax Templates -->
            <div class="card" style="margin-bottom: 16px;">
              <div class="card-header">
                <span class="card-title">Spintax Templates</span>
                <button class="btn btn-primary" data-sales-action="new-template">+ New Template</button>
              </div>
              <div id="v2-templates-table">Loading templates...</div>
            </div>

            <!-- Pending Messages -->
            <div class="card">
              <div class="card-header"><span class="card-title">Pending Messages</span></div>
              <div id="v2-pending-messages">Loading...</div>
            </div>
          </div>

          <!-- ==================== INBOX SUBVIEW ==================== -->
          <div id="sales-subview-inbox" class="sales-subview hidden">
            <!-- Inbox Stats -->
            <div class="grid">
              <div class="card">
                <div class="card-title">Total Replies</div>
                <div id="inbox-total" style="font-size: 24px; font-weight: bold; margin-top: 10px;">-</div>
              </div>
              <div class="card">
                <div class="card-title">Needs Review</div>
                <div id="inbox-needs-review" style="font-size: 24px; font-weight: bold; margin-top: 10px;">-</div>
              </div>
              <div class="card">
                <div class="card-title">Interested</div>
                <div id="inbox-interested" style="font-size: 24px; font-weight: bold; margin-top: 10px; color: var(--rams-green);">-</div>
              </div>
              <div class="card">
                <div class="card-title">Not Interested</div>
                <div id="inbox-not-interested" style="font-size: 24px; font-weight: bold; margin-top: 10px; color: var(--rams-red);">-</div>
              </div>
            </div>

            <!-- Inbox Queue -->
            <div class="card" style="margin-top: 16px;">
              <div class="card-header">
                <span class="card-title">Reply Inbox</span>
                <button class="btn" data-sales-action="refresh-inbox">Refresh</button>
              </div>
              <div id="intel-inbox">Loading inbox...</div>
            </div>
          </div>

          <!-- ==================== METRICS SUBVIEW ==================== -->
          <div id="sales-subview-metrics" class="sales-subview hidden">
            <!-- Unified Metrics -->
            <div class="card" style="margin-bottom: 16px;">
              <div class="card-header">
                <span class="card-title">UNIFIED ENGAGEMENT METRICS (7 Days)</span>
                <button class="btn" data-sales-action="refresh-metrics">Refresh</button>
              </div>
              <div class="grid" style="margin-top: 12px;">
                <div style="text-align: center; padding: 16px; background: var(--rams-panel); border-radius: 2px;">
                  <div style="font-size: 10px; color: var(--rams-muted); text-transform: uppercase;">Sent</div>
                  <div id="metrics-sent" style="font-size: 28px; font-weight: bold;">-</div>
                </div>
                <div style="text-align: center; padding: 16px; background: var(--rams-panel); border-radius: 2px;">
                  <div style="font-size: 10px; color: var(--rams-muted); text-transform: uppercase;">Delivered</div>
                  <div id="metrics-delivered" style="font-size: 28px; font-weight: bold;">-</div>
                </div>
                <div style="text-align: center; padding: 16px; background: var(--rams-panel); border-radius: 2px;">
                  <div style="font-size: 10px; color: var(--rams-muted); text-transform: uppercase;">Opened</div>
                  <div id="metrics-opened" style="font-size: 28px; font-weight: bold; color: var(--rams-steel);">-</div>
                </div>
                <div style="text-align: center; padding: 16px; background: var(--rams-panel); border-radius: 2px;">
                  <div style="font-size: 10px; color: var(--rams-muted); text-transform: uppercase;">Clicked</div>
                  <div id="metrics-clicked" style="font-size: 28px; font-weight: bold; color: var(--rams-steel);">-</div>
                </div>
                <div style="text-align: center; padding: 16px; background: var(--rams-panel); border-radius: 2px;">
                  <div style="font-size: 10px; color: var(--rams-muted); text-transform: uppercase;">Replied</div>
                  <div id="metrics-replied" style="font-size: 28px; font-weight: bold; color: var(--rams-green);">-</div>
                </div>
                <div style="text-align: center; padding: 16px; background: var(--rams-panel); border-radius: 2px;">
                  <div style="font-size: 10px; color: var(--rams-muted); text-transform: uppercase;">Bounced</div>
                  <div id="metrics-bounced" style="font-size: 28px; font-weight: bold; color: var(--rams-red);">-</div>
                </div>
              </div>
            </div>

            <!-- By System Breakdown -->
            <div class="grid">
              <div class="card">
                <div class="card-title">V1 OUTBOUND (7d)</div>
                <div id="metrics-v1-summary" style="font-size: 12px; margin-top: 8px;">Loading...</div>
              </div>
              <div class="card">
                <div class="card-title">V2 AUTONOMOUS (7d)</div>
                <div id="metrics-v2-summary" style="font-size: 12px; margin-top: 8px;">Loading...</div>
              </div>
            </div>

            <!-- Domain Health -->
            <div class="card" style="margin-top: 16px;">
              <div class="card-header">
                <span class="card-title">DOMAIN REPUTATION</span>
              </div>
              <div id="metrics-domain-health" style="font-size: 12px; margin-top: 8px;">Loading domain health...</div>
            </div>
          </div>
        </div>
        <!-- END SALES HUB -->

        <!-- Legacy view aliases for backward compatibility -->
        <div id="view-sales" class="view hidden"></div>
        <div id="view-intelligence" class="view hidden"></div>

        <!-- Compliance View -->
        <div id="view-compliance" class="view hidden">
          <div class="card" style="margin-bottom: 24px;">
            <div class="card-header">
              <span class="card-title">Estonian Compliance Status</span>
              <div>
                <button class="btn btn-primary" onclick="generateAnnualReport()">Generate 2025 Annual Report</button>
                <button class="btn" onclick="downloadPdf('/api/v1/compliance/annual-report/2025/pdf', 'annual-report-2025.pdf')">Download PDF</button>
              </div>
            </div>
            <div id="compliance-status-content">Loading...</div>
          </div>
          
          <div class="card" style="margin-bottom: 24px;">
            <div class="card-header">
              <span class="card-title">VAT Reports</span>
              <button class="btn" onclick="downloadPdf('/api/v1/compliance/kmd-inf/2025/12/pdf', 'kmd-inf-2025-12.pdf')">Download KMD INF (Dec 2025)</button>
            </div>
            <div id="vat-reports-content">Loading...</div>
          </div>

          <div class="card" style="margin-bottom: 24px;">
            <div class="card-header">
              <span class="card-title">Tax Forecast (Next 3 Months)</span>
            </div>
            <div id="tax-forecast-content">Loading...</div>
          </div>

          <div class="card">
            <div class="card-header">
              <span class="card-title">TSD Reports (Payroll)</span>
              <button class="btn" onclick="downloadPdf('/api/v1/compliance/tsd-report/2025/pdf', 'tsd-report-2025.pdf')">Download 2025 Annual TSD</button>
            </div>
            <div id="tsd-reports-content">
              <p style="font-size: 13px; color: #94a3b8;">Detailed monthly TSD declarations can be generated via API or requested from accounting.</p>
            </div>
          </div>
        </div>

        <!-- Data Explorer View -->
        <div id="view-data-explorer" class="view hidden">
          <div class="card" style="margin-bottom: 24px;">
            <div class="card-header"><span class="card-title">Publishers / Users</span></div>
            <div id="users-table">Loading...</div>
          </div>
          <div class="card">
            <div class="card-header"><span class="card-title">Recent Invoices</span></div>
            <div id="invoices-table">Loading...</div>
          </div>
        </div>

        <!-- Analytics View -->
        <div id="view-analytics" class="view hidden">
          <div class="grid">
            <div class="card">
              <div class="card-title">Impressions (Today)</div>
              <div id="analytics-impressions" style="font-size: 24px; font-weight: bold; margin-top: 10px;">-</div>
              <div id="analytics-impressions-change" style="font-size: 12px; margin-top: 5px;"></div>
            </div>
            <div class="card">
              <div class="card-title">Revenue (Today)</div>
              <div id="analytics-revenue" style="font-size: 24px; font-weight: bold; margin-top: 10px;">-</div>
              <div id="analytics-revenue-change" style="font-size: 12px; margin-top: 5px;"></div>
            </div>
            <div class="card">
              <div class="card-title">Active Publishers</div>
              <div id="analytics-publishers" style="font-size: 24px; font-weight: bold; margin-top: 10px;">-</div>
            </div>
          </div>
          
          <div class="grid" style="margin-top: 20px;">
            <div class="card">
              <div class="card-title">Estimated Runway</div>
              <div id="analytics-runway" style="font-size: 24px; font-weight: bold; margin-top: 10px;">-</div>
              <div style="font-size: 12px; color: #94a3b8; margin-top: 5px;">Months remaining</div>
            </div>
            <div class="card">
              <div class="card-title">Net Cash Flow (30d)</div>
              <div id="analytics-cashflow" style="font-size: 24px; font-weight: bold; margin-top: 10px;">-</div>
              <div id="analytics-cashflow-details" style="font-size: 12px; color: #94a3b8; margin-top: 5px;">-</div>
            </div>
            <div class="card">
              <div class="card-title">Average LTV</div>
              <div id="analytics-ltv" style="font-size: 24px; font-weight: bold; margin-top: 10px;">-</div>
              <div id="analytics-churn" style="font-size: 12px; color: #94a3b8; margin-top: 5px;">-</div>
            </div>
          </div>

          <div class="card" style="margin-top: 20px;">
            <div class="card-header"><span class="card-title">Performance Insights</span></div>
            <div id="analytics-insights">Loading business data...</div>
          </div>
        </div>

        <!-- Advanced Analytics View -->
        <div id="view-adv-analytics" class="view hidden">
          <div class="card" style="margin-bottom: 24px;">
            <div class="card-header"><span class="card-title">Cohort Retention (6 Months)</span></div>
            <div id="cohorts-content">Loading...</div>
          </div>
          <div class="card">
            <div class="card-header"><span class="card-title">Revenue Forecasting</span></div>
            <div id="forecast-content">Loading...</div>
          </div>
        </div>

        <!-- Reconciliation View -->
        <div id="view-reconciliation" class="view hidden">
          <div class="card" style="margin-bottom: 24px;">
            <div class="card-header">
              <span class="card-title">Bank Statement Reconciliation</span>
              <button class="btn btn-primary" onclick="loadReconciliation()">Process New Statement</button>
            </div>
            <div id="reconciliation-content">Click 'Process New Statement' to match bank moves with ledger.</div>
          </div>
        </div>
        
        <!-- Other views placeholder -->
        <div id="view-telemetry" class="view hidden">
          <div class="grid">
            <div class="card">
              <div class="card-title">App Health</div>
              <div id="telemetry-app-health" style="font-size: 20px; font-weight: bold; margin-top: 10px;">-</div>
              <div id="telemetry-app-health-detail" style="font-size: 12px; color: #94a3b8; margin-top: 6px;">-</div>
            </div>
            <div class="card">
              <div class="card-title">DB Sessions</div>
              <div id="telemetry-db-sessions" style="font-size: 20px; font-weight: bold; margin-top: 10px;">-</div>
              <div id="telemetry-db-sessions-detail" style="font-size: 12px; color: #94a3b8; margin-top: 6px;">-</div>
            </div>
            <div class="card">
              <div class="card-title">Host Storage</div>
              <div id="telemetry-host-storage" style="font-size: 20px; font-weight: bold; margin-top: 10px;">-</div>
              <div id="telemetry-host-storage-detail" style="font-size: 12px; color: #94a3b8; margin-top: 6px;">-</div>
            </div>
          </div>
          <div class="card">
            <div class="card-header"><span class="card-title">Telemetry Insights</span></div>
            <div id="telemetry-insights" style="font-size: 13px; color: #94a3b8;">Loading...</div>
          </div>
          <div class="card">
            <div class="card-header"><span class="card-title">Raw Telemetry</span></div>
            <pre id="telemetry-content">Loading...</pre>
          </div>
        </div>
        <div id="view-partitions" class="view hidden"><div id="partitions-content">Loading...</div></div>

        <div class="scroll-spacer" aria-hidden="true"></div>
      </div>
    </div>
  </div>

    <script src="/static/admin-dashboard.js?v=${Date.now()}"></script>
  </body>
</html>`
  );
});

app.get('/admin/logout', (req: Request, res: Response) => {
  sendHtml(
    res,
    `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>Signing out</title>
    <style>
      :root { color-scheme: light; }
      body { margin: 0; font-family: ui-sans-serif, system-ui, -apple-system, Segoe UI, Roboto, Helvetica, Arial; background: #f2f2f2; color: #1a1a1a; }
      .wrap { min-height: 100vh; display: grid; place-items: center; }
      .card { padding: 24px; border: 1px solid #cccccc; background: #e6e6e6; }
    </style>
  </head>
  <body>
    <div class="wrap">
      <div class="card">Signing out…</div>
    </div>
    <script src="/static/admin-logout.js"></script>
  </body>
</html>`
  );
});

app.get('/admin/:view', (req: Request, res: Response) => {
  const view = String(req.params.view || '').toLowerCase();
  if (!view || view === 'login' || view === 'logout') {
    res.redirect('/admin');
    return;
  }
  res.redirect(`/admin#${encodeURIComponent(view)}`);
});

app.get('/static/admin-login.js', (req: Request, res: Response) => {
  res.setHeader('content-type', 'application/javascript; charset=utf-8');
  res.setHeader('cache-control', 'no-store, max-age=0, must-revalidate');
  res.setHeader('pragma', 'no-cache');
  res.setHeader('expires', '0');
  res.status(200).send(`
(function () {
  var form = document.getElementById('login-form');
  var errorEl = document.getElementById('error');
  var submit = document.getElementById('submit');
  if (!form || !errorEl || !submit) return;

  // CSRF token extraction from cookie
  function getCsrfToken() {
    var cookies = document.cookie.split(';');
    for (var i = 0; i < cookies.length; i++) {
      var cookie = cookies[i].trim();
      if (cookie.indexOf('csrf_token=') === 0) {
        return cookie.substring('csrf_token='.length);
      }
    }
    return '';
  }

  function setError(msg) {
    errorEl.textContent = msg;
    errorEl.style.display = msg ? 'block' : 'none';
  }

  function toBase64Url(bytes) {
    var bin = '';
    for (var i = 0; i < bytes.length; i++) bin += String.fromCharCode(bytes[i]);
    var b64 = btoa(bin);
    // Avoid regex escapes inside a server-side template string (\+ and \// can be mangled).
    // Use replaceAll for the base64url alphabet conversion.
    return b64.replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/g, '');
  }

  function getOrCreateDeviceId() {
    try {
      var existing = localStorage.getItem(DEVICE_KEY);
      if (existing && existing.length >= 16) return existing;
      var bytes = new Uint8Array(32);
      if (window.crypto && window.crypto.getRandomValues) {
        window.crypto.getRandomValues(bytes);
      } else {
        for (var i = 0; i < bytes.length; i++) bytes[i] = Math.floor(Math.random() * 256);
      }
      var id = toBase64Url(bytes);
      localStorage.setItem(DEVICE_KEY, id);
      return id;
    } catch (e) {
      // If storage is blocked, generate a per-session id.
      var bytes2 = new Uint8Array(16);
      if (window.crypto && window.crypto.getRandomValues) window.crypto.getRandomValues(bytes2);
      return toBase64Url(bytes2);
    }
  }

  form.addEventListener('submit', async function (e) {
    e.preventDefault();
    setError('');
    submit.disabled = true;
    submit.textContent = 'Signing in…';

    try {
      var username = document.getElementById('username').value;
      var password = document.getElementById('password').value;
      var deviceId = getOrCreateDeviceId();
      var deviceLabel = (navigator && navigator.userAgent) ? String(navigator.userAgent).slice(0, 120) : undefined;
      var csrfToken = getCsrfToken();

      var resp = await fetch('/api/v1/auth/login', {
        method: 'POST',
        headers: { 
          'content-type': 'application/json',
          'x-csrf-token': csrfToken
        },
        body: JSON.stringify({ username: username, password: password, deviceId: deviceId, deviceLabel: deviceLabel }),
      });

      var data = null;
      try { data = await resp.json(); } catch (_) {}

      if (!resp.ok || !data || !data.token) {
        var msg = (data && (data.error || data.message)) ? (data.error || data.message) : 'Authentication failed';
        if (data && data.error === 'DEVICE_CONFIRMATION_REQUIRED') {
          msg = data.message || 'This device requires approval. Please check contact@apexmediation.ee and retry.';
        }
        setError(msg);
        return;
      }

      localStorage.setItem('controlPlaneToken', data.token);
      window.location.replace('/admin');
    } catch (err) {
      setError('Network error');
    } finally {
      submit.disabled = false;
      submit.textContent = 'Sign in';
    }
  });
})();
`);
});

app.get('/static/admin-dashboard.js', (req: Request, res: Response) => {
  res.setHeader('content-type', 'application/javascript; charset=utf-8');
  res.setHeader('cache-control', 'no-store, max-age=0, must-revalidate');
  res.setHeader('pragma', 'no-cache');
  res.setHeader('expires', '0');
  res.status(200).send(`
// SECURITY: HTML escaping function to prevent XSS (P0 Fix #13)
function escapeHtml(str) {
  if (str === null || str === undefined) return '';
  return String(str)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}
(function () {
  // IMPROVEMENT 4: Error boundary - catch and display JS errors gracefully
  window.onerror = function(message, source, lineno, colno, error) {
    console.error('Dashboard error:', message, source, lineno, colno, error);
    var content = document.querySelector('.content');
    if (content) {
      content.innerHTML = '<div style="background: #fee; border: 1px solid #c00; padding: 16px; margin: 16px; border-radius: 4px;">' +
        '<h3 style="color: #c00; margin: 0 0 8px;">Dashboard Error</h3>' +
        '<p style="margin: 0 0 8px;">An error occurred while loading the dashboard:</p>' +
        '<pre style="background: #fff; padding: 8px; overflow: auto; font-size: 12px;">' + 
        (message || 'Unknown error') + '\\n' +
        (source ? 'Source: ' + source + ':' + lineno + ':' + colno : '') +
        '</pre>' +
        '<p style="margin: 8px 0 0; font-size: 12px; color: #666;">Try refreshing the page. If the problem persists, contact support.</p>' +
        '<button onclick="window.location.reload()" style="margin-top: 12px; padding: 8px 16px; cursor: pointer;">Reload Page</button>' +
        '</div>';
    }
    return true; // Prevent default error handling
  };
  
  // Also catch unhandled promise rejections
  window.addEventListener('unhandledrejection', function(event) {
    console.error('Unhandled promise rejection:', event.reason);
  });

  var currentView = 'dashboard';
  var initialView = (function() {
    var hash = (window.location.hash || '').replace('#', '');
    if (hash) return hash;
    return 'dashboard';
  })();
  
  // Navigation
  document.querySelectorAll('.nav-item').forEach(function(item) {
    item.addEventListener('click', function(e) {
      if (e && typeof e.preventDefault === 'function') e.preventDefault();
      var view = item.getAttribute('data-view');
      if (!view) return;
      switchView(view);
    });
  });

  function switchView(view, opts) {
    if (!document.getElementById('view-' + view)) {
      view = 'dashboard';
    }
    currentView = view;
    document.querySelectorAll('.view').forEach(v => v.classList.add('hidden'));
    document.getElementById('view-' + view).classList.remove('hidden');
    
    document.querySelectorAll('.nav-item').forEach(i => i.classList.remove('active'));
    document.querySelector('.nav-item[data-view="' + view + '"]').classList.add('active');
    
    document.getElementById('view-title').textContent = view.charAt(0).toUpperCase() + view.slice(1).replace('-', ' ');

    if (!opts || !opts.skipHash) {
      if (view === 'dashboard') {
        history.replaceState(null, '', '/admin');
        window.location.hash = '';
      } else {
        window.location.hash = view;
      }
    }
    
    loadViewData(view);
  }

  window.addEventListener('hashchange', function() {
    var hash = (window.location.hash || '').replace('#', '');
    switchView(hash || 'dashboard', { skipHash: true });
  });

  // CSRF token extraction from cookie
  function getCsrfToken() {
    var cookies = document.cookie.split(';');
    for (var i = 0; i < cookies.length; i++) {
      var cookie = cookies[i].trim();
      if (cookie.indexOf('__Host-csrf=') === 0) {
        return cookie.substring('__Host-csrf='.length);
      }
    }
    return '';
  }

  async function apiFetch(path, opts) {
    var token = localStorage.getItem('controlPlaneToken');
    if (!token) {
      window.location.replace('/admin/login');
      throw new Error('No token');
    }
    var headers = (opts && opts.headers) ? opts.headers : {};
    headers.Authorization = 'Bearer ' + token;
    headers['x-csrf-token'] = getCsrfToken();
    if (opts && opts.body && !headers['content-type']) {
      headers['content-type'] = 'application/json';
    }
    return fetch(path, Object.assign({}, opts || {}, { headers: headers }));
  }

  async function apiFetchJson(path, fallback, opts) {
    try {
      var resp = await apiFetch(path, opts);
      if (!resp.ok) throw new Error('HTTP ' + resp.status);
      return await resp.json();
    } catch (e) {
      console.error(e);
      return fallback;
    }
  }

  function formatBytes(bytes) {
    if (!bytes || bytes <= 0) return '0 B';
    var units = ['B','KB','MB','GB','TB'];
    var idx = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
    var val = bytes / Math.pow(1024, idx);
    return val.toFixed(val >= 10 ? 0 : 1) + ' ' + units[idx];
  }

  async function loadViewData(view) {
    switch(view) {
      case 'dashboard': loadDashboard(); break;
      case 'intelligence': loadIntelligence(); break;
      case 'sales': loadSales(); break;
      case 'sales-hub': loadSalesHub(); break;
      case 'compliance': loadCompliance(); break;
      case 'data-explorer': loadDataExplorer(); break;
      case 'analytics': loadAnalytics(); break;
      case 'adv-analytics': loadAdvAnalytics(); break;
      case 'reconciliation': loadReconciliation(true); break;
      case 'telemetry': loadTelemetry(); break;
      case 'partitions': loadPartitions(); break;
    }
  }

  // === SALES HUB CONSOLIDATED VIEW ===
  var activeSalesSubview = 'controls';

  function bindSalesHubHandlers() {
    var hub = document.getElementById('view-sales-hub');
    if (!hub || hub.dataset.bound === 'true') return;
    hub.dataset.bound = 'true';

    hub.addEventListener('click', function(event) {
      var target = event.target.closest('[data-sales-action]');
      if (!target) return;
      var action = target.getAttribute('data-sales-action');
      switch (action) {
        case 'show-subview':
          showSalesSubview(target.getAttribute('data-subview'));
          break;
        case 'toggle-kill-switch':
          toggleGlobalKillSwitch();
          break;
        case 'refresh-safe-mode':
          refreshSafeModeStatus();
          break;
        case 'toggle-v1':
          toggleV1System();
          break;
        case 'toggle-v2':
          toggleV2System();
          break;
        case 'refresh-audit-log':
          loadControlAuditLog();
          break;
        case 'add-suppression':
          addToSuppressionList();
          break;
        case 'refresh-suppression':
          loadSuppressionList();
          break;
        case 'toggle-v1-system':
          toggleSalesSystem();
          break;
        case 'run-outreach-dry':
          runOutreachDryRun();
          break;
        case 'run-outreach':
          runOutreach();
          break;
        case 'new-sequence':
          showSequenceModal();
          break;
        case 'add-contact':
          showAddContactModal();
          break;
        case 'enroll-contacts':
          showEnrollModal();
          break;
        case 'check-worker-health':
          checkWorkerHealth();
          break;
        case 'run-discovery':
          runDiscoveryJob();
          break;
        case 'run-scoring':
          runScoringJob();
          break;
        case 'refresh-intel':
          loadIntelligenceStats();
          break;
        case 'add-source':
          showAddSourceModal();
          break;
        case 'compose-batch':
          runV2ComposeBatch();
          break;
        case 'send-batch':
          runV2SendBatch();
          break;
        case 'new-template':
          showSpintaxModal();
          break;
        case 'refresh-inbox':
          loadInboxMessages();
          break;
        case 'refresh-metrics':
          loadUnifiedMetrics();
          break;
        case 'edit-template':
          editTemplate(target.getAttribute('data-template-id'));
          break;
        case 'review-reply':
          reviewReply(target.getAttribute('data-reply-id'));
          break;
        case 'delete-source':
          deleteSource(target.getAttribute('data-source-id'));
          break;
        case 'toggle-sequence': {
          var seqId = target.getAttribute('data-sequence-id');
          var isActive = target.getAttribute('data-sequence-active') === 'true';
          toggleSequence(seqId, !isActive);
          break;
        }
        case 'edit-sequence':
          editSequence(target.getAttribute('data-sequence-id'));
          break;
        case 'edit-contact':
          editContact(target.getAttribute('data-contact-id'));
          break;
        case 'delete-contact':
          deleteContact(target.getAttribute('data-contact-id'));
          break;
        case 'pause-enrollment':
          pauseEnrollment(target.getAttribute('data-enrollment-id'));
          break;
        case 'resume-enrollment':
          resumeEnrollment(target.getAttribute('data-enrollment-id'));
          break;
      }
    });

    hub.addEventListener('change', function(event) {
      var target = event.target;
      if (target && target.getAttribute('data-sales-action') === 'filter-contacts') {
        loadSalesContacts();
      }
    });

    hub.addEventListener('keyup', function(event) {
      var target = event.target;
      if (target && target.getAttribute('data-sales-action') === 'search-contacts') {
        debounceContactSearch();
      }
    });
  }
  
  function showSalesSubview(subview) {
    activeSalesSubview = subview;
    document.querySelectorAll('.sales-sub-nav').forEach(function(btn) {
      btn.classList.remove('active');
    });
    var activeBtn = document.querySelector('.sales-sub-nav[data-subview="' + subview + '"]');
    if (activeBtn) {
      activeBtn.classList.add('active');
    }
    document.querySelectorAll('.sales-subview').forEach(function(sv) {
      sv.classList.add('hidden');
    });
    var subviewEl = document.getElementById('sales-subview-' + subview);
    if (subviewEl) {
      subviewEl.classList.remove('hidden');
    }
    loadSalesSubviewData(subview);
  }
  
  function loadSalesSubviewData(subview) {
    switch(subview) {
      case 'controls': loadSalesControls(); break;
      case 'suppression': loadSuppressionList(); break;
      case 'v1-sequences': loadSales(); break;
      case 'v2-discovery': loadIntelligence(); break;
      case 'v2-messaging': loadV2Messaging(); break;
      case 'inbox': loadInboxMessages(); break;
      case 'metrics': loadUnifiedMetrics(); break;
    }
  }

  async function loadSalesHub() {
    bindSalesHubHandlers();
    loadSalesSubviewData(activeSalesSubview);
  }
  
  async function loadSalesControls() {
    // Load kill switch status from sales-controls API
    try {
      var status = await apiFetchJson('/api/sales-controls/status', { killSwitchActive: false });
      var badge = document.getElementById('kill-switch-badge');
      var btn = document.getElementById('kill-switch-btn');
      if (!status.killSwitchActive) {
        badge.textContent = 'SENDS ENABLED';
        badge.className = 'pill pill-success';
        btn.textContent = 'EMERGENCY STOP';
        btn.style.background = 'var(--rams-red)';
      } else {
        badge.textContent = 'SENDS BLOCKED';
        badge.className = 'pill pill-danger';
        btn.textContent = 'RESUME SENDS';
        btn.style.background = 'var(--rams-green)';
      }
      
      // Update V1/V2 badges (use explicit enable flags)
      var v1Badge = document.getElementById('v1-enabled-badge');
      var v2Badge = document.getElementById('v2-enabled-badge');
      if (typeof status.v1OutboundEnabled === 'boolean') {
        v1Badge.textContent = status.v1OutboundEnabled ? 'ENABLED' : 'DISABLED';
        v1Badge.className = 'pill ' + (status.v1OutboundEnabled ? 'pill-success' : 'pill-danger');
      }
      if (typeof status.v2AutonomousEnabled === 'boolean') {
        v2Badge.textContent = status.v2AutonomousEnabled ? 'ENABLED' : 'DISABLED';
        v2Badge.className = 'pill ' + (status.v2AutonomousEnabled ? 'pill-success' : 'pill-danger');
      }
    } catch(e) {
      console.error('Failed to load kill switch status', e);
    }

    // Load safe mode status
    refreshSafeModeStatus();
    
    // Load control audit log
    loadControlAuditLog();
  }

  async function toggleGlobalKillSwitch() {
    try {
      var status = await apiFetchJson('/api/sales-controls/status', { killSwitchActive: false });
      var newState = !status.killSwitchActive;
      var reason = newState ? prompt('Enter reason for activating kill switch:') : prompt('Enter reason for resuming sends:');
      if (!reason || reason.length < 5) {
        alert('Reason is required (min 5 characters)');
        return;
      }
      await apiFetch('/api/sales-controls/kill-switch', {
        method: 'POST',
        body: JSON.stringify({ active: newState, reason: reason })
      });
      loadSalesControls();
    } catch(e) {
      alert('Failed to toggle kill switch: ' + e.message);
    }
  }

  async function toggleV1System() {
    try {
      var status = await apiFetchJson('/api/sales-controls/status', { v1OutboundEnabled: true });
      var current = status.v1OutboundEnabled !== false;
      var newState = !current;
      var reason = newState
        ? prompt('Enter reason for enabling V1 outbound:')
        : 'Operator disabled V1 outbound via Control Plane';
      if (newState && (!reason || reason.length < 5)) {
        alert('Reason is required (min 5 characters)');
        return;
      }
      await apiFetch('/api/sales-controls/update', {
        method: 'POST',
        body: JSON.stringify({ key: 'v1_outbound_enabled', value: newState ? 'true' : 'false', reason: reason })
      });
      loadSalesControls();
    } catch (e) {
      alert('Failed to toggle V1: ' + e.message);
    }
  }

  async function toggleV2System() {
    try {
      var status = await apiFetchJson('/api/sales-controls/status', { v2AutonomousEnabled: true });
      var current = status.v2AutonomousEnabled !== false;
      var newState = !current;
      var reason = newState
        ? prompt('Enter reason for enabling V2 autonomous:')
        : 'Operator disabled V2 autonomous via Control Plane';
      if (newState && (!reason || reason.length < 5)) {
        alert('Reason is required (min 5 characters)');
        return;
      }
      await apiFetch('/api/sales-controls/update', {
        method: 'POST',
        body: JSON.stringify({ key: 'v2_autonomous_enabled', value: newState ? 'true' : 'false', reason: reason })
      });
      loadSalesControls();
    } catch (e) {
      alert('Failed to toggle V2: ' + e.message);
    }
  }
  
  async function refreshSafeModeStatus() {
    try {
      var safeMode = await apiFetchJson('/api/sales-controls/safe-mode', { state: 'normal', bounceRate: 0, complaintRate: 0, throttlePercent: 100, emailsSent7d: 0 });
      document.getElementById('safe-mode-badge').textContent = safeMode.state ? safeMode.state.toUpperCase() : 'NORMAL';
      document.getElementById('safe-mode-badge').className = 'pill ' + (safeMode.state === 'normal' ? 'pill-success' : safeMode.state === 'warning' ? 'pill-warning' : 'pill-danger');
      document.getElementById('safe-mode-bounce').textContent = ((safeMode.bounceRate || 0) * 100).toFixed(2) + '%';
      document.getElementById('safe-mode-complaint').textContent = ((safeMode.complaintRate || 0) * 100).toFixed(3) + '%';
      document.getElementById('safe-mode-throttle').textContent = Math.round(safeMode.throttlePercent || 100) + '%';
      document.getElementById('safe-mode-sent').textContent = safeMode.emailsSent7d || 0;
    } catch(e) {
      console.error('Failed to load safe mode', e);
    }
  }

  async function loadControlAuditLog() {
    try {
      var logs = await apiFetchJson('/api/sales-controls/control-audit-log?limit=20', []);
      if (!logs || logs.length === 0) {
        document.getElementById('control-audit-log').innerHTML = '<div style="color: var(--rams-muted);">No audit log entries yet.</div>';
        return;
      }
      var html = logs.map(function(log) {
        var time = new Date(log.created_at).toLocaleString();
        // SECURITY: Escape user-controlled content (P0 Fix #13)
        return '<div style="padding: 4px 0; border-bottom: 1px solid var(--rams-line);">' +
          '<span style="color: var(--rams-muted);">' + escapeHtml(time) + '</span> ' +
          '<span style="color: var(--rams-orange);">' + escapeHtml(log.action) + '</span>: ' +
          escapeHtml(log.details) + ' <span style="color: var(--rams-muted);">by ' + escapeHtml(log.actor || 'system') + '</span></div>';
      }).join('');
      document.getElementById('control-audit-log').innerHTML = html;
    } catch(e) {
      document.getElementById('control-audit-log').innerHTML = '<div style="color: var(--rams-red);">Failed to load audit log.</div>';
    }
  }
  
  async function loadSuppressionList() {
    try {
      var stats = await apiFetchJson('/api/sales-controls/suppression-stats', { hardBounces: 0, unsubscribes: 0, complaints: 0, manual: 0 });
      document.getElementById('supp-hard-bounces').textContent = stats.hardBounces || 0;
      document.getElementById('supp-unsubscribes').textContent = stats.unsubscribes || 0;
      document.getElementById('supp-complaints').textContent = stats.complaints || 0;
      document.getElementById('supp-manual').textContent = stats.manual || 0;
      
      var list = await apiFetchJson('/api/sales-controls/suppression-list?limit=50', []);
      if (!list || list.length === 0) {
        document.getElementById('suppression-list-table').innerHTML = '<div style="color: var(--rams-muted); padding: 12px;">No suppression entries found.</div>';
        return;
      }
      var html = '<table><thead><tr><th>Email</th><th>Type</th><th>Reason</th><th>Added</th></tr></thead><tbody>';
      list.forEach(function(entry) {
        var time = new Date(entry.created_at).toLocaleDateString();
        // SECURITY: Escape user-controlled content (P0 Fix #13)
        html += '<tr><td>' + escapeHtml(entry.email) + '</td><td><span class="pill">' + escapeHtml(entry.suppression_type) + '</span></td><td>' + escapeHtml(entry.reason || '-') + '</td><td>' + escapeHtml(time) + '</td></tr>';
      });
      html += '</tbody></table>';
      document.getElementById('suppression-list-table').innerHTML = html;
    } catch(e) {
      document.getElementById('suppression-list-table').innerHTML = '<div style="color: var(--rams-red);">Failed to load suppression list.</div>';
    }
  }

  async function addToSuppressionList() {
    var email = document.getElementById('supp-add-email').value.trim();
    var type = document.getElementById('supp-add-type').value;
    var reason = document.getElementById('supp-add-reason').value.trim();
    if (!email || !reason) {
      alert('Email and reason are required.');
      return;
    }
    try {
      await apiFetch('/api/sales-controls/suppression-list', {
        method: 'POST',
        body: JSON.stringify({ email: email, type: type, reason: reason })
      });
      document.getElementById('supp-add-email').value = '';
      document.getElementById('supp-add-reason').value = '';
      loadSuppressionList();
    } catch(e) {
      alert('Failed to add to suppression list: ' + e.message);
    }
  }
  
  async function loadV2Messaging() {
    // Load V2 messaging stats
    try {
      var statsResp = await apiFetchJson('/api/autonomous/stats', {});
      var stats = statsResp.stats || statsResp || {};
      var messaging = stats.messaging || stats;
      document.getElementById('v2-messages-composed').textContent = messaging.composed || 0;
      document.getElementById('v2-messages-sent').textContent = messaging.sent || 0;
      document.getElementById('v2-open-rate').textContent = ((messaging.openRate || 0) * 100).toFixed(1) + '%';
      document.getElementById('v2-reply-rate').textContent = ((messaging.replyRate || 0) * 100).toFixed(1) + '%';
    } catch(e) {
      console.error('Failed to load V2 messaging stats', e);
    }
    
    // Load templates
    try {
      var templatesResp = await apiFetchJson('/api/autonomous/templates?type=email', { templates: [] });
      var templates = templatesResp.templates || templatesResp || [];
      window.v2Templates = templates;
      if (!templates || templates.length === 0) {
        document.getElementById('v2-templates-table').innerHTML = '<div style="color: var(--rams-muted); padding: 12px;">No spintax templates configured.</div>';
      } else {
        var html = '<table><thead><tr><th>Name</th><th>Subject</th><th>Status</th><th>Actions</th></tr></thead><tbody>';
        templates.forEach(function(t) {
          // SECURITY: Escape user-controlled content (P0 Fix #13)
          html += '<tr><td>' + escapeHtml(t.name || '-') + '</td><td>' + escapeHtml(t.subject || t.subjectSpintax || '-') + '</td><td><span class="pill ' + (t.active ? 'pill-success' : 'pill-warning') + '">' + (t.active ? 'Active' : 'Paused') + '</span></td><td><button class="btn" data-sales-action="edit-template" data-template-id="' + escapeHtml(t.id) + '">Edit</button></td></tr>';
        });
        html += '</tbody></table>';
        document.getElementById('v2-templates-table').innerHTML = html;
      }
    } catch(e) {
      document.getElementById('v2-templates-table').innerHTML = '<div style="color: var(--rams-red);">Failed to load templates.</div>';
    }
    
    // Load pending messages
    try {
      var pendingResp = await apiFetchJson('/api/autonomous/pending-messages?limit=20', { messages: [] });
      var pending = pendingResp.messages || pendingResp || [];
      if (!pending || pending.length === 0) {
        document.getElementById('v2-pending-messages').innerHTML = '<div style="color: var(--rams-muted); padding: 12px;">No pending messages in queue.</div>';
      } else {
        var html = '<table><thead><tr><th>Recipient</th><th>Subject</th><th>Score</th><th>Queued</th></tr></thead><tbody>';
        pending.forEach(function(m) {
          var time = new Date(m.created_at || m.queued_at).toLocaleString();
          // SECURITY: Escape user-controlled content (P0 Fix #13)
          html += '<tr><td>' + escapeHtml(m.email || m.to_email || '-') + '</td><td>' + escapeHtml(m.subject || '-') + '</td><td>' + escapeHtml(m.lead_score || m.score || '-') + '</td><td>' + escapeHtml(time) + '</td></tr>';
        });
        html += '</tbody></table>';
        document.getElementById('v2-pending-messages').innerHTML = html;
      }
    } catch(e) {
      document.getElementById('v2-pending-messages').innerHTML = '<div style="color: var(--rams-red);">Failed to load pending messages.</div>';
    }
  }

  async function runV2ComposeBatch() {
    var resultEl = document.getElementById('v2-compose-result');
    try {
      // IMPROVEMENT 5: Ask user if they want dry-run mode
      var dryRun = confirm('Run in DRY-RUN mode?\\n\\nDry-run will preview what would be composed without actually creating messages.\\n\\nClick OK for dry-run, Cancel to compose for real.');
      
      var batchSize = parseInt(prompt('Compose batch size:', '25') || '25', 10);
      if (!batchSize || batchSize <= 0) {
        alert('Invalid batch size');
        return;
      }
      
      if (dryRun) {
        resultEl.innerHTML = '<span style="color: var(--rams-steel);">DRY-RUN: Analyzing leads...</span>';
      } else {
        resultEl.textContent = 'Selecting hot leads...';
      }
      
      var leadsResp = await apiFetchJson('/api/autonomous/hot-leads?minScore=50&limit=' + batchSize + '&status=all', { leads: [] });
      var leads = leadsResp.leads || [];
      if (!leads.length) {
        resultEl.innerHTML = '<span style="color: var(--rams-muted);">No hot leads available for composition.</span>';
        return;
      }
      var eligibleLeads = leads.filter(function(lead) {
        var hasEmail = lead.email && lead.email.trim();
        var hasApp = lead.app_name && lead.app_name.trim();
        return hasEmail && hasApp;
      });
      var skipped = leads.length - eligibleLeads.length;
      if (!eligibleLeads.length) {
        resultEl.innerHTML = '<span style="color: var(--rams-muted);">No hot leads with email + app data available for composition.</span>';
        return;
      }
      
      // DRY-RUN mode: Show what would happen without actually composing
      if (dryRun) {
        var gatingResults = { pass: 0, fail: 0, reasons: {} };
        resultEl.innerHTML = '<span style="color: var(--rams-steel);">DRY-RUN: Checking quality gates for ' + eligibleLeads.length + ' leads...</span>';
        
        for (var i = 0; i < Math.min(eligibleLeads.length, 10); i += 1) {
          try {
            var resp = await apiFetch('/api/autonomous/compose?showReasons=true', {
              method: 'POST',
              body: JSON.stringify({ contactId: eligibleLeads[i].id, showReasons: true })
            });
            var data = await resp.json();
            if (data.success) {
              gatingResults.pass += 1;
            } else if (data.gatingFailure) {
              gatingResults.fail += 1;
              (data.reasons || []).forEach(function(r) {
                gatingResults.reasons[r] = (gatingResults.reasons[r] || 0) + 1;
              });
            }
          } catch (e) {
            gatingResults.fail += 1;
          }
        }
        
        var reasonsList = Object.keys(gatingResults.reasons).map(function(r) {
          return '<li>' + r + ': ' + gatingResults.reasons[r] + '</li>';
        }).join('');
        
        resultEl.innerHTML = '<div style="padding: 8px; background: var(--rams-module); border: 1px solid var(--rams-steel);">' +
          '<strong style="color: var(--rams-steel);">DRY-RUN RESULTS</strong> (sampled ' + Math.min(eligibleLeads.length, 10) + ' of ' + eligibleLeads.length + ' leads)<br>' +
          '<span style="color: var(--rams-green);">Would pass: ' + gatingResults.pass + '</span> | ' +
          '<span style="color: var(--rams-red);">Would fail: ' + gatingResults.fail + '</span>' +
          (reasonsList ? '<br><strong>Failure reasons:</strong><ul style="margin: 4px 0; padding-left: 20px;">' + reasonsList + '</ul>' : '') +
          '<br><em>No messages were actually composed.</em></div>';
        return;
      }
      
      try {
        resultEl.textContent = 'Generating icebreakers...';
        await apiFetch('/api/autonomous/icebreaker/batch', {
          method: 'POST',
          body: JSON.stringify({ limit: eligibleLeads.length })
        });
      } catch (e) {
        console.warn('Icebreaker batch skipped', e);
      }
      var composed = 0;
      var failed = 0;
      for (var i = 0; i < eligibleLeads.length; i += 1) {
        try {
          var resp = await apiFetch('/api/autonomous/compose', {
            method: 'POST',
            body: JSON.stringify({ contactId: eligibleLeads[i].id })
          });
          if (resp.ok) {
            composed += 1;
          } else {
            failed += 1;
          }
        } catch (e) {
          failed += 1;
        }
      }
      resultEl.innerHTML = '<span style="color: var(--rams-green);">Composed ' + composed + ' / ' + eligibleLeads.length + ' messages.</span>' +
        (failed ? ' <span style="color: var(--rams-red);">Failed: ' + failed + '</span>' : '') +
        (skipped ? ' <span style="color: var(--rams-muted);">Skipped: ' + skipped + ' missing email/app data.</span>' : '');
      loadV2Messaging();
    } catch (e) {
      // SECURITY: Escape error message (P0 Fix #13)
      resultEl.innerHTML = '<span style="color: var(--rams-red);">Compose error: ' + escapeHtml(e.message) + '</span>';
    }
  }

  async function runV2SendBatch() {
    var resultEl = document.getElementById('v2-compose-result');
    try {
      var batchSize = parseInt(prompt('Send batch size:', '25') || '25', 10);
      if (!batchSize || batchSize <= 0) {
        alert('Invalid batch size');
        return;
      }
      resultEl.textContent = 'Sending batch...';
      var resp = await apiFetch('/api/autonomous/send-batch', {
        method: 'POST',
        body: JSON.stringify({ limit: batchSize })
      });
      var data = await resp.json();
      if (data.success) {
        resultEl.innerHTML = '<span style="color: var(--rams-green);">Sent ' + (data.sent || 0) + ' of ' + (data.total || batchSize) + ' messages.</span>' +
          (data.failed ? ' <span style="color: var(--rams-red);">Failed: ' + data.failed + '</span>' : '');
      } else {
        resultEl.innerHTML = '<span style="color: var(--rams-red);">Send failed: ' + (data.error || 'unknown') + '</span>';
      }
      loadV2Messaging();
    } catch (e) {
      resultEl.innerHTML = '<span style="color: var(--rams-red);">Send error: ' + e.message + '</span>';
    }
  }

  async function editTemplate(templateId) {
    try {
      var templates = window.v2Templates || [];
      var tmpl = templates.find(function(t) { return t.id === templateId; });
      if (!tmpl) {
        alert('Template not found.');
        return;
      }
      var subjectSpintax = tmpl.subjectSpintax || tmpl.subject_spintax || tmpl.subject || '';
      var bodySpintax = tmpl.bodySpintax || tmpl.body_spintax || tmpl.body || '';
      if (!subjectSpintax || !bodySpintax) {
        alert('Template preview unavailable (missing spintax).');
        return;
      }
      var preview = await apiFetchJson('/api/autonomous/templates/preview', { variations: [] }, {
        method: 'POST',
        body: JSON.stringify({
          subjectSpintax: subjectSpintax,
          bodySpintax: bodySpintax,
          context: {
            first_name: 'Taylor',
            company_name: 'Acme Corp',
            app_name: 'Acme App'
          },
          count: 3
        })
      });
      var variations = preview.variations || [];
      if (!variations.length) {
        alert('No preview variations returned.');
        return;
      }
      var lines = variations.map(function(v, idx) {
        return '--- Variation ' + (idx + 1) + ' ---\\n' + (v.subject || '') + '\\n\\n' + (v.body || '');
      }).join('\\n\\n');
      alert(lines);
    } catch (e) {
      alert('Template preview failed: ' + e.message);
    }
  }

  async function showSpintaxModal() {
    var name = prompt('Template name:', 'V2 Outreach v1');
    if (!name) return;
    var subject = prompt('Subject spintax:', 'Quick question about {{app_name}}');
    if (!subject) return;
    var body = prompt('Body spintax:', 'Hi {{first_name}},\\n\\nI noticed {{app_name}} and wanted to reach out...');
    if (!body) return;
    var description = prompt('Description (optional):', '');
    try {
      var resp = await apiFetch('/api/autonomous/templates', {
        method: 'POST',
        body: JSON.stringify({
          templateType: 'email',
          name: name,
          description: description || undefined,
          subjectSpintax: subject,
          bodySpintax: body
        })
      });
      var data = await resp.json();
      if (data.success) {
        alert('Template created.');
        loadV2Messaging();
      } else {
        alert('Template creation failed: ' + (data.error || 'unknown'));
      }
    } catch (e) {
      alert('Template creation failed: ' + e.message);
    }
  }

  async function reviewReply(replyId) {
    var sentiment = prompt('Set reply status (interested, not_interested, needs_review, spam, bounce, meeting, ooo):', 'interested');
    if (!sentiment) return;
    try {
      await apiFetch('/api/autonomous/inbox/' + replyId, {
        method: 'POST',
        body: JSON.stringify({ sentiment: sentiment, reviewed: true })
      });
      loadInboxMessages();
    } catch (e) {
      alert('Failed to update reply: ' + e.message);
    }
  }
  
  async function loadInboxMessages() {
    try {
      var inboxResp = await apiFetchJson('/api/autonomous/inbox?limit=20', { messages: [], stats: {} });
      var stats = inboxResp.stats || {};
      document.getElementById('inbox-total').textContent = stats.total || 0;
      document.getElementById('inbox-needs-review').textContent = stats.pendingReview || stats.needsReview || 0;
      document.getElementById('inbox-interested').textContent = stats.interested || 0;
      document.getElementById('inbox-not-interested').textContent = stats.notInterested || 0;

      var inbox = inboxResp.messages || [];
      if (!inbox || inbox.length === 0) {
        document.getElementById('intel-inbox').innerHTML = '<div style="color: var(--rams-muted); padding: 12px;">No replies in inbox yet.</div>';
      } else {
        var html = '<table><thead><tr><th>From</th><th>Subject</th><th>Status</th><th>Received</th><th>Actions</th></tr></thead><tbody>';
        inbox.forEach(function(r) {
          var time = new Date(r.received_at || r.created_at).toLocaleString();
          var status = r.sentiment || r.status || 'needs_review';
          var statusClass = status === 'interested' ? 'pill-success' : status === 'not_interested' ? 'pill-danger' : 'pill-warning';
          html += '<tr><td>' + (r.from_email || r.fromEmail || '-') + '</td><td>' + (r.subject || '-') + '</td><td><span class="pill ' + statusClass + '">' + status + '</span></td><td>' + time + '</td><td><button class="btn" data-sales-action="review-reply" data-reply-id="' + r.id + '">Review</button></td></tr>';
        });
        html += '</tbody></table>';
        document.getElementById('intel-inbox').innerHTML = html;
      }
    } catch(e) {
      document.getElementById('intel-inbox').innerHTML = '<div style="color: var(--rams-red);">Failed to load inbox.</div>';
    }
  }
  
  async function loadUnifiedMetrics() {
    try {
      var resp = await apiFetchJson('/api/sales-controls/unified-metrics', { totals: {}, dailyMetrics: [] });
      var totals = resp.totals || {};
      document.getElementById('metrics-sent').textContent = totals.total_sent || 0;
      document.getElementById('metrics-delivered').textContent = totals.total_delivered || 0;
      document.getElementById('metrics-opened').textContent = totals.total_opens || 0;
      document.getElementById('metrics-clicked').textContent = totals.total_clicks || 0;
      document.getElementById('metrics-replied').textContent = totals.total_replies || 0;
      document.getElementById('metrics-bounced').textContent = totals.total_bounces || 0;
    } catch(e) {
      console.error('Failed to load unified metrics', e);
    }
    
    // V1 summary
    try {
      var v1 = await apiFetchJson('/api/v1/sales/stats', {});
      document.getElementById('metrics-v1-summary').innerHTML = 
        '<div>Total Sent: ' + (v1.emailsSent || 0) + '</div>' +
        '<div>Open Rate: ' + ((v1.openRate || 0) * 100).toFixed(1) + '%</div>' +
        '<div>Reply Rate: ' + ((v1.replyRate || 0) * 100).toFixed(1) + '%</div>';
    } catch(e) {
      document.getElementById('metrics-v1-summary').innerHTML = '<div style="color: var(--rams-muted);">Unavailable</div>';
    }
    
    // V2 summary
    try {
      var v2 = await apiFetchJson('/api/autonomous/stats', {});
      document.getElementById('metrics-v2-summary').innerHTML = 
        '<div>Total Sent: ' + (v2.sent || 0) + '</div>' +
        '<div>Open Rate: ' + ((v2.openRate || 0) * 100).toFixed(1) + '%</div>' +
        '<div>Reply Rate: ' + ((v2.replyRate || 0) * 100).toFixed(1) + '%</div>';
    } catch(e) {
      document.getElementById('metrics-v2-summary').innerHTML = '<div style="color: var(--rams-muted);">Unavailable</div>';
    }
    
    // Domain health - show placeholder since endpoint may not exist yet
    document.getElementById('metrics-domain-health').innerHTML = 
      '<div style="display: grid; grid-template-columns: repeat(3, 1fr); gap: 16px;">' +
      '<div><div style="font-size: 10px; color: var(--rams-muted);">SPF</div><div style="font-weight: bold; color: var(--rams-muted);">-</div></div>' +
      '<div><div style="font-size: 10px; color: var(--rams-muted);">DKIM</div><div style="font-weight: bold; color: var(--rams-muted);">-</div></div>' +
      '<div><div style="font-size: 10px; color: var(--rams-muted);">DMARC</div><div style="font-weight: bold; color: var(--rams-muted);">-</div></div>' +
      '</div>';
  }
  // === END SALES HUB FUNCTIONS ===

  async function loadDashboard() {
    try {
      var data = await apiFetchJson('/api/v1/dashboard', null);
      if (!data) {
        document.getElementById('health-status').textContent = 'Unavailable';
        document.getElementById('db-sessions').textContent = '-';
        document.getElementById('storage-app').textContent = '-';
        document.getElementById('storage-db').textContent = '-';
        document.getElementById('storage-s3').textContent = '-';
        document.getElementById('dashboard-insights').textContent = 'Failed to load dashboard data.';
        document.getElementById('recent-events').textContent = 'Failed to load dashboard data.';
        return;
      }
      
      document.getElementById('health-status').textContent = data.health.overall;
      document.getElementById('health-status').style.color = 
        data.health.overall === 'OK' ? '#10b981' : (data.health.overall === 'DEGRADED' ? '#f59e0b' : '#ef4444');
      
      var backends = 0;
      if (data.database && data.database.length > 0) {
        backends = data.database.reduce((sum, db) => sum + (db.numbackends || 0), 0);
      }
      document.getElementById('db-sessions').textContent = backends;

      var appDisk = data.storage && data.storage.app && data.storage.app.length > 0 ? data.storage.app[0] : null;
      var dbDisk = data.storage && data.storage.db && data.storage.db.length > 0 ? data.storage.db[0] : null;
      var s3 = data.storage ? data.storage.s3 : null;

      if (appDisk) {
        document.getElementById('storage-app').textContent = Math.round(appDisk.use_percent) + '%';
        document.getElementById('storage-app-detail').textContent = formatBytes(appDisk.used_bytes) + ' / ' + formatBytes(appDisk.total_bytes);
      } else {
        document.getElementById('storage-app').textContent = 'Unavailable';
        document.getElementById('storage-app-detail').textContent = 'Configure HOST_TELEMETRY_ENDPOINT_APP';
      }

      if (dbDisk) {
        document.getElementById('storage-db').textContent = Math.round(dbDisk.use_percent) + '%';
        document.getElementById('storage-db-detail').textContent = formatBytes(dbDisk.used_bytes) + ' / ' + formatBytes(dbDisk.total_bytes);
      } else {
        document.getElementById('storage-db').textContent = 'Unavailable';
        document.getElementById('storage-db-detail').textContent = 'Configure HOST_TELEMETRY_ENDPOINT_DB';
      }

      if (s3) {
        document.getElementById('storage-s3').textContent = formatBytes(s3.total_bytes);
        var detail = s3.bucket ? ('Bucket: ' + s3.bucket) : 'S3 bucket';
        if (s3.object_count !== undefined) {
          detail += ' • Objects: ' + s3.object_count;
        }
        document.getElementById('storage-s3-detail').textContent = detail;
      } else {
        document.getElementById('storage-s3').textContent = 'Unavailable';
        document.getElementById('storage-s3-detail').textContent = 'Configure S3_STORAGE_ENDPOINT';
      }
      
      var eventsHtml = data.health.issues.length > 0 
        ? data.health.issues.join('\\n') 
        : 'No active issues detected.';
      document.getElementById('recent-events').textContent = eventsHtml;
      if (data.insights && data.insights.length > 0) {
        document.getElementById('dashboard-insights').innerHTML = '<ul style="margin:0; padding-left:18px;">' + data.insights.map(function(i){return '<li>' + i + '</li>';}).join('') + '</ul>';
      } else {
        document.getElementById('dashboard-insights').textContent = 'No action items detected.';
      }
    } catch (e) { console.error(e); }
  }

  // =============================================
  // INTELLIGENCE V2 (Autonomous Sales System)
  // =============================================

  async function loadIntelligence() {
    await Promise.all([
      loadIntelligenceStats(),
      loadIntelligenceSources(),
      loadHotLeads(),
      loadIntelligenceInbox()
    ]);
  }

  async function loadIntelligenceStats() {
    try {
      var stats = await apiFetchJson('/api/autonomous/stats', null);
      if (!stats || !stats.stats) {
        document.getElementById('intel-sources-count').textContent = '-';
        document.getElementById('intel-contacts-scored').textContent = '-';
        document.getElementById('intel-avg-score').textContent = '-';
        document.getElementById('intel-bandit-arms').textContent = '-';
        return;
      }
      var s = stats.stats;
      
      document.getElementById('intel-sources-count').textContent = s.discovery.sources || 0;
      document.getElementById('intel-sources-detail').textContent = s.discovery.lastScrape 
        ? 'Last scrape: ' + new Date(s.discovery.lastScrape).toLocaleString() 
        : 'No scrapes yet';
      
      document.getElementById('intel-contacts-scored').textContent = s.scoring.scored + ' / ' + s.scoring.contacts;
      document.getElementById('intel-contacts-detail').textContent = 'Total contacts: ' + s.scoring.contacts;
      
      document.getElementById('intel-avg-score').textContent = s.scoring.avgScore || 0;
      document.getElementById('intel-score-detail').textContent = s.scoring.avgScore >= 50 ? 'Good lead quality' : 'Needs more sources';
      
      document.getElementById('intel-bandit-arms').textContent = s.optimizer.arms + ' arms';
      document.getElementById('intel-bandit-detail').textContent = s.optimizer.trials + ' trials, ' + Math.round(s.optimizer.convergence * 100) + '% convergence';
    } catch (e) { console.error(e); }
  }

  window.checkWorkerHealth = async function() {
    var badge = document.getElementById('intel-worker-badge');
    var info = document.getElementById('intel-worker-info');
    badge.textContent = 'CHECKING...';
    badge.style.background = '#78716c';
    badge.style.color = '#e7e5e4';
    
    try {
      var resp = await apiFetch('/api/autonomous/health');
      var data = await resp.json();
      
      if (data.status === 'healthy') {
        badge.textContent = 'HEALTHY';
        badge.style.background = '#166534';
        badge.style.color = '#4ade80';
        info.textContent = 'Worker is connected and operational.';
      } else {
        badge.textContent = 'UNHEALTHY';
        badge.style.background = '#7f1d1d';
        badge.style.color = '#f87171';
        info.textContent = 'Worker returned unhealthy status.';
      }
    } catch (e) {
      badge.textContent = 'ERROR';
      badge.style.background = '#7f1d1d';
      badge.style.color = '#f87171';
      info.textContent = 'Could not reach worker: ' + e.message;
    }
  }

  window.runDiscoveryJob = async function() {
    document.getElementById('intel-job-result').innerHTML = 'Running discovery job...';
    try {
      var resp = await apiFetch('/api/autonomous/discover', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ batchSize: 5 })
      });
      var data = await resp.json();
      // Handle both async (new) and sync (legacy) response formats
      if (data.success && data.async && data.jobId) {
        // Async response - job is queued
        document.getElementById('intel-job-result').innerHTML = 
          '<span style="color:#10b981;">Discovery job queued (Job ID: ' + data.jobId.slice(0,8) + '...). Running in background.</span>';
      } else if (data.success && data.job) {
        // Legacy sync response
        document.getElementById('intel-job-result').innerHTML = 
          '<span style="color:#10b981;">Discovery complete: processed ' + data.job.processed + ' sources, ' + data.job.errors + ' errors</span>';
      } else {
        document.getElementById('intel-job-result').innerHTML = '<span style="color:#ef4444;">Discovery failed: ' + (data.error || 'unknown') + '</span>';
      }
      loadIntelligenceStats();
      loadIntelligenceSources();
    } catch (e) {
      document.getElementById('intel-job-result').innerHTML = '<span style="color:#ef4444;">Error: ' + e.message + '</span>';
    }
  }

  window.runScoringJob = async function() {
    document.getElementById('intel-job-result').innerHTML = 'Running scoring job...';
    try {
      var resp = await apiFetch('/api/autonomous/score', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ batchSize: 100 })
      });
      var data = await resp.json();
      // Handle both async (new) and sync (legacy) response formats
      if (data.success && data.async && data.jobId) {
        // Async response - job is queued
        document.getElementById('intel-job-result').innerHTML = 
          '<span style="color:#10b981;">Scoring job queued (Job ID: ' + data.jobId.slice(0,8) + '...). Running in background.</span>';
      } else if (data.success && data.job) {
        // Legacy sync response
        document.getElementById('intel-job-result').innerHTML = 
          '<span style="color:#10b981;">Scoring complete: processed ' + data.job.processed + ' contacts, ' + data.job.errors + ' errors</span>';
      } else {
        document.getElementById('intel-job-result').innerHTML = '<span style="color:#ef4444;">Scoring failed: ' + (data.error || 'unknown') + '</span>';
      }
      loadIntelligenceStats();
      loadHotLeads();
    } catch (e) {
      document.getElementById('intel-job-result').innerHTML = '<span style="color:#ef4444;">Error: ' + e.message + '</span>';
    }
  }

  async function loadIntelligenceSources() {
    try {
      var data = await apiFetchJson('/api/autonomous/sources', { sources: [] });
      var sources = data.sources || [];
      
      if (sources.length === 0) {
        document.getElementById('intel-sources-table').innerHTML = '<p style="color: #94a3b8;">No sources configured. Add a source to start lead discovery.</p>';
        return;
      }
      
      var html = '<table><thead><tr><th>Name</th><th>Type</th><th>Last Scraped</th><th>Status</th><th>Actions</th></tr></thead><tbody>';
      sources.forEach(function(s) {
        var lastScraped = s.last_scraped_at ? new Date(s.last_scraped_at).toLocaleString() : 'Never';
        html += '<tr>' +
          '<td>' + (s.source_name || s.sourceName || 'Unnamed') + '</td>' +
          '<td>' + (s.source_type || s.sourceType || '-') + '</td>' +
          '<td>' + lastScraped + '</td>' +
          '<td><span class="pill ' + (s.active !== false ? 'pill-success' : 'pill-warning') + '">' + (s.active !== false ? 'Active' : 'Paused') + '</span></td>' +
          '<td><button class="btn" data-sales-action="delete-source" data-source-id="' + s.id + '" style="padding:4px 8px;font-size:11px;color:#ef4444;">Delete</button></td>' +
        '</tr>';
      });
      html += '</tbody></table>';
      document.getElementById('intel-sources-table').innerHTML = html;
    } catch (e) { 
      console.error(e);
      document.getElementById('intel-sources-table').innerHTML = '<p style="color: #ef4444;">Failed to load sources.</p>';
    }
  }

  window.showAddSourceModal = async function() {
    var name = prompt('Source name (e.g., "Google Play Games"):');
    if (!name) return;
    
    var typeChoice = prompt('Source type:\\n1) google_play\\n2) itunes\\n\\nEnter number:', '1');
    var sourceType = typeChoice === '2' ? 'itunes' : 'google_play';
    
    var category = prompt('Category/Search term (e.g., "GAME" for Google Play, "game" for iTunes):', 'GAME');
    if (!category) return;
    
    try {
      var resp = await apiFetch('/api/autonomous/sources', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({
          sourceName: name,
          sourceType: sourceType,
          sourceConfig: sourceType === 'google_play' ? { category: category } : { searchTerm: category },
          scrapeIntervalHours: 24
        })
      });
      var data = await resp.json();
      if (data.success) {
        alert('Source added!');
        loadIntelligenceSources();
        loadIntelligenceStats();
      } else {
        alert('Error: ' + (data.error || 'Failed to add source'));
      }
    } catch (e) {
      alert('Error: ' + e.message);
    }
  }

  window.deleteSource = async function(id) {
    if (!confirm('Delete this source?')) return;
    try {
      await apiFetch('/api/autonomous/sources/' + id, { method: 'DELETE' });
      loadIntelligenceSources();
      loadIntelligenceStats();
    } catch (e) {
      alert('Error: ' + e.message);
    }
  }

  async function loadHotLeads() {
    try {
      // Use the dedicated hot-leads endpoint
      var leadsData = await apiFetchJson('/api/autonomous/hot-leads?minScore=50&limit=10&status=all', { leads: [] });
      var leads = leadsData.leads || [];
      
      if (leads.length === 0) {
        document.getElementById('intel-hot-leads').innerHTML = '<p style="color: #94a3b8;">No hot leads yet. Run scoring to analyze contacts.</p>';
        return;
      }
      
      var html = '<table><thead><tr><th>Company</th><th>Email</th><th>Score</th><th>Status</th></tr></thead><tbody>';
      leads.forEach(function(l) {
        var tierColor = l.score >= 70 ? '#10b981' : (l.score >= 50 ? '#f59e0b' : '#94a3b8');
        html += '<tr>' +
          '<td>' + (l.company_name || l.app_name || '-') + '</td>' +
          '<td>' + (l.email || '-') + '</td>' +
          '<td style="color:' + tierColor + ';font-weight:bold;">' + (l.score || 0) + '</td>' +
          '<td><span class="pill">' + (l.status || 'new') + '</span></td>' +
        '</tr>';
      });
      html += '</tbody></table>';
      document.getElementById('intel-hot-leads').innerHTML = html;
    } catch (e) { 
      console.error(e);
      document.getElementById('intel-hot-leads').innerHTML = '<p style="color: #94a3b8;">No leads data available.</p>';
    }
  }

  async function loadIntelligenceInbox() {
    try {
      var data = await apiFetchJson('/api/autonomous/inbox', { messages: [], pendingCount: 0 });
      var messages = data.messages || [];
      
      if (messages.length === 0) {
        document.getElementById('intel-inbox').innerHTML = '<p style="color: #94a3b8;">No replies pending review. ' + (data.pendingCount || 0) + ' total in queue.</p>';
        return;
      }
      
      var html = '<table><thead><tr><th>From</th><th>Subject</th><th>Classification</th><th>Received</th><th>Actions</th></tr></thead><tbody>';
      messages.forEach(function(m) {
        html += '<tr>' +
          '<td>' + (m.fromEmail || '-') + '</td>' +
          '<td style="max-width:200px;overflow:hidden;text-overflow:ellipsis;">' + (m.subject || '-') + '</td>' +
          '<td><span class="pill">' + (m.classification || 'unclassified') + '</span></td>' +
          '<td>' + (m.receivedAt ? new Date(m.receivedAt).toLocaleString() : '-') + '</td>' +
          '<td><button class="btn" data-sales-action="review-reply" data-reply-id="' + m.id + '" style="padding:4px 8px;font-size:11px;">Review</button></td>' +
        '</tr>';
      });
      html += '</tbody></table>';
      document.getElementById('intel-inbox').innerHTML = html;
    } catch (e) { 
      console.error(e);
      document.getElementById('intel-inbox').innerHTML = '<p style="color: #94a3b8;">Inbox unavailable.</p>';
    }
  }

  // =============================================
  // SALES AUTOMATION
  // =============================================
  var salesSequences = [];
  var contactSearchTimeout = null;

  async function loadSales() {
    await Promise.all([
      loadSalesStats(),
      loadSalesSequences(),
      window.loadSalesContacts(),
      loadSalesEnrollments(),
      loadSalesEmails()
    ]);
  }

  async function loadSalesStats() {
    try {
      var stats = await apiFetchJson('/api/v1/sales/stats', null);
      var status = await apiFetchJson('/api/v1/sales/status', null);

      if (!stats || !status) {
        document.getElementById('sales-contacts-count').textContent = '-';
        document.getElementById('sales-enrollments-count').textContent = '-';
        document.getElementById('sales-emails-count').textContent = '-';
        document.getElementById('sales-limit').textContent = '-';
        document.getElementById('sales-contacts-breakdown').textContent = 'Failed to load sales stats.';
        return;
      }
      
      // Update system enabled/disabled badge
      var badge = document.getElementById('v1-system-badge');
      var toggleBtn = document.getElementById('v1-toggle-main-btn');
      if (status.enabled) {
        badge.textContent = 'ENABLED';
        badge.style.background = '#166534';
        badge.style.color = '#4ade80';
        toggleBtn.textContent = 'Disable System';
        toggleBtn.className = 'btn';
      } else {
        badge.textContent = 'DISABLED';
        badge.style.background = '#7f1d1d';
        badge.style.color = '#f87171';
        toggleBtn.textContent = 'Enable System';
        toggleBtn.className = 'btn btn-primary';
      }
      
      document.getElementById('sales-contacts-count').textContent = stats.contacts.total;
      var contactBreakdown = Object.entries(stats.contacts.byStatus).map(([k,v]) => k + ': ' + v).join(', ');
      document.getElementById('sales-contacts-breakdown').textContent = contactBreakdown || 'No contacts yet';
      
      document.getElementById('sales-enrollments-count').textContent = stats.enrollments.byStatus.active || 0;
      var enrollBreakdown = Object.entries(stats.enrollments.byStatus).map(([k,v]) => k + ': ' + v).join(', ');
      document.getElementById('sales-enrollments-breakdown').textContent = enrollBreakdown || 'No enrollments yet';
      
      document.getElementById('sales-emails-count').textContent = stats.emails.sent;
      document.getElementById('sales-emails-rates').textContent = 
        'Open: ' + stats.emails.openRate + '% | Click: ' + stats.emails.clickRate + '% | Reply: ' + stats.emails.replyRate + '%';
      
      document.getElementById('sales-limit').textContent = status.todayRemaining + ' / ' + status.dailyLimit;
      document.getElementById('sales-limit-detail').textContent = status.todaySent + ' sent today';
    } catch (e) { console.error(e); }
  }

  async function toggleSalesSystem() {
    var badge = document.getElementById('v1-system-badge');
    var isCurrentlyEnabled = badge.textContent === 'ENABLED';
    var newState = !isCurrentlyEnabled;
    var toggleBtn = document.getElementById('v1-toggle-main-btn');
    
    if (newState) {
      if (!confirm('Are you sure you want to ENABLE the sales automation system? This will allow real emails to be sent.')) {
        return;
      }
    }
    
    try {
      if (toggleBtn) {
        toggleBtn.disabled = true;
        toggleBtn.textContent = 'Applying...';
      }
      if (newState) {
        badge.textContent = 'ENABLED';
        badge.style.background = '#166534';
        badge.style.color = '#4ade80';
      } else {
        badge.textContent = 'DISABLED';
        badge.style.background = '#7f1d1d';
        badge.style.color = '#f87171';
      }
      var resp = await apiFetch('/api/v1/sales/toggle', {
        method: 'POST',
        body: JSON.stringify({ enabled: newState })
      });
      var result = await resp.json();
      
      if (result.success) {
        loadSalesStats();
        alert('Sales automation ' + (result.enabled ? 'ENABLED' : 'DISABLED'));
      } else {
        alert('Failed to toggle system');
      }
    } catch (e) {
      console.error(e);
      alert('Error toggling system: ' + e.message);
    } finally {
      if (toggleBtn) {
        toggleBtn.disabled = false;
        toggleBtn.textContent = badge.textContent === 'ENABLED' ? 'Disable System' : 'Enable System';
        toggleBtn.className = badge.textContent === 'ENABLED' ? 'btn' : 'btn btn-primary';
      }
    }
  }
  window.toggleSalesSystem = toggleSalesSystem;

  async function loadSalesSequences() {
    try {
      var data = await apiFetchJson('/api/v1/sales/sequences', { sequences: [] });
      salesSequences = data.sequences || [];
      
      if (salesSequences.length === 0) {
        document.getElementById('sales-sequences-table').innerHTML = '<p style="color: #94a3b8;">No sequences created yet. Create one to get started.</p>';
        return;
      }
      
      var html = '<table><thead><tr><th>Name</th><th>Steps</th><th>Enrolled</th><th>Replied</th><th>Active</th><th>Status</th><th>Actions</th></tr></thead><tbody>';
      salesSequences.forEach(s => {
        var steps = typeof s.steps === 'string' ? JSON.parse(s.steps) : s.steps;
        html += '<tr>' +
          '<td>' + s.name + '</td>' +
          '<td>' + (steps ? steps.length : 0) + ' emails</td>' +
          '<td>' + (s.enrolled_count || 0) + '</td>' +
          '<td>' + (s.replied_count || 0) + '</td>' +
          '<td>' + (s.active_count || 0) + '</td>' +
          '<td><span class="pill ' + (s.active ? 'pill-success' : 'pill-warning') + '">' + (s.active ? 'Active' : 'Paused') + '</span></td>' +
          '<td>' +
            '<button class="btn" data-sales-action="toggle-sequence" data-sequence-id="' + s.id + '" data-sequence-active="' + (s.active ? 'true' : 'false') + '" style="padding:4px 8px;font-size:11px;">' + (s.active ? 'Pause' : 'Activate') + '</button> ' +
            '<button class="btn" data-sales-action="edit-sequence" data-sequence-id="' + s.id + '" style="padding:4px 8px;font-size:11px;">Edit</button>' +
          '</td>' +
        '</tr>';
      });
      html += '</tbody></table>';
      document.getElementById('sales-sequences-table').innerHTML = html;
    } catch (e) { console.error(e); }
  }

  window.loadSalesContacts = async function() {
    try {
      var status = document.getElementById('contact-status-filter').value;
      var search = document.getElementById('contact-search').value;
      var url = '/api/v1/sales/contacts?limit=50';
      if (status) url += '&status=' + encodeURIComponent(status);
      if (search) url += '&search=' + encodeURIComponent(search);
      
      var data = await apiFetchJson(url, { contacts: [], total: 0 });
      
      if (!data.contacts || data.contacts.length === 0) {
        document.getElementById('sales-contacts-table').innerHTML = '<p style="color: #94a3b8;">No contacts found.</p>';
        return;
      }
      
      var html = '<table><thead><tr><th>Email</th><th>Name</th><th>Company</th><th>App</th><th>Status</th><th>Score</th><th>Actions</th></tr></thead><tbody>';
      data.contacts.forEach(c => {
        html += '<tr>' +
          '<td>' + c.email + '</td>' +
          '<td>' + (c.first_name || '-') + '</td>' +
          '<td>' + (c.company_name || '-') + '</td>' +
          '<td>' + (c.app_name || '-') + '</td>' +
          '<td><span class="pill">' + c.status + '</span></td>' +
          '<td>' + (c.score || 0) + '</td>' +
          '<td>' +
            '<button class="btn" data-sales-action="edit-contact" data-contact-id="' + c.id + '" style="padding:4px 8px;font-size:11px;">Edit</button> ' +
            '<button class="btn" data-sales-action="delete-contact" data-contact-id="' + c.id + '" style="padding:4px 8px;font-size:11px;color:#ef4444;">Delete</button>' +
          '</td>' +
        '</tr>';
      });
      html += '</tbody></table>';
      if (data.total > data.contacts.length) {
        html += '<p style="color: #94a3b8; font-size: 12px; margin-top: 10px;">Showing ' + data.contacts.length + ' of ' + data.total + ' contacts</p>';
      }
      document.getElementById('sales-contacts-table').innerHTML = html;
    } catch (e) { console.error(e); }
  }

  window.debounceContactSearch = function() {
    if (contactSearchTimeout) clearTimeout(contactSearchTimeout);
    contactSearchTimeout = setTimeout(loadSalesContacts, 300);
  }

  async function loadSalesEnrollments() {
    try {
      var data = await apiFetchJson('/api/v1/sales/enrollments?limit=20', { enrollments: [], total: 0 });
      
      if (!data.enrollments || data.enrollments.length === 0) {
        document.getElementById('sales-enrollments-table').innerHTML = '<p style="color: #94a3b8;">No enrollments yet. Enroll contacts in a sequence to start outreach.</p>';
        return;
      }
      
      var html = '<table><thead><tr><th>Contact</th><th>Sequence</th><th>Step</th><th>Status</th><th>Next Send</th><th>Actions</th></tr></thead><tbody>';
      data.enrollments.forEach(e => {
        html += '<tr>' +
          '<td>' + e.email + '</td>' +
          '<td>' + e.sequence_name + '</td>' +
          '<td>Step ' + (e.current_step + 1) + '</td>' +
          '<td><span class="pill ' + (e.status === 'active' ? 'pill-success' : e.status === 'paused' ? 'pill-warning' : '') + '">' + e.status + '</span></td>' +
          '<td>' + (e.next_send_at ? new Date(e.next_send_at).toLocaleString() : '-') + '</td>' +
          '<td>' +
            (e.status === 'active' ? '<button class="btn" data-sales-action="pause-enrollment" data-enrollment-id="' + e.id + '" style="padding:4px 8px;font-size:11px;">Pause</button>' : '') +
            (e.status === 'paused' ? '<button class="btn" data-sales-action="resume-enrollment" data-enrollment-id="' + e.id + '" style="padding:4px 8px;font-size:11px;">Resume</button>' : '') +
          '</td>' +
        '</tr>';
      });
      html += '</tbody></table>';
      document.getElementById('sales-enrollments-table').innerHTML = html;
    } catch (e) { console.error(e); }
  }

  async function loadSalesEmails() {
    try {
      var data = await apiFetchJson('/api/v1/sales/emails?limit=20', { emails: [], total: 0 });
      
      if (!data.emails || data.emails.length === 0) {
        document.getElementById('sales-emails-table').innerHTML = '<p style="color: #94a3b8;">No emails sent yet.</p>';
        return;
      }
      
      var html = '<table><thead><tr><th>To</th><th>Subject</th><th>Status</th><th>Sent</th><th>Opened</th><th>Clicked</th></tr></thead><tbody>';
      data.emails.forEach(e => {
        html += '<tr>' +
          '<td>' + e.to_email + '</td>' +
          '<td style="max-width:200px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;">' + (e.subject || '-') + '</td>' +
          '<td><span class="pill ' + (e.status === 'sent' ? 'pill-success' : e.status === 'bounced' ? 'pill-danger' : '') + '">' + e.status + '</span></td>' +
          '<td>' + (e.sent_at ? new Date(e.sent_at).toLocaleString() : '-') + '</td>' +
          '<td>' + (e.opened_at ? new Date(e.opened_at).toLocaleString() : '-') + '</td>' +
          '<td>' + (e.clicked_at ? new Date(e.clicked_at).toLocaleString() : '-') + '</td>' +
        '</tr>';
      });
      html += '</tbody></table>';
      document.getElementById('sales-emails-table').innerHTML = html;
    } catch (e) { console.error(e); }
  }

  window.runOutreachDryRun = async function() {
    document.getElementById('sales-action-result').innerHTML = 'Running dry run...';
    try {
      var resp = await apiFetch('/api/v1/sales/run', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ dryRun: true })
      });
      var data = await resp.json();
      document.getElementById('sales-action-result').innerHTML = 
        '<span style="color:#10b981;">Dry run complete: Would send ' + data.sent + ' emails (processed ' + data.processed + ' enrollments)</span>';
    } catch (e) {
      document.getElementById('sales-action-result').innerHTML = '<span style="color:#ef4444;">Error: ' + e.message + '</span>';
    }
  }

  window.runOutreach = async function() {
    if (!confirm('Send pending outreach emails now?')) return;
    document.getElementById('sales-action-result').innerHTML = 'Sending emails...';
    try {
      var resp = await apiFetch('/api/v1/sales/run', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ dryRun: false })
      });
      var data = await resp.json();
      var errorMsg = data.errors && data.errors.length > 0 ? ' (Errors: ' + data.errors.join(', ') + ')' : '';
      document.getElementById('sales-action-result').innerHTML = 
        '<span style="color:#10b981;">Sent ' + data.sent + ' emails!' + errorMsg + '</span>';
      loadSalesStats();
      loadSalesEmails();
    } catch (e) {
      document.getElementById('sales-action-result').innerHTML = '<span style="color:#ef4444;">Error: ' + e.message + '</span>';
    }
  }

  window.showSequenceModal = function() {
    var modal = prompt('Enter sequence name:');
    if (!modal) return;
    
    var steps = [];
    var addMore = true;
    var stepNum = 1;
    while (addMore) {
      var delay = prompt('Step ' + stepNum + ' - Days delay (e.g., 0 for immediate, 3 for 3 days):', stepNum === 1 ? '0' : '3');
      if (delay === null) { addMore = false; break; }
      var subject = prompt('Step ' + stepNum + ' - Email subject:', 'Quick question about {{app_name}}');
      if (subject === null) { addMore = false; break; }
      var body = prompt('Step ' + stepNum + ' - Email body (use {{first_name}}, {{company_name}}, {{app_name}}):', 'Hi {{first_name}},\\n\\nI noticed {{app_name}} and wanted to reach out...');
      if (body === null) { addMore = false; break; }
      
      steps.push({ delay_days: parseInt(delay) || 0, subject: subject, body: body.replace(/\\\\n/g, '\\n') });
      stepNum++;
      addMore = confirm('Add another step?');
    }
    
    if (steps.length === 0) return;
    createSequence(modal, steps);
  }

  async function createSequence(name, steps) {
    try {
      var resp = await apiFetch('/api/v1/sales/sequences', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ name: name, steps: steps })
      });
      var data = await resp.json();
      if (data.success) {
        alert('Sequence created!');
        loadSalesSequences();
      } else {
        alert('Error: ' + data.error);
      }
    } catch (e) { alert('Error creating sequence'); }
  }

  window.toggleSequence = async function(id, active) {
    try {
      await apiFetch('/api/v1/sales/sequences/' + id, {
        method: 'PUT',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ active: active })
      });
      loadSalesSequences();
    } catch (e) { alert('Error updating sequence'); }
  }

  window.editSequence = function(id) {
    alert('Sequence editing modal coming soon. For now, create a new sequence.');
  }

  window.showAddContactModal = function() {
    var email = prompt('Contact email:');
    if (!email) return;
    var firstName = prompt('First name (optional):');
    var companyName = prompt('Company name (optional):');
    var appName = prompt('App name (optional):');
    
    addContact({ email: email, first_name: firstName, company_name: companyName, app_name: appName });
  }

  async function addContact(data) {
    try {
      var resp = await apiFetch('/api/v1/sales/contacts', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(data)
      });
      var result = await resp.json();
      if (result.success) {
        loadSalesContacts();
        loadSalesStats();
      } else {
        alert('Error: ' + result.error);
      }
    } catch (e) { alert('Error adding contact'); }
  }

  window.editContact = function(id) {
    var newStatus = prompt('Enter new status (new, contacted, replied, meeting_scheduled, converted, opted_out):');
    if (!newStatus) return;
    updateContact(id, { status: newStatus });
  }

  async function updateContact(id, data) {
    try {
      await apiFetch('/api/v1/sales/contacts/' + id, {
        method: 'PUT',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(data)
      });
      loadSalesContacts();
      loadSalesStats();
    } catch (e) { alert('Error updating contact'); }
  }

  window.deleteContact = async function(id) {
    if (!confirm('Delete this contact?')) return;
    try {
      await apiFetch('/api/v1/sales/contacts/' + id, { method: 'DELETE' });
      loadSalesContacts();
      loadSalesStats();
    } catch (e) { alert('Error deleting contact'); }
  }

  window.showEnrollModal = function() {
    if (salesSequences.length === 0) {
      alert('Create a sequence first!');
      return;
    }
    
    var seqOptions = salesSequences.map((s, i) => (i + 1) + ') ' + s.name).join('\\n');
    var seqChoice = prompt('Select sequence:\\n' + seqOptions + '\\n\\nEnter number:');
    if (!seqChoice) return;
    var seqIdx = parseInt(seqChoice) - 1;
    if (isNaN(seqIdx) || seqIdx < 0 || seqIdx >= salesSequences.length) {
      alert('Invalid selection');
      return;
    }
    
    var limit = prompt('How many contacts to enroll? (will pick from "new" status, highest score first):', '10');
    if (!limit) return;
    
    enrollContacts(salesSequences[seqIdx].id, parseInt(limit) || 10);
  }

  async function enrollContacts(sequenceId, limit) {
    try {
      var resp = await apiFetch('/api/v1/sales/enrollments/enroll', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ sequenceId: sequenceId, contactStatus: 'new', limit: limit })
      });
      var data = await resp.json();
      alert('Enrolled ' + data.enrolled + ' contacts!' + (data.errors.length > 0 ? ' Errors: ' + data.errors.join(', ') : ''));
      loadSalesEnrollments();
      loadSalesStats();
    } catch (e) { alert('Error enrolling contacts'); }
  }

  window.pauseEnrollment = async function(id) {
    try {
      await apiFetch('/api/v1/sales/enrollments/' + id + '/pause', { method: 'POST' });
      loadSalesEnrollments();
    } catch (e) { alert('Error pausing enrollment'); }
  }

  window.resumeEnrollment = async function(id) {
    try {
      await apiFetch('/api/v1/sales/enrollments/' + id + '/resume', { method: 'POST' });
      loadSalesEnrollments();
    } catch (e) { alert('Error resuming enrollment'); }
  }

  async function loadCompliance() {
    try {
      var status = await apiFetchJson('/api/v1/compliance/status', null);
      var vat = await apiFetchJson('/api/v1/compliance/vat-reports', []);
      var forecast = await apiFetchJson('/api/v1/compliance/forecast', { forecast: [] });

      if (!status) {
        document.getElementById('compliance-status-content').textContent = 'Failed to load compliance status.';
        document.getElementById('vat-reports-content').textContent = 'Failed to load VAT reports.';
        document.getElementById('tax-forecast-content').textContent = 'Failed to load tax forecast.';
        return;
      }
      
      var statusHtml = '<div style="display:grid; grid-template-columns: 1fr 1fr; gap: 20px;">' +
        '<div>' +
          '<div style="color: #94a3b8; font-size: 12px; margin-bottom: 4px;">Next VAT Deadline</div>' +
          '<div style="font-size: 18px; font-weight: bold;">' + (status.vatStatus.nextDeadline || 'None') + '</div>' +
        '</div>' +
        '<div>' +
          '<div style="color: #94a3b8; font-size: 12px; margin-bottom: 4px;">Pending VAT Reports</div>' +
          '<div style="font-size: 18px; font-weight: bold;">' + status.vatStatus.pendingReports + '</div>' +
        '</div>' +
      '</div>';
      document.getElementById('compliance-status-content').innerHTML = statusHtml;
      
      var vatHtml = '<table><thead><tr><th>Period</th><th>Sales</th><th>VAT Collected</th><th>VAT Paid</th><th>Status</th><th>Actions</th></tr></thead><tbody>';
      vat.forEach(r => {
        vatHtml += '<tr>' +
          '<td>' + r.year + ' Q' + r.quarter + '</td>' +
          '<td>€' + r.totalSalesEur.toLocaleString() + '</td>' +
          '<td>€' + r.vatCollectedEur.toLocaleString() + '</td>' +
          '<td>€' + r.vatPaidEur.toLocaleString() + '</td>' +
          '<td><span class="pill">' + r.status + '</span></td>' +
          '<td><button class="btn" onclick="downloadPdf(&apos;/api/v1/compliance/vat-reports/' + r.id + '/pdf&apos;, &apos;vat-report-' + r.year + '-Q' + r.quarter + '.pdf&apos;)" style="padding: 4px 8px; font-size: 11px;">PDF</button></td>' +
        '</tr>';
      });
      vatHtml += '</tbody></table>';
      document.getElementById('vat-reports-content').innerHTML = vatHtml;

      var fHtml = '<table><thead><tr><th>Period</th><th>Est. VAT</th><th>Est. TSD</th><th>Total liability</th></tr></thead><tbody>';
      forecast.forecast.forEach(f => {
        fHtml += '<tr><td>' + f.period + '</td><td>€' + f.estimatedVatEur.toLocaleString() + '</td><td>€' + f.estimatedTsdEur.toLocaleString() + '</td><td>€' + f.totalTaxLiabilityEur.toLocaleString() + '</td></tr>';
      });
      fHtml += '</tbody></table>';
      document.getElementById('tax-forecast-content').innerHTML = fHtml;
    } catch (e) { console.error(e); }
  }

  async function loadDataExplorer() {
    try {
      var users = await apiFetchJson('/api/v1/data/users', { users: [] });
      var invoices = await apiFetchJson('/api/v1/data/invoices', { invoices: [] });
      
      var uHtml = '<table><thead><tr><th>Email</th><th>Company</th><th>Status</th><th>Actions</th></tr></thead><tbody>';
      users.users.forEach(u => {
        uHtml += '<tr><td>' + u.email + '</td><td>' + (u.company_name || '-') + '</td><td>' + u.status + '</td>' +
          '<td><button class="btn" onclick="impersonateUser(&apos;' + u.id + '&apos;)" style="padding:4px 8px; font-size:11px;">Impersonate</button></td></tr>';
      });
      uHtml += '</tbody></table>';
      document.getElementById('users-table').innerHTML = uHtml;

      var iHtml = '<table><thead><tr><th>Invoice #</th><th>Customer</th><th>Total</th><th>Status</th></tr></thead><tbody>';
      invoices.invoices.forEach(i => {
        iHtml += '<tr><td>' + i.invoice_number + '</td><td>' + i.customer_email + '</td><td>€' + (i.total_cents/100).toLocaleString() + '</td><td>' + i.status + '</td></tr>';
      });
      iHtml += '</tbody></table>';
      document.getElementById('invoices-table').innerHTML = iHtml;
    } catch (e) { console.error(e); }
  }

  async function loadTelemetry() {
    try {
      var app = await apiFetchJson('/api/v1/telemetry/app', null);
      var db = await apiFetchJson('/api/v1/telemetry/database', null);
      var hosts = await apiFetchJson('/api/v1/telemetry/hosts', null);
      var slow = await apiFetchJson('/api/v1/telemetry/app/slow?threshold=1500', { endpoints: [] });

      if (!app || !db || !hosts) {
        document.getElementById('telemetry-content').textContent = 'Failed to load telemetry data.';
        document.getElementById('telemetry-insights').textContent = 'Telemetry data unavailable.';
        return;
      }

      var appHealth = app.telemetry && app.telemetry.health ? app.telemetry.health : null;
      var appHealthText = appHealth ? appHealth.status : 'UNKNOWN';
      document.getElementById('telemetry-app-health').textContent = appHealthText;
      document.getElementById('telemetry-app-health').style.color = appHealthText === 'OK' ? '#10b981' : (appHealthText === 'DEGRADED' ? '#f59e0b' : '#ef4444');
      document.getElementById('telemetry-app-health-detail').textContent = appHealth ? ('Uptime: ' + Math.round(appHealth.uptime_sec / 3600) + 'h • Version: ' + appHealth.version) : 'No health data.';

      var activeSessions = (db.activeSessions || []).length;
      var blockingSessions = (db.blockingSessions || []).length;
      document.getElementById('telemetry-db-sessions').textContent = activeSessions;
      document.getElementById('telemetry-db-sessions-detail').textContent = blockingSessions > 0 ? (blockingSessions + ' blocking sessions') : 'No blocking sessions';

      var appDisk = hosts.hosts && hosts.hosts.app && hosts.hosts.app.disk && hosts.hosts.app.disk[0] ? hosts.hosts.app.disk[0] : null;
      var dbDisk = hosts.hosts && hosts.hosts.db && hosts.hosts.db.disk && hosts.hosts.db.disk[0] ? hosts.hosts.db.disk[0] : null;
      if (appDisk || dbDisk) {
        var text = '';
        if (appDisk) text += 'App: ' + Math.round(appDisk.use_percent) + '%';
        if (dbDisk) text += (text ? ' • ' : '') + 'DB: ' + Math.round(dbDisk.use_percent) + '%';
        document.getElementById('telemetry-host-storage').textContent = text;
        document.getElementById('telemetry-host-storage-detail').textContent = (appDisk ? ('App: ' + formatBytes(appDisk.used_bytes) + ' / ' + formatBytes(appDisk.total_bytes)) : '') + (dbDisk ? (' • DB: ' + formatBytes(dbDisk.used_bytes) + ' / ' + formatBytes(dbDisk.total_bytes)) : '');
      } else {
        document.getElementById('telemetry-host-storage').textContent = 'Unavailable';
        document.getElementById('telemetry-host-storage-detail').textContent = 'Configure HOST_TELEMETRY_ENDPOINT_APP/DB';
      }

      var insights = [];
      if (appHealth && appHealth.status !== 'OK') insights.push('App health is ' + appHealth.status + ': inspect backend health + metrics endpoint.');
      if (blockingSessions > 0) insights.push('Blocking DB sessions detected: review pg_stat_activity and locks.');
      if (slow && slow.endpoints && slow.endpoints.length > 0) {
        insights.push('Slow endpoints detected (>1500ms p99): review latency hotspots.');
      }
      if (!hosts.hosts || (!hosts.hosts.app && !hosts.hosts.db)) insights.push('Host telemetry missing: configure node-exporter endpoints.');

      document.getElementById('telemetry-insights').innerHTML = insights.length
        ? '<ul style="margin:0; padding-left:18px;">' + insights.map(function(i){return '<li>' + i + '</li>';}).join('') + '</ul>'
        : 'No telemetry action items detected.';

      document.getElementById('telemetry-content').textContent = JSON.stringify({
        app: app,
        database: db,
        hosts: hosts,
        slow: slow
      }, null, 2);
    } catch (e) { console.error(e); }
  }

  async function loadAnalytics() {
    try {
      var data = await apiFetchJson('/api/v1/analytics/overview', null);
      if (!data) {
        document.getElementById('analytics-impressions').textContent = '-';
        document.getElementById('analytics-revenue').textContent = '-';
        document.getElementById('analytics-publishers').textContent = '-';
        document.getElementById('analytics-insights').textContent = 'Failed to load analytics.';
        return;
      }
      
      document.getElementById('analytics-impressions').textContent = data.impressions.today.toLocaleString();
      var impChange = document.getElementById('analytics-impressions-change');
      impChange.textContent = (data.impressions.change_pct >= 0 ? '+' : '') + data.impressions.change_pct + '% vs yesterday';
      impChange.style.color = data.impressions.change_pct >= 0 ? '#10b981' : '#ef4444';
      
      document.getElementById('analytics-revenue').textContent = '$' + data.revenue_usd.today.toLocaleString();
      var revChange = document.getElementById('analytics-revenue-change');
      revChange.textContent = (data.revenue_usd.change_pct >= 0 ? '+' : '') + data.revenue_usd.change_pct + '% vs yesterday';
      revChange.style.color = data.revenue_usd.change_pct >= 0 ? '#10b981' : '#ef4444';
      
      document.getElementById('analytics-publishers').textContent = data.active_publishers;

      // CEO Metrics
      document.getElementById('analytics-runway').textContent = data.financials.runway_months.toFixed(1);
      document.getElementById('analytics-cashflow').textContent = '€' + data.financials.net_cash_flow_eur.toLocaleString();
      document.getElementById('analytics-cashflow-details').textContent = 
        'In: €' + data.financials.monthly_inflow_eur.toLocaleString() + ' / Out: €' + data.financials.monthly_outflow_eur.toLocaleString();
      
      document.getElementById('analytics-ltv').textContent = '€' + data.ceo_metrics.ltv_eur.toLocaleString();
      document.getElementById('analytics-churn').textContent = 'Churn Rate: ' + data.ceo_metrics.churn_rate_pct + '% (30d)';
      
      var insights = [];
      insights.push('Active Experiments: ' + data.active_experiments + ' (review winners weekly).');
      if (data.revenue_usd.change_pct <= -10) {
        insights.push('Revenue down ' + Math.abs(data.revenue_usd.change_pct) + '%: check fill rate + top publisher performance.');
      } else if (data.revenue_usd.change_pct >= 10) {
        insights.push('Revenue up ' + data.revenue_usd.change_pct + '%: consider raising daily budgets or expanding traffic sources.');
      } else {
        insights.push('Revenue change within normal band (' + data.revenue_usd.change_pct + '%).');
      }

      if (data.impressions.change_pct <= -10) {
        insights.push('Impressions down ' + Math.abs(data.impressions.change_pct) + '%: verify SDK integrations and bid response rates.');
      } else if (data.impressions.change_pct >= 10) {
        insights.push('Impressions up ' + data.impressions.change_pct + '%: check infra saturation + inventory pacing.');
      }

      if (data.ceo_metrics.churn_rate_pct >= 5) {
        insights.push('Churn at ' + data.ceo_metrics.churn_rate_pct + '%: trigger retention outreach and account reviews.');
      }

      insights.push('Projected runway: ' + data.financials.runway_months + ' months based on current burn.');
      insights.push('Customer LTV: €' + data.ceo_metrics.ltv_eur.toLocaleString() + ' (target upsell on top cohorts).');
      document.getElementById('analytics-insights').innerHTML = '<ul style="margin: 0; padding-left: 20px;">' + 
        insights.map(i => '<li>' + i + '</li>').join('') + '</ul>';
        
    } catch (e) { console.error(e); }
  }

  window.downloadPdf = async function(url, filename) {
    try {
      var resp = await apiFetch(url);
      if (!resp.ok) throw new Error('PDF generation failed');
      var blob = await resp.blob();
      var link = document.createElement('a');
      link.href = window.URL.createObjectURL(blob);
      link.download = filename;
      link.click();
    } catch (e) { alert('Failed to download PDF: ' + e.message); }
  }

  async function loadPartitions() {
    try {
      var data = await apiFetchJson('/api/v1/partitions', { statuses: [], health: { healthy: true, issues: [] } });
      
      var rows = [];
      (data.statuses || []).forEach(function(status) {
        var tableLabel = (status.schema ? status.schema + '.' : '') + status.table_name;
        (status.partitions || []).forEach(function(p) {
          rows.push({
            table: tableLabel,
            partition: p.partition_name,
            range: p.range_start + ' to ' + p.range_end,
            size: (p.size_bytes || 0)
          });
        });
      });

      if (rows.length === 0) {
        document.getElementById('partitions-content').innerHTML = '<p style="color: #94a3b8;">No partitions found.</p>';
        return;
      }

      var insights = [];
      (data.statuses || []).forEach(function(status) {
        if (status.months_ahead !== undefined && status.months_ahead < 1) {
          insights.push((status.schema ? status.schema + '.' : '') + status.table_name + ': create future partitions (months ahead=' + status.months_ahead + ')');
        }
        if (status.total_size_gb !== undefined && status.total_size_gb > 20) {
          insights.push((status.schema ? status.schema + '.' : '') + status.table_name + ': total size ' + status.total_size_gb + 'GB — review retention or archive policy.');
        }
      });
      if (data.health && !data.health.healthy) {
        (data.health.issues || []).forEach(function(issue) {
          insights.push((issue.table || 'unknown') + ': ' + issue.issue);
        });
      }

      var html = '<table><thead><tr><th>Table</th><th>Partition</th><th>Range</th><th>Size</th></tr></thead><tbody>';
      rows.forEach(function(row) {
        var sizeMb = row.size ? (row.size / (1024 * 1024)).toFixed(2) + ' MB' : '-';
        html += '<tr>' +
          '<td>' + row.table + '</td>' +
          '<td>' + row.partition + '</td>' +
          '<td>' + row.range + '</td>' +
          '<td>' + sizeMb + '</td>' +
        '</tr>';
      });
      html += '</tbody></table>';
      
      var insightsHtml = insights.length > 0
        ? '<div class="card" style="margin-bottom: 16px;"><div class="card-header"><span class="card-title">Partition Action Summary</span></div><div style="font-size: 13px; color: #94a3b8;"><ul style="margin:0; padding-left:18px;">' + insights.map(function(i){return '<li>' + i + '</li>';}).join('') + '</ul></div></div>'
        : '<div class="card" style="margin-bottom: 16px;"><div class="card-header"><span class="card-title">Partition Action Summary</span></div><div style="font-size: 13px; color: #94a3b8;">No partition action items detected.</div></div>';

      if (data.health && !data.health.healthy) {
        var issueText = (data.health.issues || []).map(function(i) {
          return i.table + ': ' + i.issue;
        }).join(' | ');
        html = '<div class="pill pill-danger" style="margin-bottom: 15px;">Partition issues detected: ' + issueText + '</div>' + html;
      }
      
      document.getElementById('partitions-content').innerHTML = insightsHtml + html;
    } catch (e) { console.error(e); }
  }

  window.generateAnnualReport = async function() {
    if (!confirm('Generate 2025 Annual Report? This will be archived in the ledger.')) return;
    try {
      var data = await apiFetchJson('/api/v1/compliance/annual-report/2025', null);
      if (!data) throw new Error('Report generation failed');
      alert('Report generated and archived! Net Profit: €' + data.financials.profitEur.toLocaleString());
      loadCompliance();
    } catch (e) { alert('Failed to generate report'); }
  }

  async function loadAdvAnalytics() {
    try {
      var cohorts = await apiFetchJson('/api/v1/analytics/cohorts', { cohorts: [] });
      var forecast = await apiFetchJson('/api/v1/analytics/forecast', { forecast: [] });

      var cHtml = '<table><thead><tr><th>Cohort</th><th>Size</th><th>M0</th><th>M1</th><th>M2</th><th>M3</th><th>M4</th><th>M5</th></tr></thead><tbody>';
      cohorts.cohorts.forEach(c => {
        cHtml += '<tr><td>' + c.cohort_month + '</td><td>' + c.size + '</td>';
        for(var i=0; i<6; i++) {
          var val = c.retention[i] !== undefined ? c.retention[i] + '%' : '-';
          cHtml += '<td>' + val + '</td>';
        }
        cHtml += '</tr>';
      });
      cHtml += '</tbody></table>';
      document.getElementById('cohorts-content').innerHTML = cHtml;

      var fHtml = '<table><thead><tr><th>Month</th><th>Actual</th><th>Projected</th><th>Range</th></tr></thead><tbody>';
      forecast.forecast.forEach(f => {
        fHtml += '<tr><td>' + f.month + '</td><td>' + (f.actual ? '€' + f.actual.toLocaleString() : '-') + '</td><td>€' + f.projected.toLocaleString() + '</td><td>€' + f.lower_bound.toLocaleString() + ' - €' + f.upper_bound.toLocaleString() + '</td></tr>';
      });
      fHtml += '</tbody></table>';
      document.getElementById('forecast-content').innerHTML = fHtml;
    } catch (e) { console.error(e); }
  }

  window.loadReconciliation = async function(initial) {
    if (!initial) {
      document.getElementById('reconciliation-content').innerHTML = 'Processing bank statement...';
    }
    try {
      var data = await apiFetchJson('/api/v1/reconciliation/process', { matches: [] });
      if (!data.matches || data.matches.length === 0) {
        document.getElementById('reconciliation-content').innerHTML = 'No matches found or failed to load reconciliation data.';
        return;
      }
      
      var html = '<table><thead><tr><th>Date</th><th>Entry</th><th>Amount</th><th>Suggested Match</th><th>Action</th></tr></thead><tbody>';
      data.matches.forEach((m, idx) => {
        html += '<tr>' +
          '<td>' + m.entry.date + '</td>' +
          '<td>' + m.entry.counterparty + '<br><small style="color:#94a3b8">' + m.entry.reference + '</small></td>' +
          '<td>€' + m.entry.amount.toLocaleString() + '</td>' +
          '<td>' + (m.suggestedMatch ? m.suggestedMatch.type.toUpperCase() + ': ' + m.suggestedMatch.number + ' (Conf: ' + (m.suggestedMatch.confidence * 100) + '%)' : 'No match') + '</td>' +
          '<td>' + (m.suggestedMatch ? '<button class="btn btn-primary" onclick="confirmRecon(' + idx + ')" style="padding:4px 8px; font-size:11px;">Reconcile</button>' : '-') + '</td>' +
        '</tr>';
      });
      html += '</tbody></table>';
      document.getElementById('reconciliation-content').innerHTML = html;
      
      window.lastMatches = data.matches;
    } catch (e) { console.error(e); }
  }

  window.confirmRecon = async function(idx) {
    var match = window.lastMatches[idx];
    if (!confirm('Reconcile ' + match.entry.reference + ' with ' + match.suggestedMatch.number + '?')) return;
    
    try {
      var resp = await apiFetch('/api/v1/reconciliation/confirm', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ match: { entry: match.entry, targetType: match.suggestedMatch.type, targetId: match.suggestedMatch.id } })
      });
      var data = await resp.json();
      if (data.ok) {
        alert('Successfully reconciled!');
        loadReconciliation();
      }
    } catch (e) { alert('Failed to reconcile'); }
  }

  window.impersonateUser = async function(userId) {
    if (!confirm('Launch read-only impersonation session for this user?')) return;
    try {
      var resp = await apiFetch('/api/v1/support/impersonate', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ userId: userId })
      });
      var data = await resp.json();
      if (data.consoleUrl) {
        window.open(data.consoleUrl, '_blank');
      } else {
        alert('Failed to get impersonation link');
      }
    } catch (e) { alert('Error starting impersonation'); }
  }

  // Initial load
  switchView(initialView || 'dashboard', { skipHash: true });
})();
`);
});

app.get('/static/admin-logout.js', (req: Request, res: Response) => {
  res.setHeader('content-type', 'application/javascript; charset=utf-8');
  res.setHeader('cache-control', 'no-store, max-age=0, must-revalidate');
  res.setHeader('pragma', 'no-cache');
  res.setHeader('expires', '0');
  res.status(200).send(`
(function () {
  try { localStorage.removeItem('controlPlaneToken'); } catch (_) {}
  window.location.assign('/admin/login');
})();
`);
});

// Request logging
app.use((req: Request, res: Response, next: NextFunction) => {
  const start = Date.now();
  const requestId = `req_${Date.now()}_${Math.random().toString(36).substr(2, 9)}`;
  
  // Attach request ID
  (req as any).requestId = requestId;
  
  res.on('finish', () => {
    const duration = Date.now() - start;
    
    // Don't log health checks at info level
    if (req.path === '/health' || req.path === '/ready') {
      logger.debug('Request completed', {
        requestId,
        method: req.method,
        path: req.path,
        status: res.statusCode,
        duration,
      });
    } else {
      logger.info('Request completed', {
        requestId,
        method: req.method,
        path: req.path,
        status: res.statusCode,
        duration,
        ip: req.ip,
        userAgent: req.get('User-Agent'),
      });
    }
  });
  
  next();
});

// ============================================
// ROUTES
// ============================================

// Autonomous sales routes are mounted via routes.ts → /api/autonomous/*

app.use(routes);

// ============================================
// ERROR HANDLING
// ============================================

// 404 handler
app.use((req: Request, res: Response) => {
  res.status(404).json({ error: 'Not found' });
});

// Global error handler
app.use((err: Error, req: Request, res: Response, next: NextFunction) => {
  const requestId = (req as any).requestId || 'unknown';
  
  logger.error('Unhandled error', {
    requestId,
    error: err.message,
    stack: err.stack,
    path: req.path,
    method: req.method,
  });
  
  // Log audit for errors on action routes
  if (req.path.includes('/actions/')) {
    logAudit({
      actor: (req as any).operator?.username || 'unknown',
      action: `error:${req.path}`,
      result: 'failure',
      reason: err.message,
      ip: req.ip || 'unknown',
    });
  }
  
  // Don't leak error details in production
  if (config.NODE_ENV === 'production') {
    res.status(500).json({ error: 'Internal server error' });
  } else {
    res.status(500).json({ 
      error: err.message,
      stack: err.stack,
    });
  }
});

// ============================================
// SERVER STARTUP
// ============================================

async function startServer() {
  try {
    const skipDbInit = process.env.CONTROL_PLANE_SKIP_DB === 'true'

    if (skipDbInit) {
      logger.warn('Skipping database initialization (CONTROL_PLANE_SKIP_DB=true)');
    } else {
      // Initialize database pools
      logger.info('Initializing database connections...');
      await initializePools();

      // Verify read-only connection
      const pool = getReadOnlyPool();
      const result = await pool.query('SELECT NOW() as time');
      logger.info('Database connection verified', {
        serverTime: result.rows[0].time,
      });

      // Log execution mode
      if (process.env.SALES_AUTOMATION_ROLE === 'execution') {
         logger.info('Sales automation EXECUTION mode: handling requests locally');
         
         // Start the sales automation scheduler (only in execution mode)
         initializeSalesScheduler();
      } else {
         // App server is strictly a proxy client - all sales automation work
         // runs on test server via /api/autonomous/* proxy routes
         logger.info('Sales automation PROXY mode: all requests forwarded to test server');
      }
    }
    
    // Start HTTP server
    const server = app.listen(config.PORT, () => {
      logger.info(`Control plane started`, {
        port: config.PORT,
        nodeEnv: config.NODE_ENV,
      });
      
      console.log(`
╔══════════════════════════════════════════════════════════════╗
║                     CONTROL PLANE                            ║
║══════════════════════════════════════════════════════════════║
║  Status:    RUNNING                                          ║
║  Port:      ${config.PORT}                                          ║
║  Env:       ${config.NODE_ENV.padEnd(50)}║
║  Auth:      Operator JWT                                     ║
║  DB Roles:  read-only, operator, break-glass                 ║
╚══════════════════════════════════════════════════════════════╝
      `);
    });
    
    // Graceful shutdown
    const shutdown = async (signal: string) => {
      logger.info(`Received ${signal}, shutting down gracefully...`);
      
      // Stop the scheduler first
      stopSalesScheduler();
      
      server.close(async () => {
        logger.info('HTTP server closed');

        if (!skipDbInit) {
          await closePools();
          logger.info('Database pools closed');
        }
        
        process.exit(0);
      });
      
      // Force exit after 30 seconds
      setTimeout(() => {
        logger.error('Forced shutdown after timeout');
        process.exit(1);
      }, 30000);
    };
    
    process.on('SIGTERM', () => shutdown('SIGTERM'));
    process.on('SIGINT', () => shutdown('SIGINT'));
    
    // Handle uncaught errors
    process.on('uncaughtException', (err) => {
      logger.error('Uncaught exception', { error: err.message, stack: err.stack });
      process.exit(1);
    });
    
    process.on('unhandledRejection', (reason, promise) => {
      logger.error('Unhandled rejection', { reason: String(reason) });
    });
    
  } catch (err) {
    const error = err as Error;
    logger.error('Failed to start server', { error: error.message, stack: error.stack });
    console.error('Server startup error:', error);
    process.exit(1);
  }
}

startServer();
