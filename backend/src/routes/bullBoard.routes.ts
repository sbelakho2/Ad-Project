/**
 * Bull Board Routes
 * 
 * Visual dashboard for monitoring BullMQ job queues
 * Accessible at /admin/queues (operator-only via parent admin routes)
 */

import { Router } from 'express';
import { createBullBoard } from '@bull-board/api';
import { BullMQAdapter } from '@bull-board/api/bullMQAdapter';
import { ExpressAdapter } from '@bull-board/express';
import { queueManager, QueueName } from '../queues/queueManager';
import logger from '../utils/logger';

const router = Router();

// Express adapter for Bull Board
const serverAdapter = new ExpressAdapter();
serverAdapter.setBasePath('/admin/queues');

/**
 * Initialize Bull Board with all available queues
 * Must be called after queueManager.initialize()
 */
export function initializeBullBoard(): void {
  if (!queueManager.isReady()) {
    logger.warn('[BullBoard] QueueManager not ready, skipping initialization');
    return;
  }

  const queueAdapters: BullMQAdapter[] = [];

  for (const queueName of Object.values(QueueName)) {
    const queue = queueManager.getQueue(queueName);
    if (queue) {
      queueAdapters.push(new BullMQAdapter(queue));
      logger.debug('[BullBoard] Added queue to dashboard', { queueName });
    }
  }

  createBullBoard({
    queues: queueAdapters,
    serverAdapter,
  });

  logger.info('[BullBoard] Dashboard initialized', {
    queueCount: queueAdapters.length,
    basePath: '/admin/queues',
  });
}

// Mount Bull Board UI
router.use('/', serverAdapter.getRouter());

export default router;
