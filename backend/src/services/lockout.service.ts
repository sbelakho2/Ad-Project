/**
 * Account Lockout Service
 * 
 * Handles account lockout logic for brute force protection.
 */

import { query } from '../utils/postgres';
import logger from '../utils/logger';
import emailService from './email.service';

// Configuration
const MAX_FAILED_ATTEMPTS = 10;
const LOCKOUT_DURATION_MINUTES = 30;
const LOCKOUT_DURATION_MS = LOCKOUT_DURATION_MINUTES * 60 * 1000;

export interface LockoutStatus {
  isLocked: boolean;
  lockedUntil: Date | null;
  failedAttempts: number;
  remainingAttempts: number;
}

export interface LoginAttemptRecord {
  userId: string | null;
  email: string;
  ipAddress: string | null;
  userAgent: string | null;
  success: boolean;
  failureReason: string | null;
}

/**
 * Check if an account is currently locked
 */
export const checkLockoutStatus = async (email: string): Promise<LockoutStatus> => {
  try {
    const result = await query<{
      failed_login_attempts: number;
      locked_until: Date | null;
    }>(
      `SELECT failed_login_attempts, locked_until 
       FROM users 
       WHERE email = $1`,
      [email.toLowerCase()]
    );

    if (!result.rows[0]) {
      // User doesn't exist - not locked
      return {
        isLocked: false,
        lockedUntil: null,
        failedAttempts: 0,
        remainingAttempts: MAX_FAILED_ATTEMPTS,
      };
    }

    const { failed_login_attempts, locked_until } = result.rows[0];

    // Check if lockout has expired
    if (locked_until && new Date(locked_until) > new Date()) {
      return {
        isLocked: true,
        lockedUntil: new Date(locked_until),
        failedAttempts: failed_login_attempts,
        remainingAttempts: 0,
      };
    }

    // Lockout expired or not locked
    return {
      isLocked: false,
      lockedUntil: null,
      failedAttempts: failed_login_attempts,
      remainingAttempts: Math.max(0, MAX_FAILED_ATTEMPTS - failed_login_attempts),
    };
  } catch (error) {
    logger.error('Error checking lockout status', { email, error });
    // SECURITY: Fail CLOSED on DB errors to prevent lockout bypass
    // An attacker could cause DB errors to circumvent brute force protection
    // Better to briefly block legitimate users than allow unlimited attempts
    return {
      isLocked: true,
      lockedUntil: new Date(Date.now() + 60000), // Lock for 1 minute on error
      failedAttempts: MAX_FAILED_ATTEMPTS,
      remainingAttempts: 0,
    };
  }
};

/**
 * Record a failed login attempt
 */
export const recordFailedAttempt = async (
  email: string,
  ipAddress: string | null,
  userAgent: string | null,
  failureReason: string
): Promise<LockoutStatus> => {
  try {
    // Update user's failed attempt count
    const updateResult = await query<{
      id: string;
      failed_login_attempts: number;
      locked_until: Date | null;
    }>(
      `UPDATE users 
       SET failed_login_attempts = CASE 
           WHEN locked_until IS NOT NULL AND locked_until > NOW() THEN failed_login_attempts
           ELSE failed_login_attempts + 1
         END,
         last_failed_login_at = NOW(),
         locked_until = CASE 
           WHEN failed_login_attempts + 1 >= $2 THEN NOW() + INTERVAL '${LOCKOUT_DURATION_MINUTES} minutes'
           ELSE locked_until
         END
       WHERE email = $1
       RETURNING id, failed_login_attempts, locked_until`,
      [email.toLowerCase(), MAX_FAILED_ATTEMPTS]
    );

    const user = updateResult.rows[0];

    // Record login attempt in audit log
    await query(
      `INSERT INTO login_attempts (user_id, email, ip_address, user_agent, success, failure_reason)
       VALUES ($1, $2, $3, $4, FALSE, $5)`,
      [user?.id || null, email.toLowerCase(), ipAddress, userAgent, failureReason]
    );

    if (!user) {
      return {
        isLocked: false,
        lockedUntil: null,
        failedAttempts: 1,
        remainingAttempts: MAX_FAILED_ATTEMPTS - 1,
      };
    }

    const isNowLocked = user.failed_login_attempts >= MAX_FAILED_ATTEMPTS;

    // Send lockout notification email if just locked
    if (isNowLocked && user.locked_until) {
      await emailService.sendEmail({
        to: email,
        subject: 'Account Temporarily Locked - ApexMediation',
        template: 'account-locked',
        data: {
          ipAddress,
          timestamp: new Date().toISOString(),
          failedAttempts: user.failed_login_attempts,
          lockDurationMinutes: LOCKOUT_DURATION_MINUTES,
        },
      });
      
      logger.warn('Account locked due to failed attempts', {
        email,
        failedAttempts: user.failed_login_attempts,
        lockedUntil: user.locked_until,
        ipAddress,
      });
    }

    return {
      isLocked: isNowLocked,
      lockedUntil: user.locked_until ? new Date(user.locked_until) : null,
      failedAttempts: user.failed_login_attempts,
      remainingAttempts: Math.max(0, MAX_FAILED_ATTEMPTS - user.failed_login_attempts),
    };
  } catch (error) {
    logger.error('Error recording failed attempt', { email, error });
    return {
      isLocked: false,
      lockedUntil: null,
      failedAttempts: 0,
      remainingAttempts: MAX_FAILED_ATTEMPTS,
    };
  }
};

/**
 * Record a successful login (resets failed attempts)
 */
export const recordSuccessfulLogin = async (
  userId: string,
  email: string,
  ipAddress: string | null,
  userAgent: string | null
): Promise<void> => {
  try {
    // Reset failed attempts on successful login
    await query(
      `UPDATE users 
       SET failed_login_attempts = 0, 
           locked_until = NULL,
           last_failed_login_at = NULL
       WHERE id = $1`,
      [userId]
    );

    // Record successful login in audit log
    await query(
      `INSERT INTO login_attempts (user_id, email, ip_address, user_agent, success)
       VALUES ($1, $2, $3, $4, TRUE)`,
      [userId, email.toLowerCase(), ipAddress, userAgent]
    );

    logger.debug('Successful login recorded', { userId, email });
  } catch (error) {
    // Non-critical - log but don't fail login
    logger.error('Error recording successful login', { userId, email, error });
  }
};

/**
 * Manually unlock an account (admin action)
 */
export const unlockAccount = async (email: string): Promise<boolean> => {
  try {
    const result = await query(
      `UPDATE users 
       SET failed_login_attempts = 0, 
           locked_until = NULL,
           last_failed_login_at = NULL
       WHERE email = $1
       RETURNING id`,
      [email.toLowerCase()]
    );

    if (result.rows.length > 0) {
      logger.info('Account manually unlocked', { email });
      return true;
    }

    return false;
  } catch (error) {
    logger.error('Error unlocking account', { email, error });
    return false;
  }
};

export default {
  checkLockoutStatus,
  recordFailedAttempt,
  recordSuccessfulLogin,
  unlockAccount,
  MAX_FAILED_ATTEMPTS,
  LOCKOUT_DURATION_MINUTES,
};
