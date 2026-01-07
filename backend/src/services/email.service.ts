/**
 * Email Service
 * 
 * Handles sending transactional emails for verification, password reset, etc.
 * Uses a stub implementation that logs emails in development.
 * In production, configure SMTP or use a service like Resend/SendGrid.
 */

import crypto from 'node:crypto';
import logger from '../utils/logger';

export interface EmailOptions {
  to: string;
  subject: string;
  template: 'email-verification' | 'password-reset' | 'login-alert' | 'account-locked';
  data: Record<string, unknown>;
}

export interface EmailResult {
  success: boolean;
  messageId?: string;
  error?: string;
}

const FRONTEND_URL = process.env.FRONTEND_URL || 'https://console.apexmediation.ee';

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

    default:
      return `<p>${JSON.stringify(data)}</p>`;
  }
};

/**
 * Send an email
 * In development/test, logs the email instead of sending
 */
export const sendEmail = async (options: EmailOptions): Promise<EmailResult> => {
  const messageId = crypto.randomUUID();
  const html = generateEmailHtml(options.template, options.data);

  // In production with SMTP configured, send real email
  if (process.env.SMTP_HOST && process.env.NODE_ENV === 'production') {
    try {
      // TODO: Implement actual SMTP sending with nodemailer
      // For now, log that we would send
      logger.info('Would send email via SMTP', {
        to: options.to,
        subject: options.subject,
        template: options.template,
        messageId,
      });
      return { success: true, messageId };
    } catch (error) {
      logger.error('Failed to send email', {
        to: options.to,
        subject: options.subject,
        error: error instanceof Error ? error.message : 'Unknown error',
      });
      return { success: false, error: 'Email sending failed' };
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

  // Log the verification/reset URL for easy access in development
  if (options.data.verificationUrl) {
    logger.info(`📧 Verification URL: ${options.data.verificationUrl}`);
  }
  if (options.data.resetUrl) {
    logger.info(`📧 Password Reset URL: ${options.data.resetUrl}`);
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
