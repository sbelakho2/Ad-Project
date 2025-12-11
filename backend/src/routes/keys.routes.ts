import { Router } from 'express';
import { authenticate } from '../middleware/auth';
import { authRateLimiter } from '../middleware/redisRateLimiter';
import * as apiKeysController from '../controllers/apiKeys.controller';

const router = Router();

// All API key management endpoints require an authenticated user session.
router.use(authenticate);

// GET /api/v1/keys
router.get('/', apiKeysController.listKeys);

// POST /api/v1/keys { live?: boolean }
router.post('/', authRateLimiter, apiKeysController.createKey);

// POST /api/v1/keys/:id/rotate
router.post('/:id/rotate', authRateLimiter, apiKeysController.rotateKey);

// DELETE /api/v1/keys/:id
router.delete('/:id', authRateLimiter, apiKeysController.deleteKey);

export default router;
