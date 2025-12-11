import { Request, Response, NextFunction } from 'express'
import * as appKeysRepo from '../repositories/appApiKeyRepository'
import AppError from '../utils/AppError'
import logger from '../utils/logger'

const requirePublisherId = (req: Request): string => {
  const publisherId = req.user?.publisherId
  if (!publisherId) throw new AppError('Missing publisher context', 403)
  return publisherId
}

export async function list(req: Request, res: Response, next: NextFunction) {
  try {
    const publisherId = requirePublisherId(req)
    const { appId } = req.params
    if (!appId) throw new AppError('appId is required', 400)

    const keys = await appKeysRepo.listByApp(publisherId, appId)
    return res.json({ success: true, data: { keys } })
  } catch (e) {
    return next(e)
  }
}

export async function create(req: Request, res: Response, next: NextFunction) {
  try {
    const publisherId = requirePublisherId(req)
    const { appId } = req.params
    if (!appId) throw new AppError('appId is required', 400)

    const live = Boolean(req.body?.live)
    const environment = (req.body?.environment || 'production') as any
    if (environment !== 'production' && environment !== 'staging') {
      throw new AppError('environment must be production or staging', 400)
    }

    const created = await appKeysRepo.createForApp(publisherId, appId, { live, environment })
    if (!created) throw new AppError('App not found for publisher', 404)

    logger.info('App API key created', { publisherId, appId, keyId: created.key.id, prefix: created.key.prefix })
    return res.status(201).json({ success: true, data: { ...created.key, secret: created.secret } })
  } catch (e) {
    return next(e)
  }
}

export async function rotate(req: Request, res: Response, next: NextFunction) {
  try {
    const publisherId = requirePublisherId(req)
    const { appId, id } = req.params as any
    if (!appId || !id) throw new AppError('appId and id are required', 400)

    const rotated = await appKeysRepo.rotate(publisherId, appId, id)
    if (!rotated) throw new AppError('Not found', 404)

    logger.info('App API key rotated', { publisherId, appId, oldKeyId: id, newKeyId: rotated.key.id })
    return res.json({ success: true, data: { ...rotated.key, secret: rotated.secret } })
  } catch (e) {
    return next(e)
  }
}

export async function revoke(req: Request, res: Response, next: NextFunction) {
  try {
    const publisherId = requirePublisherId(req)
    const { appId, id } = req.params as any
    if (!appId || !id) throw new AppError('appId and id are required', 400)

    const ok = await appKeysRepo.revoke(publisherId, appId, id)
    if (!ok) throw new AppError('Not found', 404)

    logger.info('App API key revoked', { publisherId, appId, keyId: id })
    return res.status(204).send()
  } catch (e) {
    return next(e)
  }
}
