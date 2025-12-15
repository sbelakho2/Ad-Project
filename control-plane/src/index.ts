/**
 * CONTROL PLANE - MAIN ENTRY POINT
 * 
 * Separate control plane service for system administration.
 * Runs on its own port (4000) with its own hostname (admin.apexmediation.ee)
 * 
 * Security layers:
 * 1. IP allowlist - returns 404 for non-allowed IPs
 * 2. Operator JWT authentication - separate from customer auth
 * 3. Tiered action permissions - confirmation required for dangerous ops
 * 4. Full audit logging with 7-year retention
 */

import express, { Request, Response, NextFunction } from 'express';
import helmet from 'helmet';
import cors from 'cors';
import compression from 'compression';

import { loadConfig } from './config';
import { logger, logAudit } from './logger';
import { ipAllowlist } from './auth';
import { initializePools, closePools, getReadOnlyPool } from './database';
import routes from './routes';

// Load and validate configuration
const config = loadConfig();

const app = express();

// ============================================
// SECURITY MIDDLEWARE
// ============================================

// Helmet for security headers
app.use(helmet({
  contentSecurityPolicy: {
    directives: {
      defaultSrc: ["'self'"],
      scriptSrc: ["'self'"],
      styleSrc: ["'self'", "'unsafe-inline'"],
      imgSrc: ["'self'", 'data:'],
    },
  },
  hsts: {
    maxAge: 31536000,
    includeSubDomains: true,
    preload: true,
  },
}));

// CORS - only allow same origin (admin.* subdomain)
app.use(cors({
  origin: config.CORS_ORIGIN || false,
  credentials: true,
}));

// Trust proxy for accurate IP detection behind Caddy/nginx
app.set('trust proxy', true);

// IP Allowlist - FIRST security check
// Returns 404 for non-allowed IPs (stealth mode)
app.use(ipAllowlist);

// ============================================
// STANDARD MIDDLEWARE
// ============================================

// Compression
app.use(compression());

// JSON body parsing
app.use(express.json({ limit: '1mb' }));

// ============================================
// MINIMAL OPERATOR UI (same-origin)
// ============================================

function sendHtml(res: Response, html: string): void {
  res.setHeader('content-type', 'text/html; charset=utf-8');
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
  res.redirect('/admin/');
});

app.get('/admin', (req: Request, res: Response) => {
  res.redirect('/admin/');
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
      :root { color-scheme: dark; }
      body { margin: 0; font-family: ui-sans-serif, system-ui, -apple-system, Segoe UI, Roboto, Helvetica, Arial; background: #0b1020; color: #e5e7eb; }
      .wrap { min-height: 100vh; display: grid; place-items: center; padding: 24px; }
      .card { width: 100%; max-width: 440px; background: #0f172a; border: 1px solid #1f2a44; border-radius: 14px; padding: 24px; }
      h1 { margin: 0 0 6px; font-size: 22px; }
      p { margin: 0 0 18px; color: #9ca3af; font-size: 14px; }
      label { display: block; font-size: 12px; color: #cbd5e1; margin: 14px 0 6px; }
      input { width: 100%; padding: 10px 12px; border-radius: 10px; border: 1px solid #334155; background: #0b1224; color: #e5e7eb; }
      button { width: 100%; margin-top: 16px; padding: 10px 12px; border-radius: 10px; border: 0; background: #2563eb; color: white; font-weight: 600; cursor: pointer; }
      button:disabled { opacity: .6; cursor: not-allowed; }
      .err { display:none; margin-top: 12px; padding: 10px 12px; border-radius: 10px; border: 1px solid #7f1d1d; background: rgba(127,29,29,.35); color: #fecaca; font-size: 13px; }
      .foot { margin-top: 16px; color: #94a3b8; font-size: 12px; }
      .muted { color: #94a3b8; }
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
          <div class="muted">If you see 404s, your IP may not be allowlisted.</div>
        </div>
      </div>
    </div>

    <script src="/static/admin-login.js" defer></script>
  </body>
</html>`
  );
});

app.get('/admin/', (req: Request, res: Response) => {
  const title = 'Control Plane';
  sendHtml(
    res,
    `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>${escapeHtml(title)}</title>
    <style>
      :root { color-scheme: dark; }
      body { margin: 0; font-family: ui-sans-serif, system-ui, -apple-system, Segoe UI, Roboto, Helvetica, Arial; background: #0b1020; color: #e5e7eb; }
      .top { display:flex; justify-content: space-between; align-items:center; padding: 16px 20px; border-bottom: 1px solid #1f2a44; background: #0f172a; }
      .top h1 { margin: 0; font-size: 16px; }
      .top a { color: #93c5fd; text-decoration: none; font-size: 13px; }
      .wrap { padding: 20px; }
      .card { background: #0f172a; border: 1px solid #1f2a44; border-radius: 14px; padding: 16px; }
      .row { display:flex; gap: 12px; flex-wrap: wrap; margin-bottom: 12px; }
      .pill { padding: 6px 10px; border-radius: 999px; border: 1px solid #334155; background: #0b1224; font-size: 12px; }
      pre { margin: 0; white-space: pre-wrap; word-break: break-word; font-size: 12px; color: #d1d5db; }
      .err { display:none; margin-top: 12px; padding: 10px 12px; border-radius: 10px; border: 1px solid #7f1d1d; background: rgba(127,29,29,.35); color: #fecaca; font-size: 13px; }
      .muted { color: #94a3b8; }
    </style>
  </head>
  <body>
    <div class="top">
      <h1>Control Plane</h1>
      <a href="/admin/logout">Sign out</a>
    </div>
    <div class="wrap">
      <div class="row">
        <div class="pill" id="status">Loading…</div>
        <div class="pill muted">API: /api/v1/dashboard</div>
      </div>
      <div id="error" class="err"></div>
      <div class="card">
        <pre id="payload">Loading…</pre>
      </div>
    </div>
    <script src="/static/admin-dashboard.js" defer></script>
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
    <title>Signed out</title>
  </head>
  <body>
    <script src="/static/admin-logout.js" defer></script>
  </body>
</html>`
  );
});

app.get('/static/admin-login.js', (req: Request, res: Response) => {
  res.setHeader('content-type', 'application/javascript; charset=utf-8');
  res.status(200).send(`
(function () {
  var form = document.getElementById('login-form');
  var errorEl = document.getElementById('error');
  var submit = document.getElementById('submit');
  if (!form || !errorEl || !submit) return;

  function setError(msg) {
    errorEl.textContent = msg;
    errorEl.style.display = msg ? 'block' : 'none';
  }

  form.addEventListener('submit', async function (e) {
    e.preventDefault();
    setError('');
    submit.disabled = true;
    submit.textContent = 'Signing in…';

    try {
      var username = document.getElementById('username').value;
      var password = document.getElementById('password').value;

      var resp = await fetch('/api/v1/auth/login', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ username: username, password: password }),
      });

      var data = null;
      try { data = await resp.json(); } catch (_) {}

      if (!resp.ok || !data || !data.token) {
        var msg = (data && (data.error || data.message)) ? (data.error || data.message) : 'Authentication failed';
        setError(msg);
        return;
      }

      localStorage.setItem('controlPlaneToken', data.token);
      window.location.assign('/admin/');
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
  res.status(200).send(`
(function () {
  var statusEl = document.getElementById('status');
  var payloadEl = document.getElementById('payload');
  var errorEl = document.getElementById('error');
  if (!statusEl || !payloadEl || !errorEl) return;

  function setError(msg) {
    errorEl.textContent = msg;
    errorEl.style.display = msg ? 'block' : 'none';
  }

  function setStatus(msg) {
    statusEl.textContent = msg;
  }

  async function run() {
    var token = localStorage.getItem('controlPlaneToken');
    if (!token) {
      window.location.assign('/admin/login');
      return;
    }

    try {
      setError('');
      setStatus('Loading dashboard…');

      var resp = await fetch('/api/v1/dashboard', {
        headers: { Authorization: 'Bearer ' + token },
      });

      if (resp.status === 401) {
        localStorage.removeItem('controlPlaneToken');
        window.location.assign('/admin/login');
        return;
      }

      var data = await resp.json();
      if (!resp.ok) {
        setStatus('Error');
        setError((data && data.error) ? data.error : 'Failed to load dashboard');
        payloadEl.textContent = JSON.stringify(data, null, 2);
        return;
      }

      setStatus('OK');
      payloadEl.textContent = JSON.stringify(data, null, 2);
    } catch (err) {
      setStatus('Error');
      setError('Network error');
    }
  }

  run();
})();
`);
});

app.get('/static/admin-logout.js', (req: Request, res: Response) => {
  res.setHeader('content-type', 'application/javascript; charset=utf-8');
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
    // Initialize database pools
    logger.info('Initializing database connections...');
    await initializePools();
    
    // Verify read-only connection
    const pool = getReadOnlyPool();
    const result = await pool.query('SELECT NOW() as time');
    logger.info('Database connection verified', { 
      serverTime: result.rows[0].time 
    });
    
    // Start HTTP server
    const server = app.listen(config.PORT, () => {
      logger.info(`Control plane started`, {
        port: config.PORT,
        nodeEnv: config.NODE_ENV,
        allowedIPs: config.ALLOWED_IPS.split(',').length,
      });
      
      console.log(`
╔══════════════════════════════════════════════════════════════╗
║                     CONTROL PLANE                            ║
║══════════════════════════════════════════════════════════════║
║  Status:    RUNNING                                          ║
║  Port:      ${config.PORT}                                          ║
║  Env:       ${config.NODE_ENV.padEnd(50)}║
║  Auth:      IP Allowlist + Operator JWT                      ║
║  DB Roles:  read-only, operator, break-glass                 ║
╚══════════════════════════════════════════════════════════════╝
      `);
    });
    
    // Graceful shutdown
    const shutdown = async (signal: string) => {
      logger.info(`Received ${signal}, shutting down gracefully...`);
      
      server.close(async () => {
        logger.info('HTTP server closed');
        
        await closePools();
        logger.info('Database pools closed');
        
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
    logger.error('Failed to start server', { error: (err as Error).message });
    process.exit(1);
  }
}

startServer();
