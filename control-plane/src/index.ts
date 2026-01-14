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

import express, { Request, Response, NextFunction } from 'express';
import helmet from 'helmet';
import cors from 'cors';
import compression from 'compression';

import { loadConfig } from './config';
import { logger, logAudit } from './logger';
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
      body { margin: 0; font-family: ui-sans-serif, system-ui, -apple-system, Segoe UI, Roboto, Helvetica, Arial; background: #0b1020; color: #e5e7eb; display: flex; height: 100vh; overflow: hidden; }
      
      /* Sidebar */
      .sidebar { width: 240px; border-right: 1px solid #1f2a44; background: #0f172a; display: flex; flex-direction: column; }
      .sidebar-header { padding: 20px; border-bottom: 1px solid #1f2a44; font-weight: bold; font-size: 18px; color: #3b82f6; }
      .sidebar-nav { flex: 1; padding: 12px 0; }
      .nav-item { padding: 10px 20px; cursor: pointer; color: #94a3b8; font-size: 14px; display: block; text-decoration: none; }
      .nav-item:hover { background: #1e293b; color: #e5e7eb; }
      .nav-item.active { background: #1e293b; color: #3b82f6; border-left: 3px solid #3b82f6; }
      .sidebar-footer { padding: 20px; border-top: 1px solid #1f2a44; font-size: 12px; }
      .sidebar-footer a { color: #ef4444; text-decoration: none; }

      /* Content Area */
      .main { flex: 1; display: flex; flex-direction: column; overflow: hidden; }
      .header { height: 60px; border-bottom: 1px solid #1f2a44; background: #0f172a; display: flex; align-items: center; justify-content: space-between; padding: 0 24px; }
      .header h1 { font-size: 16px; margin: 0; }
      .content { flex: 1; padding: 24px; overflow-y: auto; }

      /* Cards and Layout */
      .grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(300px, 1fr)); gap: 20px; margin-bottom: 24px; }
      .card { background: #0f172a; border: 1px solid #1f2a44; border-radius: 12px; padding: 20px; }
      .card-header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 16px; }
      .card-title { font-size: 14px; font-weight: 600; color: #94a3b8; text-transform: uppercase; letter-spacing: 0.05em; }
      
      /* Tables */
      table { width: 100%; border-collapse: collapse; font-size: 13px; }
      th, td { padding: 12px; border-bottom: 1px solid #1f2a44; text-align: left; }
      th { color: #94a3b8; font-weight: 500; }
      tr:hover { background: #111a2e; }

      /* Utilities */
      .pill { padding: 4px 10px; border-radius: 999px; background: #1e293b; font-size: 11px; }
      .pill-success { background: rgba(16, 185, 129, 0.2); color: #10b981; }
      .pill-warning { background: rgba(245, 158, 11, 0.2); color: #f59e0b; }
      .pill-danger { background: rgba(239, 68, 68, 0.2); color: #ef4444; }
      .btn { padding: 8px 16px; border-radius: 8px; border: 1px solid #334155; background: #0b1224; color: #e5e7eb; cursor: pointer; font-size: 13px; }
      .btn:hover { background: #1e293b; }
      .btn-primary { background: #2563eb; border: none; }
      .btn-primary:hover { background: #1d4ed8; }
      pre { margin: 0; font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace; font-size: 12px; overflow: auto; }
      
      .hidden { display: none !important; }
    </style>
  </head>
  <body>
    <div class="sidebar">
      <div class="sidebar-header">APEX Control</div>
      <nav class="sidebar-nav">
        <a class="nav-item active" data-view="dashboard">Dashboard</a>
        <a class="nav-item" data-view="compliance">Compliance</a>
        <a class="nav-item" data-view="data-explorer">Data Explorer</a>
        <a class="nav-item" data-view="analytics">Business Analytics</a>
        <a class="nav-item" data-view="adv-analytics">Advanced Analytics</a>
        <a class="nav-item" data-view="reconciliation">Reconciliation</a>
        <a class="nav-item" data-view="telemetry">System Telemetry</a>
        <a class="nav-item" data-view="partitions">Partitions</a>
      </nav>
      <div class="sidebar-footer">
        <a href="/admin/logout">Sign Out</a>
      </div>
    </div>
    
    <div class="main">
      <header class="header">
        <h1 id="view-title">Dashboard</h1>
        <div id="global-status" class="pill pill-success">System Online</div>
      </header>
      
      <div class="content">
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
              <div class="card-title">Disk Usage</div>
              <div id="disk-usage" style="font-size: 24px; font-weight: bold; margin-top: 10px;">-</div>
            </div>
          </div>
          <div class="card">
            <div class="card-header"><span class="card-title">Recent System Events</span></div>
            <pre id="recent-events">Loading system telemetry...</pre>
          </div>
        </div>

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
        <div id="view-telemetry" class="view hidden"><div class="card"><pre id="telemetry-content">Loading...</pre></div></div>
        <div id="view-partitions" class="view hidden"><div id="partitions-content">Loading...</div></div>
      </div>
    </div>

    <script src="/static/admin-dashboard.js"></script>
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
    <script src="/static/admin-logout.js"></script>
  </body>
</html>`
  );
});

app.get('/static/admin-login.js', (req: Request, res: Response) => {
  res.setHeader('content-type', 'application/javascript; charset=utf-8');
  res.status(200).send(`
(function () {
  var DEVICE_KEY = 'controlPlaneDeviceId';
  var form = document.getElementById('login-form');
  var errorEl = document.getElementById('error');
  var submit = document.getElementById('submit');
  if (!form || !errorEl || !submit) return;

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

      var resp = await fetch('/api/v1/auth/login', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
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
  var currentView = 'dashboard';
  
  // Navigation
  document.querySelectorAll('.nav-item').forEach(function(item) {
    item.addEventListener('click', function() {
      var view = item.getAttribute('data-view');
      switchView(view);
    });
  });

  function switchView(view) {
    currentView = view;
    document.querySelectorAll('.view').forEach(v => v.classList.add('hidden'));
    document.getElementById('view-' + view).classList.remove('hidden');
    
    document.querySelectorAll('.nav-item').forEach(i => i.classList.remove('active'));
    document.querySelector('.nav-item[data-view="' + view + '"]').classList.add('active');
    
    document.getElementById('view-title').textContent = view.charAt(0).toUpperCase() + view.slice(1).replace('-', ' ');
    
    loadViewData(view);
  }

  async function apiFetch(path, opts) {
    var token = localStorage.getItem('controlPlaneToken');
    if (!token) {
      window.location.assign('/admin/login');
      throw new Error('No token');
    }
    var headers = (opts && opts.headers) ? opts.headers : {};
    headers.Authorization = 'Bearer ' + token;
    return fetch(path, Object.assign({}, opts || {}, { headers: headers }));
  }

  async function loadViewData(view) {
    switch(view) {
      case 'dashboard': loadDashboard(); break;
      case 'compliance': loadCompliance(); break;
      case 'data-explorer': loadDataExplorer(); break;
      case 'analytics': loadAnalytics(); break;
      case 'adv-analytics': loadAdvAnalytics(); break;
      case 'reconciliation': loadReconciliation(true); break;
      case 'telemetry': loadTelemetry(); break;
      case 'partitions': loadPartitions(); break;
    }
  }

  async function loadDashboard() {
    try {
      var resp = await apiFetch('/api/v1/dashboard');
      var data = await resp.json();
      
      document.getElementById('health-status').textContent = data.health.overall;
      document.getElementById('health-status').style.color = 
        data.health.overall === 'OK' ? '#10b981' : (data.health.overall === 'DEGRADED' ? '#f59e0b' : '#ef4444');
      
      var backends = 0;
      if (data.database && data.database.length > 0) {
        backends = data.database.reduce((sum, db) => sum + (db.numbackends || 0), 0);
      }
      document.getElementById('db-sessions').textContent = backends;
      
      if (data.host && data.host.disk && data.host.disk.length > 0) {
        document.getElementById('disk-usage').textContent = data.host.disk[0].use_percent + '%';
      }
      
      var eventsHtml = data.health.issues.length > 0 
        ? data.health.issues.join('\n') 
        : 'No active issues detected.';
      document.getElementById('recent-events').textContent = eventsHtml;
    } catch (e) { console.error(e); }
  }

  async function loadCompliance() {
    try {
      var [statusResp, vatResp, forecastResp] = await Promise.all([
        apiFetch('/api/v1/compliance/status'),
        apiFetch('/api/v1/compliance/vat-reports'),
        apiFetch('/api/v1/compliance/forecast')
      ]);
      
      var status = await statusResp.json();
      var vat = await vatResp.json();
      var forecast = await forecastResp.json();
      
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
          '<td><button class="btn" onclick="downloadPdf(\'/api/v1/compliance/vat-reports/' + r.id + '/pdf\', \'vat-report-' + r.year + '-Q' + r.quarter + '.pdf\')" style="padding: 4px 8px; font-size: 11px;">PDF</button></td>' +
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
      var [usersResp, invResp] = await Promise.all([
        apiFetch('/api/v1/data/users'),
        apiFetch('/api/v1/data/invoices')
      ]);
      
      var users = await usersResp.json();
      var invoices = await invResp.json();
      
      var uHtml = '<table><thead><tr><th>Email</th><th>Company</th><th>Status</th><th>Actions</th></tr></thead><tbody>';
      users.users.forEach(u => {
        uHtml += '<tr><td>' + u.email + '</td><td>' + (u.company_name || '-') + '</td><td>' + u.status + '</td>' +
          '<td><button class="btn" onclick="impersonateUser(\'' + u.id + '\')" style="padding:4px 8px; font-size:11px;">Impersonate</button></td></tr>';
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
      var [appResp, dbResp, hostResp] = await Promise.all([
        apiFetch('/api/v1/telemetry/app'),
        apiFetch('/api/v1/telemetry/database'),
        apiFetch('/api/v1/telemetry/host')
      ]);
      
      var app = await appResp.json();
      var db = await dbResp.json();
      var host = await hostResp.json();
      
      document.getElementById('telemetry-content').textContent = JSON.stringify({
        app: app,
        database: db,
        host: host
      }, null, 2);
    } catch (e) { console.error(e); }
  }

  async function loadAnalytics() {
    try {
      var resp = await apiFetch('/api/v1/analytics/overview');
      var data = await resp.json();
      
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
      
      var insights = [
        'Active Experiments: ' + data.active_experiments,
        'Revenue is ' + (data.revenue_usd.change_pct >= 0 ? 'up' : 'down') + ' by ' + Math.abs(data.revenue_usd.change_pct) + '% compared to yesterday.',
        'Impression volume is ' + (data.impressions.change_pct >= 0 ? 'increasing' : 'decreasing') + '.',
        'Projected Runway: ' + data.financials.runway_months + ' months based on current burn.',
        'Customer LTV is currently €' + data.ceo_metrics.ltv_eur.toLocaleString() + '.'
      ];
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
      var resp = await apiFetch('/api/v1/partitions');
      var data = await resp.json();
      
      var html = '<table><thead><tr><th>Table</th><th>Partition</th><th>Range</th><th>Size</th></tr></thead><tbody>';
      data.statuses.forEach(s => {
        html += '<tr>' +
          '<td>' + s.parent_table + '</td>' +
          '<td>' + s.partition_name + '</td>' +
          '<td>' + s.range_start + ' to ' + s.range_end + '</td>' +
          '<td>' + s.size + '</td>' +
        '</tr>';
      });
      html += '</tbody></table>';
      
      if (!data.health.healthy) {
        html = '<div class="pill pill-danger" style="margin-bottom: 15px;">Partition issues detected: ' + 
          data.health.issues.join(', ') + '</div>' + html;
      }
      
      document.getElementById('partitions-content').innerHTML = html;
    } catch (e) { console.error(e); }
  }

  window.generateAnnualReport = async function() {
    if (!confirm('Generate 2025 Annual Report? This will be archived in the ledger.')) return;
    try {
      var resp = await apiFetch('/api/v1/compliance/annual-report/2025');
      var data = await resp.json();
      alert('Report generated and archived! Net Profit: €' + data.financials.profitEur.toLocaleString());
      loadCompliance();
    } catch (e) { alert('Failed to generate report'); }
  }

  async function loadAdvAnalytics() {
    try {
      var [cohortsResp, forecastResp] = await Promise.all([
        apiFetch('/api/v1/analytics/cohorts'),
        apiFetch('/api/v1/analytics/forecast')
      ]);
      var cohorts = await cohortsResp.json();
      var forecast = await forecastResp.json();

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
      var resp = await apiFetch('/api/v1/reconciliation/process');
      var data = await resp.json();
      
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
  switchView('dashboard');
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
    logger.error('Failed to start server', { error: (err as Error).message });
    process.exit(1);
  }
}

startServer();
