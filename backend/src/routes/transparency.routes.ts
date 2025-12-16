import { Router } from 'express';
import { authenticate } from '../middleware/auth';
import { getAuctions, getAuctionById, getAuctionSummary, getTransparencyKeys, getTransparencyKeysPublic, verifyAuction, getTransparencyMetrics } from '../controllers/transparency.controller';
import { readOnlyRateLimit, keysRateLimit } from '../middleware/rateLimiting';

const router = Router();

// Public keys endpoint (allows independent verification tooling to fetch signer keys)
// GET /api/v1/transparency/keys
router.get('/keys', keysRateLimit, getTransparencyKeysPublic);

// All other Transparency endpoints require auth and are publisher-scoped
router.use(authenticate);

// Apply generous read-only rate limiting to all authenticated transparency endpoints
router.use(readOnlyRateLimit);

// GET /api/v1/transparency/auctions
router.get('/auctions', getAuctions);

// GET /api/v1/transparency/auctions/:auction_id
router.get('/auctions/:auction_id', getAuctionById);

// GET /api/v1/transparency/summary/auctions
router.get('/summary/auctions', getAuctionSummary);

// GET /api/v1/transparency/auctions/:auction_id/verify
router.get('/auctions/:auction_id/verify', verifyAuction);

// GET /api/v1/transparency/metrics
router.get('/metrics', getTransparencyMetrics);

export default router;
