import nodemailer from 'nodemailer';
import { getConfig } from './config';
import { logger } from './logger';

function escapeHtml(input: string): string {
  return input
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#39;');
}

function buildEmail(params: {
  operatorEmail: string;
  deviceLabel?: string;
  userAgent?: string;
  requestedIp?: string;
  confirmUrl: string;
  kind: 'approval-required' | 'bootstrap-notice';
}): { subject: string; text: string; html: string } {
  const subject =
    params.kind === 'approval-required'
      ? `[ApexMediation] Approve new operator device (${params.operatorEmail})`
      : `[ApexMediation] Operator device enrolled (${params.operatorEmail})`;

  const lines: string[] = [];
  lines.push(`Operator: ${params.operatorEmail}`);
  if (params.deviceLabel) lines.push(`Device: ${params.deviceLabel}`);
  if (params.requestedIp) lines.push(`IP: ${params.requestedIp}`);
  if (params.userAgent) lines.push(`User-Agent: ${params.userAgent}`);
  lines.push('');

  if (params.kind === 'approval-required') {
    lines.push('A new device attempted to access the ApexMediation Control Plane.');
    lines.push('Approve this device by opening:');
    lines.push(params.confirmUrl);
    lines.push('');
    lines.push('If this wasn\'t you, do not approve it.');
  } else {
    lines.push('A device was auto-enrolled as the first trusted device for this operator.');
    lines.push('If this is unexpected, revoke the device in the Control Plane.');
  }

  const text = lines.join('\n');

  const metaRows: string[] = [];
  metaRows.push(`<div><strong>Operator:</strong> ${escapeHtml(params.operatorEmail)}</div>`);
  if (params.deviceLabel) metaRows.push(`<div><strong>Device:</strong> ${escapeHtml(params.deviceLabel)}</div>`);
  if (params.requestedIp) metaRows.push(`<div><strong>IP:</strong> ${escapeHtml(params.requestedIp)}</div>`);
  if (params.userAgent) metaRows.push(`<div><strong>User-Agent:</strong> ${escapeHtml(params.userAgent)}</div>`);

  const actionBlock =
    params.kind === 'approval-required'
      ? `
        <p style="margin:16px 0;">A new device attempted to access the ApexMediation Control Plane.</p>
        <p style="margin:16px 0;">
          <a href="${escapeHtml(params.confirmUrl)}" style="display:inline-block;background:#2563eb;color:#fff;text-decoration:none;padding:10px 14px;border-radius:10px;font-weight:600;">Approve device</a>
        </p>
        <p style="margin:16px 0;color:#475569;font-size:13px;">If the button doesn\'t work, open this link:</p>
        <p style="margin:8px 0;"><a href="${escapeHtml(params.confirmUrl)}">${escapeHtml(params.confirmUrl)}</a></p>
        <p style="margin:16px 0;color:#7f1d1d;font-size:13px;">If this wasn\'t you, do not approve.</p>
      `
      : `
        <p style="margin:16px 0;">A device was auto-enrolled as the first trusted device for this operator.</p>
        <p style="margin:16px 0;color:#475569;font-size:13px;">If this is unexpected, revoke the device in the Control Plane.</p>
      `;

  const html = `
  <!doctype html>
  <html>
    <body style="margin:0;padding:0;background:#f8fafc;font-family:ui-sans-serif,system-ui,-apple-system,Segoe UI,Roboto,Helvetica,Arial;">
      <div style="max-width:640px;margin:0 auto;padding:24px;">
        <div style="background:#ffffff;border:1px solid #e2e8f0;border-radius:14px;padding:20px;">
          <h1 style="margin:0 0 8px;font-size:18px;color:#0f172a;">ApexMediation Control Plane</h1>
          <div style="font-size:13px;color:#334155;line-height:1.5;">${metaRows.join('')}</div>
          ${actionBlock}
          <hr style="border:none;border-top:1px solid #e2e8f0;margin:18px 0;" />
          <div style="font-size:12px;color:#64748b;">This message was sent because an operator login was attempted.</div>
        </div>
      </div>
    </body>
  </html>`;

  return { subject, text, html };
}

async function sendWithResend(params: {
  apiKey: string;
  from: string;
  to: string;
  subject: string;
  text: string;
  html: string;
}): Promise<void> {
  const resp = await fetch('https://api.resend.com/emails', {
    method: 'POST',
    headers: {
      Authorization: `Bearer ${params.apiKey}`,
      'content-type': 'application/json',
    },
    body: JSON.stringify({
      from: params.from,
      to: params.to,
      subject: params.subject,
      text: params.text,
      html: params.html,
    }),
  });

  if (!resp.ok) {
    const body = await resp.text().catch(() => '');
    throw new Error(`Resend email failed: ${resp.status} ${body}`);
  }
}

export async function sendDeviceApprovalEmail(params: {
  operatorEmail: string;
  deviceLabel?: string;
  userAgent?: string;
  requestedIp?: string;
  confirmUrl: string;
  kind: 'approval-required' | 'bootstrap-notice';
}): Promise<void> {
  const cfg = getConfig();

  const enabled = cfg.DEVICE_APPROVAL_ENABLED === 'true';
  if (!enabled) {
    return;
  }

  const to = cfg.DEVICE_APPROVAL_EMAIL_TO;
  const from = cfg.EMAIL_FROM;

  const { subject, text, html } = buildEmail({
    operatorEmail: params.operatorEmail,
    deviceLabel: params.deviceLabel,
    userAgent: params.userAgent,
    requestedIp: params.requestedIp,
    confirmUrl: params.confirmUrl,
    kind: params.kind,
  });

  const hasResend = Boolean(cfg.RESEND_API_KEY);
  const hasSmtp = Boolean(cfg.SMTP_HOST);

  if (!hasResend && !hasSmtp) {
    if (params.kind === 'bootstrap-notice') {
      logger.warn('No email provider configured; skipping bootstrap device email notice', { operatorEmail: params.operatorEmail });
      return;
    }
    throw new Error('Email provider not configured (set RESEND_API_KEY or SMTP_HOST)');
  }

  if (hasResend) {
    await sendWithResend({
      apiKey: cfg.RESEND_API_KEY!,
      from,
      to,
      subject,
      text,
      html,
    });
  } else {
    const transporter = nodemailer.createTransport({
      host: cfg.SMTP_HOST,
      port: parseInt(cfg.SMTP_PORT, 10),
      secure: cfg.SMTP_SECURE === 'true',
      auth: cfg.SMTP_USER && cfg.SMTP_PASS ? { user: cfg.SMTP_USER, pass: cfg.SMTP_PASS } : undefined,
    });

    await transporter.sendMail({
      to,
      from,
      subject,
      text,
      html,
    });
  }

  logger.info('Device approval email sent', { to, operatorEmail: params.operatorEmail, kind: params.kind });
}
