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

import { loadConfig } from './config';
import { logger, logAudit } from './logger';
import { initializePools, closePools, getReadOnlyPool, queryReadOnly } from './database';
import routes from './routes';
import { initializeSalesScheduler, stopSalesScheduler } from './services/sales-scheduler';
// NOTE: Orchestrator removed - app server is strictly a proxy client
// All sales automation work runs on test server (157.90.126.205:4001)

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
      scriptSrc: ["'self'", "'unsafe-inline'"],
      scriptSrcAttr: ["'unsafe-inline'"],
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
            <a class="nav-item" data-view="intelligence" href="#intelligence">Intelligence (V2)</a>
            <a class="nav-item" data-view="sales" href="#sales">Sales Automation</a>
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

        <!-- Sales Automation View -->
        <div id="view-sales" class="view hidden">
          <!-- System Enable/Disable Toggle -->
          <div class="card" style="margin-bottom: 24px; border: 2px solid #334155;">
            <div class="card-header">
              <div style="display: flex; align-items: center; gap: 12px;">
                <span class="card-title">System Status</span>
                <span id="sales-system-badge" style="padding: 4px 10px; border-radius: 12px; font-size: 12px; font-weight: 600;">-</span>
              </div>
              <button id="sales-toggle-btn" class="btn" onclick="toggleSalesSystem()">Enable System</button>
            </div>
            <div id="sales-system-info" style="font-size: 13px; color: #94a3b8; margin-top: 8px;">
              When disabled, emails will NOT be sent. Dry runs will still work.
            </div>
          </div>

          <!-- Status & KPIs -->
          <div class="grid">
            <div class="card">
              <div class="card-title">Contacts</div>
              <div id="sales-contacts-count" style="font-size: 24px; font-weight: bold; margin-top: 10px;">-</div>
              <div id="sales-contacts-breakdown" style="font-size: 12px; color: #94a3b8; margin-top: 5px;"></div>
            </div>
            <div class="card">
              <div class="card-title">Active Enrollments</div>
              <div id="sales-enrollments-count" style="font-size: 24px; font-weight: bold; margin-top: 10px;">-</div>
              <div id="sales-enrollments-breakdown" style="font-size: 12px; color: #94a3b8; margin-top: 5px;"></div>
            </div>
            <div class="card">
              <div class="card-title">Emails Sent</div>
              <div id="sales-emails-count" style="font-size: 24px; font-weight: bold; margin-top: 10px;">-</div>
              <div id="sales-emails-rates" style="font-size: 12px; color: #94a3b8; margin-top: 5px;"></div>
            </div>
            <div class="card">
              <div class="card-title">Today's Limit</div>
              <div id="sales-limit" style="font-size: 24px; font-weight: bold; margin-top: 10px;">-</div>
              <div id="sales-limit-detail" style="font-size: 12px; color: #94a3b8; margin-top: 5px;"></div>
            </div>
          </div>

          <!-- Actions -->
          <div class="card" style="margin-bottom: 24px;">
            <div class="card-header">
              <span class="card-title">Outreach Actions</span>
              <div>
                <button class="btn" onclick="runOutreachDryRun()">Dry Run</button>
                <button class="btn btn-primary" onclick="runOutreach()">Send Pending Emails</button>
              </div>
            </div>
            <div id="sales-action-result" style="font-size: 13px; color: #94a3b8;">Ready to process outreach queue.</div>
          </div>

          <!-- Sequences -->
          <div class="card" style="margin-bottom: 24px;">
            <div class="card-header">
              <span class="card-title">Email Sequences</span>
              <button class="btn btn-primary" onclick="showSequenceModal()">+ New Sequence</button>
            </div>
            <div id="sales-sequences-table">Loading sequences...</div>
          </div>

          <!-- Contacts -->
          <div class="card" style="margin-bottom: 24px;">
            <div class="card-header">
              <span class="card-title">Contacts</span>
              <div>
                <select id="contact-status-filter" onchange="loadSalesContacts()" style="padding: 6px 10px; background: #0b1224; border: 1px solid #334155; border-radius: 6px; color: #e5e7eb; margin-right: 8px;">
                  <option value="">All Status</option>
                  <option value="new">New</option>
                  <option value="contacted">Contacted</option>
                  <option value="replied">Replied</option>
                  <option value="meeting_scheduled">Meeting Scheduled</option>
                  <option value="converted">Converted</option>
                  <option value="opted_out">Opted Out</option>
                </select>
                <input type="text" id="contact-search" placeholder="Search..." onkeyup="debounceContactSearch()" style="padding: 6px 10px; background: #0b1224; border: 1px solid #334155; border-radius: 6px; color: #e5e7eb; margin-right: 8px; width: 150px;" />
                <button class="btn btn-primary" onclick="showAddContactModal()">+ Add Contact</button>
              </div>
            </div>
            <div id="sales-contacts-table">Loading contacts...</div>
          </div>

          <!-- Enrollments -->
          <div class="card" style="margin-bottom: 24px;">
            <div class="card-header">
              <span class="card-title">Enrollments</span>
              <button class="btn btn-primary" onclick="showEnrollModal()">Enroll New Contacts</button>
            </div>
            <div id="sales-enrollments-table">Loading enrollments...</div>
          </div>

          <!-- Recent Emails -->
          <div class="card">
            <div class="card-header">
              <span class="card-title">Recent Emails</span>
            </div>
            <div id="sales-emails-table">Loading emails...</div>
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

        <!-- Intelligence V2 View -->
        <div id="view-intelligence" class="view hidden">
          <!-- Worker Status -->
          <div class="card" style="margin-bottom: 24px; border: 2px solid #334155;">
            <div class="card-header">
              <div style="display: flex; align-items: center; gap: 12px;">
                <span class="card-title">Worker Status (Test Server)</span>
                <span id="intel-worker-badge" style="padding: 4px 10px; border-radius: 12px; font-size: 12px; font-weight: 600;">-</span>
              </div>
              <button class="btn" onclick="checkWorkerHealth()">Check Health</button>
            </div>
            <div id="intel-worker-info" style="font-size: 13px; color: #94a3b8; margin-top: 8px;">Click 'Check Health' to verify worker connection.</div>
          </div>

          <!-- System Stats -->
          <div class="grid">
            <div class="card">
              <div class="card-title">Discovery Sources</div>
              <div id="intel-sources-count" style="font-size: 24px; font-weight: bold; margin-top: 10px;">-</div>
              <div id="intel-sources-detail" style="font-size: 12px; color: #94a3b8; margin-top: 5px;"></div>
            </div>
            <div class="card">
              <div class="card-title">Scored Contacts</div>
              <div id="intel-contacts-scored" style="font-size: 24px; font-weight: bold; margin-top: 10px;">-</div>
              <div id="intel-contacts-detail" style="font-size: 12px; color: #94a3b8; margin-top: 5px;"></div>
            </div>
            <div class="card">
              <div class="card-title">Avg Lead Score</div>
              <div id="intel-avg-score" style="font-size: 24px; font-weight: bold; margin-top: 10px;">-</div>
              <div id="intel-score-detail" style="font-size: 12px; color: #94a3b8; margin-top: 5px;"></div>
            </div>
            <div class="card">
              <div class="card-title">Thompson Sampler</div>
              <div id="intel-bandit-arms" style="font-size: 24px; font-weight: bold; margin-top: 10px;">-</div>
              <div id="intel-bandit-detail" style="font-size: 12px; color: #94a3b8; margin-top: 5px;"></div>
            </div>
          </div>

          <!-- Job Triggers -->
          <div class="card" style="margin: 24px 0;">
            <div class="card-header">
              <span class="card-title">Job Controls</span>
              <div>
                <button class="btn" onclick="runDiscoveryJob()">Run Discovery</button>
                <button class="btn" onclick="runScoringJob()">Run Scoring</button>
                <button class="btn btn-primary" onclick="loadIntelligenceStats()">Refresh Stats</button>
              </div>
            </div>
            <div id="intel-job-result" style="font-size: 13px; color: #94a3b8;">Ready to run jobs via worker.</div>
          </div>

          <!-- Lead Sources Management -->
          <div class="card" style="margin-bottom: 24px;">
            <div class="card-header">
              <span class="card-title">Lead Sources</span>
              <button class="btn btn-primary" onclick="showAddSourceModal()">+ Add Source</button>
            </div>
            <div id="intel-sources-table">Loading sources...</div>
          </div>

          <!-- Hot Leads -->
          <div class="card" style="margin-bottom: 24px;">
            <div class="card-header">
              <span class="card-title">Hot Leads (Score ≥ 70)</span>
            </div>
            <div id="intel-hot-leads">Loading...</div>
          </div>

          <!-- Inbox / Review Queue -->
          <div class="card">
            <div class="card-header">
              <span class="card-title">Reply Inbox</span>
            </div>
            <div id="intel-inbox">Loading inbox...</div>
          </div>
        </div>
        <div class="scroll-spacer" aria-hidden="true"></div>
      </div>
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
(function () {
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

  async function apiFetch(path, opts) {
    var token = localStorage.getItem('controlPlaneToken');
    if (!token) {
      window.location.replace('/admin/login');
      throw new Error('No token');
    }
    var headers = (opts && opts.headers) ? opts.headers : {};
    headers.Authorization = 'Bearer ' + token;
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
          '<td><button class="btn" onclick="deleteSource(\\'' + s.id + '\\')" style="padding:4px 8px;font-size:11px;color:#ef4444;">Delete</button></td>' +
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
          '<td><button class="btn" style="padding:4px 8px;font-size:11px;">Review</button></td>' +
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
      var badge = document.getElementById('sales-system-badge');
      var toggleBtn = document.getElementById('sales-toggle-btn');
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
    var badge = document.getElementById('sales-system-badge');
    var isCurrentlyEnabled = badge.textContent === 'ENABLED';
    var newState = !isCurrentlyEnabled;
    var toggleBtn = document.getElementById('sales-toggle-btn');
    
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
            '<button class="btn" onclick="toggleSequence(\\''+s.id+'\\', '+ !s.active +')" style="padding:4px 8px;font-size:11px;">' + (s.active ? 'Pause' : 'Activate') + '</button> ' +
            '<button class="btn" onclick="editSequence(\\''+s.id+'\\')" style="padding:4px 8px;font-size:11px;">Edit</button>' +
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
            '<button class="btn" onclick="editContact(\\''+c.id+'\\')" style="padding:4px 8px;font-size:11px;">Edit</button> ' +
            '<button class="btn" onclick="deleteContact(\\''+c.id+'\\')" style="padding:4px 8px;font-size:11px;color:#ef4444;">Delete</button>' +
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
            (e.status === 'active' ? '<button class="btn" onclick="pauseEnrollment(\\''+e.id+'\\')" style="padding:4px 8px;font-size:11px;">Pause</button>' : '') +
            (e.status === 'paused' ? '<button class="btn" onclick="resumeEnrollment(\\''+e.id+'\\')" style="padding:4px 8px;font-size:11px;">Resume</button>' : '') +
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
      var body = prompt('Step ' + stepNum + ' - Email body (use {{first_name}}, {{company_name}}, {{app_name}}):', 'Hi {{first_name}},\\\\n\\\\nI noticed {{app_name}} and wanted to reach out...');
      if (body === null) { addMore = false; break; }
      
      steps.push({ delay_days: parseInt(delay) || 0, subject: subject, body: body.replace(/\\\\\\\\n/g, '\\\\n') });
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
          '<td><button class="btn" onclick="downloadPdf(\\'/api/v1/compliance/vat-reports/' + r.id + '/pdf\\', \\'vat-report-' + r.year + '-Q' + r.quarter + '.pdf\\')" style="padding: 4px 8px; font-size: 11px;">PDF</button></td>' +
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
          '<td><button class="btn" onclick="impersonateUser(\\'' + u.id + '\\')" style="padding:4px 8px; font-size:11px;">Impersonate</button></td></tr>';
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
    logger.error('Failed to start server', { error: (err as Error).message });
    process.exit(1);
  }
}

startServer();
