import { Request, Response, NextFunction } from 'express'
import * as appRepo from '../repositories/appRepository'
import { AppError } from '../middleware/errorHandler'
import logger from '../utils/logger'

const VALID_PLATFORMS = ['android', 'ios', 'web', 'unity', 'ctv', 'tvos', 'android-tv'] as const

const requirePublisherId = (req: Request): string => {
  const publisherId = req.user?.publisherId
  if (!publisherId) throw new AppError('Missing publisher context', 403)
  return publisherId
}

export async function list(req: Request, res: Response, next: NextFunction) {
  try {
    const publisherId = requirePublisherId(req)
    const apps = await appRepo.listByPublisher(publisherId)
    return res.json({ success: true, data: apps })
  } catch (e) {
    return next(e)
  }
}

export async function getById(req: Request, res: Response, next: NextFunction) {
  try {
    const publisherId = requirePublisherId(req)
    const { appId } = req.params
    if (!appId) throw new AppError('appId is required', 400)

    const app = await appRepo.getById(publisherId, appId)
    if (!app) throw new AppError('App not found', 404)

    return res.json({ success: true, data: app })
  } catch (e) {
    return next(e)
  }
}

export async function create(req: Request, res: Response, next: NextFunction) {
  try {
    const publisherId = requirePublisherId(req)
    const { name, bundleId, platform } = req.body

    if (!name || typeof name !== 'string' || name.trim().length === 0) {
      throw new AppError('name is required', 400)
    }
    if (!bundleId || typeof bundleId !== 'string' || bundleId.trim().length === 0) {
      throw new AppError('bundleId is required', 400)
    }
    if (!platform || !VALID_PLATFORMS.includes(platform)) {
      throw new AppError(`platform must be one of: ${VALID_PLATFORMS.join(', ')}`, 400)
    }

    const app = await appRepo.create(publisherId, {
      name: name.trim(),
      bundleId: bundleId.trim(),
      platform,
    })

    logger.info('App created', { publisherId, appId: app.id, name: app.name, platform: app.platform })
    return res.status(201).json({ success: true, data: app })
  } catch (e) {
    return next(e)
  }
}

export async function update(req: Request, res: Response, next: NextFunction) {
  try {
    const publisherId = requirePublisherId(req)
    const { appId } = req.params
    if (!appId) throw new AppError('appId is required', 400)

    const { name, bundleId } = req.body
    const updateInput: appRepo.UpdateAppInput = {}

    if (name !== undefined) {
      if (typeof name !== 'string' || name.trim().length === 0) {
        throw new AppError('name must be a non-empty string', 400)
      }
      updateInput.name = name.trim()
    }
    if (bundleId !== undefined) {
      if (typeof bundleId !== 'string' || bundleId.trim().length === 0) {
        throw new AppError('bundleId must be a non-empty string', 400)
      }
      updateInput.bundleId = bundleId.trim()
    }

    const app = await appRepo.update(publisherId, appId, updateInput)
    if (!app) throw new AppError('App not found', 404)

    logger.info('App updated', { publisherId, appId: app.id })
    return res.json({ success: true, data: app })
  } catch (e) {
    return next(e)
  }
}

export async function deleteApp(req: Request, res: Response, next: NextFunction) {
  try {
    const publisherId = requirePublisherId(req)
    const { appId } = req.params
    if (!appId) throw new AppError('appId is required', 400)

    const deleted = await appRepo.deleteApp(publisherId, appId)
    if (!deleted) throw new AppError('App not found', 404)

    logger.info('App deleted', { publisherId, appId })
    return res.status(204).send()
  } catch (e) {
    return next(e)
  }
}
