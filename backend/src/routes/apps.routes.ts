import { Router } from 'express'
import { authenticate } from '../middleware/auth'
import { authRateLimiter } from '../middleware/redisRateLimiter'
import * as appKeysController from '../controllers/appKeys.controller'

const router = Router()

// Apps are publisher-owned; manage keys via authenticated Console sessions.
router.use(authenticate)

// GET /api/v1/apps/:appId/keys
router.get('/:appId/keys', appKeysController.list)

// POST /api/v1/apps/:appId/keys
router.post('/:appId/keys', authRateLimiter, appKeysController.create)

// POST /api/v1/apps/:appId/keys/:id/rotate
router.post('/:appId/keys/:id/rotate', authRateLimiter, appKeysController.rotate)

// DELETE /api/v1/apps/:appId/keys/:id
router.delete('/:appId/keys/:id', authRateLimiter, appKeysController.revoke)

export default router
