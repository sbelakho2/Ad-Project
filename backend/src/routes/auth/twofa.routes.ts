import { Router } from 'express';
import * as twofaController from '../../controllers/twofa.controller';
import { authenticate } from '../../middleware/auth';

const router = Router();

// 2FA status check
router.get('/status', authenticate, twofaController.status);

// 2FA enrollment and verification
router.post('/enroll', authenticate, twofaController.enroll);
router.post('/verify', authenticate, twofaController.verify);

// 2FA backup codes and disable (require auth)
router.post('/backup-codes/regenerate', authenticate, twofaController.regenerateBackupCodes);
router.post('/disable', authenticate, twofaController.disable);

export default router;
