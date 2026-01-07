import { Router } from 'express';
import { authenticate } from '../middleware/auth';
import { requireFeature } from '../utils/featureFlags';
import { readOnlyRateLimit } from '../middleware/rateLimiting';
import { 
  getReconOverview, 
  getReconDeltas, 
  getReconDeltasCsv, 
  createDispute, 
  getMonthlyRevenueDigest,
  getReviewQueue,
  approveReviewItem,
  rejectReviewItem,
  listDisputes,
  getDisputeById,
  submitDisputeById,
  cancelDisputeById,
  getDisputeStats,
} from '../controllers/vra.controller';

const router = Router();

// Guard entire VRA surface by feature flag and auth
router.use(requireFeature('vraEnabled'));
router.use(authenticate);
router.use(readOnlyRateLimit);

// GET /api/v1/recon/overview
router.get('/recon/overview', getReconOverview);

// GET /api/v1/recon/deltas
router.get('/recon/deltas', getReconDeltas);

// GET /api/v1/recon/deltas.csv (streamed CSV)
router.get('/recon/deltas.csv', getReconDeltasCsv);

// POST /api/v1/recon/disputes - Create a new dispute
router.post('/recon/disputes', createDispute);

// GET /api/v1/recon/disputes - List all disputes
router.get('/recon/disputes', listDisputes);

// GET /api/v1/recon/disputes/stats - Get dispute statistics
router.get('/recon/disputes/stats', getDisputeStats);

// GET /api/v1/recon/disputes/:id - Get dispute by ID
router.get('/recon/disputes/:id', getDisputeById);

// POST /api/v1/recon/disputes/:id/submit - Submit a draft dispute
router.post('/recon/disputes/:id/submit', submitDisputeById);

// POST /api/v1/recon/disputes/:id/cancel - Cancel a dispute
router.post('/recon/disputes/:id/cancel', cancelDisputeById);

// GET /api/v1/recon/review-queue - Get mid-confidence matches needing review
router.get('/recon/review-queue', getReviewQueue);

// POST /api/v1/recon/review-queue/:id/approve - Approve a review item
router.post('/recon/review-queue/:id/approve', approveReviewItem);

// POST /api/v1/recon/review-queue/:id/reject - Reject a review item  
router.post('/recon/review-queue/:id/reject', rejectReviewItem);

// GET /api/v1/proofs/revenue_digest?month=YYYY-MM
router.get('/proofs/revenue_digest', getMonthlyRevenueDigest);

export default router;
