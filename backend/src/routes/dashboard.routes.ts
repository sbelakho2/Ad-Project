import { Router } from 'express';
import { getOverview, getKpis } from '../controllers/dashboard.controller';
import { authenticate } from '../middleware/auth';

const router = Router();

// All dashboard routes require authentication
router.use(authenticate);

// GET /api/v1/dashboard/overview
router.get('/overview', getOverview);

// GET /api/v1/dashboard/kpis
router.get('/kpis', getKpis);

export default router;
