/**
 * Health Check Routes
 * 
 * Kubernetes-compatible health probes
 */

import { Router } from 'express';
import { Pool } from 'pg';
import { HealthCheckController } from '../controllers/HealthCheckController';
import { authenticate } from '../middleware/auth';

export function createHealthCheckRoutes(pool: Pool): Router {
  const router = Router();
  const controller = new HealthCheckController(pool);

  /**
   * Liveness probe
   * GET /health
   * 
   * No authentication required - Kubernetes needs unrestricted access
   */
  router.get('/health', (req, res) => controller.liveness(req, res));

  /**
   * Readiness probe
   * GET /ready
   * 
   * No authentication required - Kubernetes needs unrestricted access
   */
  router.get('/ready', (req, res) => controller.readiness(req, res));

  /**
   * Startup probe
   * GET /startup
   * 
   * No authentication required - Kubernetes needs unrestricted access
   */
  router.get('/startup', (req, res) => controller.startup(req, res));

  /**
   * Detailed status (authenticated)
   * GET /api/v1/status
   */
  router.get('/api/v1/status.json', (req, res) => controller.publicStatus(req, res));
  router.get('/api/v1/slo.json', (req, res) => controller.publicSLOs(req, res));
  router.get('/api/v1/status', authenticate, (req, res) => controller.detailedStatus(req, res));

  return router;
}
