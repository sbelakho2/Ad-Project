/**
 * ADMIN ROUTES - DEPRECATED / REMOVED
 * 
 * ===========================================================================
 * IMPORTANT: All operator/system admin functionality has been moved to the
 * CONTROL PLANE service running on a separate hostname (admin.apexmediation.ee)
 * 
 * This file exists only to return a helpful error message if any client
 * attempts to access the old /admin routes on the customer API.
 * ===========================================================================
 * 
 * Migration completed: ${new Date().toISOString()}
 * 
 * Control plane features:
 * - Billing reconciliation
 * - Adapter configuration
 * - System health monitoring
 * - Session management
 * - Partition management
 * 
 * Access the control plane at: https://admin.apexmediation.ee
 * (requires operator credentials and IP allowlist)
 */

import { Router, Request, Response } from 'express';

const router = Router();

// Catch-all for any /admin/* request
router.all('*', (_req: Request, res: Response) => {
  res.status(410).json({
    success: false,
    error: 'GONE',
    message: 'Operator/admin functionality has been moved to the separate control plane service.',
    details: 'This endpoint no longer exists on the customer API. System administrators should access the control plane at the designated admin hostname.',
    migration: {
      completed: true,
      reason: 'Security architecture separation - operator and customer APIs are now completely separate',
      documentation: 'See docs/Architecture/CONTROL_PLANE.md',
    },
  });
});

export default router;
