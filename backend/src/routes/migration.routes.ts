/**
 * Migration Studio Routes
 */

import { Router } from 'express';
import multer from 'multer';
import {
  createExperiment,
  getExperiment,
  listExperiments,
  updateExperiment,
  activateExperiment,
  pauseExperiment,
  deleteExperiment,
  getAssignment,
  createImportJob,
  updateMapping,
  finalizeImport,
  evaluateGuardrails,
  generateReport,
  getSdkIntegration,
  completeMigration,
  rollbackMigration,
  getExperimentMappings,
} from '../controllers/migration.controller';
import { authenticate, authorize } from '../middleware/auth';

const router = Router();
const upload = multer({
  storage: multer.memoryStorage(),
  limits: {
    fileSize: 5 * 1024 * 1024, // 5MB
  },
});

// All routes require authentication (except /assign which has its own auth)
router.use((req, res, next) => {
  // Skip auth for /assign endpoint - it has its own SDK secret auth
  if (req.path === '/assign') {
    return next();
  }
  return authenticate(req, res, next);
});

// Experiment CRUD (admin/publisher only)
router.post('/experiments', authorize(['admin', 'publisher']), createExperiment);
router.get('/experiments', authorize(['admin', 'publisher', 'readonly']), listExperiments);
router.get('/experiments/:id', authorize(['admin', 'publisher', 'readonly']), getExperiment);
router.put('/experiments/:id', authorize(['admin', 'publisher']), updateExperiment);
router.delete('/experiments/:id', authorize(['admin', 'publisher']), deleteExperiment);

// Experiment lifecycle (admin/publisher only)
router.post('/experiments/:id/activate', authorize(['admin', 'publisher']), activateExperiment);
router.post('/experiments/:id/pause', authorize(['admin', 'publisher']), pauseExperiment);
router.post('/experiments/:id/complete', authorize(['admin', 'publisher']), completeMigration);
router.post('/experiments/:id/rollback', authorize(['admin', 'publisher']), rollbackMigration);
router.post(
  '/experiments/:id/guardrails/evaluate',
  authorize(['admin', 'publisher']),
  evaluateGuardrails
);

// SDK Integration & Mappings
router.get('/experiments/:id/sdk-integration', authorize(['admin', 'publisher', 'readonly']), getSdkIntegration);
router.get('/experiments/:id/mappings', authorize(['admin', 'publisher', 'readonly']), getExperimentMappings);

// Reports (readonly+ access)
router.get('/reports/:experimentId', authorize(['admin', 'publisher', 'readonly']), generateReport);

// Imports & mappings
router.post(
  '/import',
  authorize(['admin', 'publisher']),
  upload.single('file'),
  createImportJob
);
router.post(
  '/import/:id/finalize',
  authorize(['admin', 'publisher']),
  finalizeImport
);
router.put('/mappings/:id', authorize(['admin', 'publisher']), updateMapping);

// Assignment API (used by SDKs - has its own auth via shared secret)
router.post('/assign', getAssignment);

export default router;
