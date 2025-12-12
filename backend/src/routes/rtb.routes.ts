import { Router } from 'express';
import { appApiKeyAuth } from '../middleware/appApiKeyAuth';
import { requestBid, getStatus } from '../controllers/rtb.controller';
import { getCreative, trackClick, trackImpression } from '../controllers/rtbTracking.controller';
import { validate } from '../middleware/validation';
import { trackingRateLimiter } from '../middleware/trackingRateLimiter';
import { AuctionRequestSchema } from '../schemas/rtb';

const router = Router();

// Protected auction endpoints (SDK key auth for app-scoped access)
router.post('/bid', appApiKeyAuth, validate(AuctionRequestSchema), requestBid);
router.get('/status', appApiKeyAuth, getStatus);

// Public delivery and tracking endpoints (rate-limited)
router.get('/creative', getCreative);
router.get('/t/imp', trackingRateLimiter, trackImpression);
router.get('/t/click', trackingRateLimiter, trackClick);

export default router;
