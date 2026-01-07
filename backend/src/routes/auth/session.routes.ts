import { Router } from 'express';
import rateLimit from 'express-rate-limit';
import * as authController from '../../controllers/auth.controller';
import { authenticate } from '../../middleware/auth';
import { issueCsrfToken } from '../../middleware/csrf';

const router = Router();

// Rate limit login attempts to prevent brute force attacks
// 5 attempts per 15-minute window, successful requests don't count
const loginRateLimiter = rateLimit({
  windowMs: 15 * 60 * 1000, // 15 minutes
  max: 5,
  skipSuccessfulRequests: true,
  standardHeaders: true,
  legacyHeaders: false,
  message: { error: 'Too many login attempts, please try again later' },
});

// Rate limit password reset requests
const passwordResetRateLimiter = rateLimit({
  windowMs: 5 * 60 * 1000, // 5 minutes
  max: 3,
  standardHeaders: true,
  legacyHeaders: false,
  message: { error: 'Too many password reset attempts, please try again later' },
});

// Rate limit email verification resend
const verificationResendRateLimiter = rateLimit({
  windowMs: 5 * 60 * 1000, // 5 minutes
  max: 3,
  standardHeaders: true,
  legacyHeaders: false,
  message: { error: 'Too many verification requests, please try again later' },
});

// POST /api/v1/auth/login - Authenticate user
router.post('/login', loginRateLimiter, authController.login);

// POST /api/v1/auth/login/2fa - Complete 2FA step-up login using tempToken + code
router.post('/login/2fa', loginRateLimiter, authController.login2fa);

// POST /api/v1/auth/refresh - Refresh access token
router.post('/refresh', authController.refresh);

// GET /api/v1/auth/csrf - Issue CSRF token cookie and return token
router.get('/csrf', issueCsrfToken);

// GET /api/v1/auth/me - Return current session user (cookie/JWT based)
router.get('/me', authenticate, authController.me);

// POST /api/v1/auth/logout - Clear auth cookies
router.post('/logout', authenticate, authController.logout);

// Email verification endpoints
router.post('/verify-email', authController.verifyEmail);
router.post('/resend-verification', verificationResendRateLimiter, authController.resendVerification);

// Password reset endpoints
router.post('/forgot-password', passwordResetRateLimiter, authController.requestPasswordReset);
router.post('/reset-password', authController.resetPassword);

export default router;
