import { Router } from 'express'
import { authenticate } from '../middleware/auth'
import { authRateLimiter } from '../middleware/redisRateLimiter'
import * as appsController from '../controllers/apps.controller'
import * as appKeysController from '../controllers/appKeys.controller'

const router = Router()

// Apps are publisher-owned; manage keys via authenticated Console sessions.
router.use(authenticate)

// GET /api/v1/apps - list all apps for publisher
router.get('/', appsController.list)

// POST /api/v1/apps - create a new app
router.post('/', authRateLimiter, appsController.create)

// GET /api/v1/apps/:appId - get a specific app
router.get('/:appId', appsController.getById)

// PUT /api/v1/apps/:appId - update an app
router.put('/:appId', authRateLimiter, appsController.update)

// DELETE /api/v1/apps/:appId - delete an app
router.delete('/:appId', authRateLimiter, appsController.deleteApp)

// GET /api/v1/apps/:appId/keys
router.get('/:appId/keys', appKeysController.list)

// POST /api/v1/apps/:appId/keys
router.post('/:appId/keys', authRateLimiter, appKeysController.create)

// POST /api/v1/apps/:appId/keys/:id/rotate
router.post('/:appId/keys/:id/rotate', authRateLimiter, appKeysController.rotate)

// DELETE /api/v1/apps/:appId/keys/:id
router.delete('/:appId/keys/:id', authRateLimiter, appKeysController.revoke)

export default router
