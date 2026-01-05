import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { getSupplyChainStatus } from '../supplyChainStatusService'

async function setupCorpus(opts?: {
  appAds?: Record<string, Array<{ sellerId: string; relationship?: string; appStoreId?: string; siteId?: string }>>
  sellers?: Record<string, { sellerId: string; domain?: string; name?: string; status?: 'active' | 'inactive' }>
}) {
  const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), 'supply-chain-status-'))
  const baseDir = path.join(tempRoot, 'data', 'weak-supervision')
  const supplyDir = path.join(baseDir, 'supply-chain')
  await fs.mkdir(supplyDir, { recursive: true })

  const manifest = {
    supplyChain: {
      appAds: 'supply-chain/app-ads.json',
      sellers: 'supply-chain/sellers.json',
    },
    syntheticScenarios: 'synthetic-scenarios.json',
  }

  const appAds =
    opts?.appAds ?? {
      'example.com': [
        { sellerId: 'pub-1', relationship: 'DIRECT', appStoreId: 'com.example.app' },
        { sellerId: 'pub-2', relationship: 'RESELLER', appStoreId: 'com.example.app' },
      ],
      'video.test': [{ sellerId: 'pub-3', relationship: 'DIRECT', siteId: 'video_site' }],
    }

  const sellers =
    opts?.sellers ?? {
      'pub-1': { sellerId: 'pub-1', domain: 'ssp.example.com', name: 'Example Seller' },
      'pub-2': { sellerId: 'pub-2', domain: 'reseller.example.com', name: 'Reseller Partner' },
      'pub-9': { sellerId: 'pub-9', domain: 'rogue.example', name: 'Rogue Seller' },
    }

  await fs.writeFile(path.join(baseDir, 'manifest.json'), JSON.stringify(manifest), 'utf8')
  await fs.writeFile(path.join(supplyDir, 'app-ads.json'), JSON.stringify(appAds), 'utf8')
  await fs.writeFile(path.join(supplyDir, 'sellers.json'), JSON.stringify(sellers), 'utf8')

  const cleanup = async () => fs.rm(tempRoot, { recursive: true, force: true })

  return { baseDir, cleanup }
}

describe('getSupplyChainStatus', () => {
  test('does not throw when corpus manifest is missing', async () => {
    const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), 'supply-chain-status-missing-'))
    const baseDir = path.join(tempRoot, 'data', 'weak-supervision')
    await fs.mkdir(baseDir, { recursive: true })

    try {
      const status = await getSupplyChainStatus({ domain: 'example.com', sellerId: 'pub-1' }, { baseDir })

      expect(status.authorized).toBe(false)
      expect(status.reason).toMatch(/corpus is not available|could not be loaded/i)
    } finally {
      await fs.rm(tempRoot, { recursive: true, force: true })
    }
  })

  test('returns authorized status with seller info and entries', async () => {
    const { baseDir, cleanup } = await setupCorpus()
    try {
      const status = await getSupplyChainStatus(
        { domain: 'example.com', sellerId: 'pub-1', appStoreId: 'com.example.app' },
        { baseDir }
      )

      expect(status.authorized).toBe(true)
      expect(status.sellerInfo).toMatchObject({ sellerId: 'pub-1', name: 'Example Seller' })
      expect(status.entries?.[0]).toMatchObject({ sellerId: 'pub-1', appStoreId: 'com.example.app' })
    } finally {
      await cleanup()
    }
  })

  test('flags missing domain in corpus', async () => {
    const { baseDir, cleanup } = await setupCorpus()
    try {
      const status = await getSupplyChainStatus({ domain: 'unknown.example', sellerId: 'pub-1' }, { baseDir })

      expect(status.authorized).toBe(false)
      expect(status.reason).toMatch(/missing from app-ads/i)
      expect(status.entries).toBeUndefined()
    } finally {
      await cleanup()
    }
  })

  test('flags undeclared seller but surfaces directory info', async () => {
    const { baseDir, cleanup } = await setupCorpus()
    try {
      const status = await getSupplyChainStatus({ domain: 'example.com', sellerId: 'pub-9' }, { baseDir })

      expect(status.authorized).toBe(false)
      expect(status.reason).toMatch(/not declared/i)
      expect(status.sellerInfo).toMatchObject({ sellerId: 'pub-9', name: 'Rogue Seller' })
      expect(status.entries).toHaveLength(2)
    } finally {
      await cleanup()
    }
  })
})
