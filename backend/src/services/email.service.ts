/**
 * Email Service
 * 
 * Handles sending transactional emails for verification, password reset, etc.
 * Uses ApexMail (self-hosted) via the Control Plane API.
 * Falls back to stub mode if ApexMail is not available.
 */

import crypto from 'node:crypto';
import logger from '../utils/logger';

export interface EmailOptions {
  to: string;
  subject: string;
  template: 'email-verification' | 'password-reset' | 'login-alert' | 'account-locked' | 'team-invite';
  data: Record<string, unknown>;
}

export interface EmailResult {
  success: boolean;
  messageId?: string;
  error?: string;
}

const FRONTEND_URL = process.env.FRONTEND_URL || 'https://console.apexmediation.ee';
const DEFAULT_FROM = process.env.EMAIL_FROM || 'ApexMediation <no-reply@apexmediation.ee>';

// ApexMail API endpoint (via Control Plane on App Server or localhost on Test Server)
const APEXMAIL_BASE_URL = process.env.APEXMAIL_URL || 'http://localhost:4000/api/apexmail';
const APEXMAIL_API_KEY = process.env.APEXMAIL_API_KEY;

/**
 * Generate email HTML based on template
 */
const generateEmailHtml = (template: EmailOptions['template'], data: Record<string, unknown>): string => {
  switch (template) {
    case 'email-verification':
      return `
        <!DOCTYPE html>
        <html>
        <head><meta charset="utf-8"></head>
        <body style="font-family: Arial, sans-serif; max-width: 600px; margin: 0 auto;">
          <h1 style="color: #333;">Verify Your Email</h1>
          <p>Welcome to ApexMediation! Please verify your email address by clicking the button below:</p>
          <a href="${data.verificationUrl}" 
             style="display: inline-block; background: #4F46E5; color: white; padding: 12px 24px; 
                    text-decoration: none; border-radius: 6px; margin: 16px 0;">
            Verify Email
          </a>
          <p style="color: #666; font-size: 14px;">
            This link will expire in 24 hours. If you didn't create an account, you can ignore this email.
          </p>
          <hr style="border: none; border-top: 1px solid #eee; margin: 24px 0;">
          <p style="color: #999; font-size: 12px;">ApexMediation - Ad Monetization Platform</p>
        </body>
        </html>
      `;

    case 'password-reset':
      return `
        <!DOCTYPE html>
        <html>
        <head><meta charset="utf-8"></head>
        <body style="font-family: Arial, sans-serif; max-width: 600px; margin: 0 auto;">
          <h1 style="color: #333;">Reset Your Password</h1>
          <p>We received a request to reset your password. Click the button below to create a new password:</p>
          <a href="${data.resetUrl}" 
             style="display: inline-block; background: #4F46E5; color: white; padding: 12px 24px; 
                    text-decoration: none; border-radius: 6px; margin: 16px 0;">
            Reset Password
          </a>
          <p style="color: #666; font-size: 14px;">
            This link will expire in 1 hour. If you didn't request a password reset, you can ignore this email.
          </p>
          <hr style="border: none; border-top: 1px solid #eee; margin: 24px 0;">
          <p style="color: #999; font-size: 12px;">ApexMediation - Ad Monetization Platform</p>
        </body>
        </html>
      `;

    case 'account-locked':
      return `
        <!DOCTYPE html>
        <html>
        <head><meta charset="utf-8"></head>
        <body style="font-family: Arial, sans-serif; max-width: 600px; margin: 0 auto;">
          <h1 style="color: #dc2626;">Account Temporarily Locked</h1>
          <p>Your ApexMediation account has been temporarily locked due to multiple failed login attempts.</p>
          <p><strong>Details:</strong></p>
          <ul>
            <li>IP Address: ${data.ipAddress || 'Unknown'}</li>
            <li>Time: ${data.timestamp || new Date().toISOString()}</li>
            <li>Failed Attempts: ${data.failedAttempts || 'Multiple'}</li>
          </ul>
          <p>Your account will be automatically unlocked in ${data.lockDurationMinutes || 30} minutes.</p>
          <p style="color: #666; font-size: 14px;">
            If this wasn't you, we recommend changing your password immediately after the lockout expires.
          </p>
          <hr style="border: none; border-top: 1px solid #eee; margin: 24px 0;">
          <p style="color: #999; font-size: 12px;">ApexMediation - Ad Monetization Platform</p>
        </body>
        </html>
      `;

    case 'login-alert':
      return `
        <!DOCTYPE html>
        <html>
        <head><meta charset="utf-8"></head>
        <body style="font-family: Arial, sans-serif; max-width: 600px; margin: 0 auto;">
          <h1 style="color: #333;">New Login Detected</h1>
          <p>A new login to your ApexMediation account was detected:</p>
          <ul>
            <li>IP Address: ${data.ipAddress || 'Unknown'}</li>
            <li>Device: ${data.userAgent || 'Unknown'}</li>
            <li>Time: ${data.timestamp || new Date().toISOString()}</li>
          </ul>
          <p style="color: #666; font-size: 14px;">
            If this wasn't you, please change your password immediately and enable 2FA.
          </p>
          <hr style="border: none; border-top: 1px solid #eee; margin: 24px 0;">
          <p style="color: #999; font-size: 12px;">ApexMediation - Ad Monetization Platform</p>
        </body>
        </html>
      `;

    case 'team-invite':
      return `
        <!DOCTYPE html>
        <html>
        <head><meta charset="utf-8"></head>
        <body style="font-family: Arial, sans-serif; max-width: 600px; margin: 0 auto;">
          <h1 style="color: #333;">You're Invited</h1>
          <p>${data.inviterEmail || 'A team member'} has invited you to join their ApexMediation workspace.</p>
          <p><strong>Role:</strong> ${data.role || 'Member'}</p>
          <a href="${data.inviteUrl}"
             style="display: inline-block; background: #4F46E5; color: white; padding: 12px 24px;
                    text-decoration: none; border-radius: 6px; margin: 16px 0;">
            Accept Invitation
          </a>
          <p style="color: #666; font-size: 14px;">
            This invite link will expire in 7 days. If you weren't expecting this, you can ignore this email.
          </p>
          <hr style="border: none; border-top: 1px solid #eee; margin: 24px 0;">
          <p style="color: #999; font-size: 12px;">ApexMediation - Ad Monetization Platform</p>
        </body>
        </html>
      `;

    default:
      return `<p>${JSON.stringify(data)}</p>`;
  }
};

/**
 * Send an email via ApexMail API
 * Falls back to stub mode if ApexMail is not configured
 */
export const sendEmail = async (options: EmailOptions): Promise<EmailResult> => {
  const messageId = crypto.randomUUID();
  const html = generateEmailHtml(options.template, options.data);

  const text = typeof html === 'string'
    ? html.replace(/<[^>]*>/g, ' ').replace(/\s+/g, ' ').trim()
    : '';

  // Try ApexMail API first
  if (APEXMAIL_API_KEY) {
    try {
      const headers: Record<string, string> = {
        'Content-Type': 'application/json',
        'X-ApexMail-Key': APEXMAIL_API_KEY,
      };

      const response = await fetch(`${APEXMAIL_BASE_URL}/send`, {
        method: 'POST',
        headers,
        body: JSON.stringify({
          to: options.to,
          from: DEFAULT_FROM,
          subject: options.subject,
          html,
          text,
          category: 'transactional',
          priority: 1,
        }),
      });

      if (response.ok) {
        const result = await response.json() as { emailId?: string };
        return { success: true, messageId: result.emailId || messageId };
      } else {
        const error = await response.text();
        logger.error('ApexMail send failed', { status: response.status, error });
        // Fall through to stub mode
      }
    } catch (error) {
      logger.error('ApexMail API error', {
        to: options.to,
        subject: options.subject,
        error: error instanceof Error ? error.message : 'Unknown error',
      });
      // Fall through to stub mode
    }
  }

  // Development/test mode: log email details
  logger.info('Email send (stub)', {
    to: options.to,
    subject: options.subject,
    template: options.template,
    data: options.data,
    messageId,
  });

  // Log the verification/reset URL ONLY in development for debugging
  // P2 Fix #1: Never log sensitive URLs in production
  if (process.env.NODE_ENV !== 'production') {
    if (options.data.verificationUrl) {
      logger.info(`📧 Verification URL: ${options.data.verificationUrl}`);
    }
    if (options.data.resetUrl) {
      logger.info(`📧 Password Reset URL: ${options.data.resetUrl}`);
    }
  }

  return { success: true, messageId };
};

/**
 * Generate email verification token and URL
 */
export const generateVerificationToken = (): { token: string; url: string } => {
  const token = crypto.randomUUID();
  const url = `${FRONTEND_URL}/verify-email?token=${token}`;
  return { token, url };
};

/**
 * Generate password reset token and URL
 */
export const generatePasswordResetToken = (): { token: string; url: string } => {
  const token = crypto.randomUUID();
  const url = `${FRONTEND_URL}/reset-password?token=${token}`;
  return { token, url };
};

export default {
  sendEmail,
  generateVerificationToken,
  generatePasswordResetToken,
};
