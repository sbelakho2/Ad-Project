import fs from 'node:fs'
import path from 'node:path'
import { z } from 'zod'
import logger from '../utils/logger'
import { SupplyChainCorpus } from './fraud/weakSupervision/supplyChainCorpus'

const ManifestSchema = z.object({
  supplyChain: z.object({
    appAds: z.string().min(1),
    sellers: z.string().min(1),
  }),
})

const DEFAULT_BASE_DIR = path.resolve(process.cwd(), 'data', 'weak-supervision')

export type SupplyChainStatusRequest = {
  domain: string
  sellerId?: string
  appStoreId?: string
  siteId?: string
}

export type SupplyChainStatus = {
  domain: string
  sellerId?: string
  appStoreId?: string
  siteId?: string
  authorized: boolean
  reason?: string
  sellerInfo?: { sellerId: string; domain?: string; name?: string; status?: 'active' | 'inactive' }
  entries?: Array<{ sellerId: string; relationship?: string; appStoreId?: string; siteId?: string }>
}

function resolvePaths(baseDir: string = DEFAULT_BASE_DIR) {
  const manifestPath = path.resolve(baseDir, 'manifest.json')
  const manifestRaw = fs.readFileSync(manifestPath, 'utf8')
  const manifest = ManifestSchema.parse(JSON.parse(manifestRaw))
  return {
    appAdsPath: path.resolve(baseDir, manifest.supplyChain.appAds),
    sellersPath: path.resolve(baseDir, manifest.supplyChain.sellers),
  }
}

export async function getSupplyChainStatus(
  req: SupplyChainStatusRequest,
  options?: { baseDir?: string }
): Promise<SupplyChainStatus> {
  const domain = req.domain.trim().toLowerCase()
  if (!domain) {
    throw new Error('domain is required')
  }

  const baseDir = options?.baseDir ?? DEFAULT_BASE_DIR
  let appAdsPath: string
  let sellersPath: string
  try {
    ;({ appAdsPath, sellersPath } = resolvePaths(baseDir))
  } catch (err: any) {
    const code = err?.code
    logger.error('[SupplyChain] Corpus manifest unavailable', {
      baseDir,
      code,
      message: err?.message,
    })
    return {
      domain,
      sellerId: req.sellerId,
      appStoreId: req.appStoreId,
      siteId: req.siteId,
      authorized: false,
      reason: 'Supply chain corpus is not available on this server',
    }
  }

  const corpus = new SupplyChainCorpus(appAdsPath, sellersPath)
  try {
    await corpus.load()
  } catch (err: any) {
    logger.error('[SupplyChain] Failed to load corpus', {
      appAdsPath,
      sellersPath,
      code: err?.code,
      message: err?.message,
    })
    return {
      domain,
      sellerId: req.sellerId,
      appStoreId: req.appStoreId,
      siteId: req.siteId,
      authorized: false,
      reason: 'Supply chain corpus could not be loaded',
    }
  }

  const entries = corpus.listDomainEntries(domain)
  const result = corpus.evaluateAuthorization({
    domain,
    sellerId: req.sellerId ?? '',
    appStoreId: req.appStoreId,
    siteId: req.siteId,
  })

  if (!result.authorized) {
    logger.warn('[SupplyChain] Unauthorized or missing seller', {
      domain,
      sellerId: req.sellerId,
      reason: result.reason,
    })
  } else {
    logger.info('[SupplyChain] Seller authorized', { domain, sellerId: req.sellerId })
  }

  return {
    domain,
    sellerId: req.sellerId,
    appStoreId: req.appStoreId,
    siteId: req.siteId,
    authorized: result.authorized,
    reason: result.reason,
    sellerInfo: result.sellerInfo,
    entries,
  }
}
