/**
 * Queue Routes
 * 
 * API endpoints for monitoring and managing job queues
 */

import { Router, Request, Response } from 'express';
import { queueManager, QueueName } from '../queues/queueManager';
import { authenticate } from '../middleware/auth';
import logger from '../utils/logger';

const router = Router();

/**
 * Get metrics for all queues
 * GET /api/v1/queues/metrics
 */
router.get('/metrics', authenticate, async (req: Request, res: Response) => {
  try {
  const metrics: Record<string, unknown> = {};

    for (const queueName of Object.values(QueueName)) {
      const queueMetrics = await queueManager.getQueueMetrics(queueName);
      if (queueMetrics) {
        metrics[queueName] = queueMetrics;
      }
    }

    res.json({
      success: true,
      data: metrics,
    });
  } catch (error) {
    logger.error('Failed to get queue metrics', { error });
    res.status(500).json({
      success: false,
      error: 'Failed to get queue metrics',
    });
  }
});

/**
 * Get metrics for a specific queue
 * GET /api/v1/queues/:queueName/metrics
 */
router.get('/:queueName/metrics', authenticate, async (req: Request, res: Response) => {
  try {
    const { queueName } = req.params;

    if (!Object.values(QueueName).includes(queueName as QueueName)) {
      return res.status(400).json({
        success: false,
        error: 'Invalid queue name',
      });
    }

    const metrics = await queueManager.getQueueMetrics(queueName as QueueName);

    if (!metrics) {
      return res.status(404).json({
        success: false,
        error: 'Queue not found',
      });
    }

    res.json({
      success: true,
      data: metrics,
    });
  } catch (error) {
    logger.error('Failed to get queue metrics', { error, queueName: req.params.queueName });
    res.status(500).json({
      success: false,
      error: 'Failed to get queue metrics',
    });
  }
});

/**
 * Add a job to a queue
 * POST /api/v1/queues/:queueName/jobs
 */
router.post('/:queueName/jobs', authenticate, async (req: Request, res: Response) => {
  try {
    const { queueName } = req.params;
    const { jobName, data, options } = req.body;

    if (!Object.values(QueueName).includes(queueName as QueueName)) {
      return res.status(400).json({
        success: false,
        error: 'Invalid queue name',
      });
    }

    if (!jobName || !data) {
      return res.status(400).json({
        success: false,
        error: 'Job name and data are required',
      });
    }

    const job = await queueManager.addJob(
      queueName as QueueName,
      jobName,
      data,
      options
    );

    if (!job) {
      return res.status(500).json({
        success: false,
        error: 'Failed to add job to queue',
      });
    }

    res.status(201).json({
      success: true,
      data: {
        jobId: job.id,
        queueName,
        jobName,
      },
    });
  } catch (error) {
    logger.error('Failed to add job to queue', { error, queueName: req.params.queueName });
    res.status(500).json({
      success: false,
      error: 'Failed to add job to queue',
    });
  }
});

/**
 * Get a specific job
 * GET /api/v1/queues/:queueName/jobs/:jobId
 */
router.get('/:queueName/jobs/:jobId', authenticate, async (req: Request, res: Response) => {
  try {
    const { queueName, jobId } = req.params;

    if (!Object.values(QueueName).includes(queueName as QueueName)) {
      return res.status(400).json({
        success: false,
        error: 'Invalid queue name',
      });
    }

    const job = await queueManager.getJob(queueName as QueueName, jobId);

    if (!job) {
      return res.status(404).json({
        success: false,
        error: 'Job not found',
      });
    }

    res.json({
      success: true,
      data: {
        id: job.id,
        name: job.name,
        data: job.data,
        progress: job.progress,
        returnvalue: job.returnvalue,
        finishedOn: job.finishedOn,
        processedOn: job.processedOn,
        failedReason: job.failedReason,
        attemptsMade: job.attemptsMade,
      },
    });
  } catch (error) {
    logger.error('Failed to get job', {
      error,
      queueName: req.params.queueName,
      jobId: req.params.jobId,
    });
    res.status(500).json({
      success: false,
      error: 'Failed to get job',
    });
  }
});

/**
 * Remove a job
 * DELETE /api/v1/queues/:queueName/jobs/:jobId
 */
router.delete('/:queueName/jobs/:jobId', authenticate, async (req: Request, res: Response) => {
  try {
    const { queueName, jobId } = req.params;

    if (!Object.values(QueueName).includes(queueName as QueueName)) {
      return res.status(400).json({
        success: false,
        error: 'Invalid queue name',
      });
    }

    const removed = await queueManager.removeJob(queueName as QueueName, jobId);

    if (!removed) {
      return res.status(404).json({
        success: false,
        error: 'Job not found',
      });
    }

    res.json({
      success: true,
      message: 'Job removed successfully',
    });
  } catch (error) {
    logger.error('Failed to remove job', {
      error,
      queueName: req.params.queueName,
      jobId: req.params.jobId,
    });
    res.status(500).json({
      success: false,
      error: 'Failed to remove job',
    });
  }
});

/**
 * Pause a queue
 * POST /api/v1/queues/:queueName/pause
 */
router.post('/:queueName/pause', authenticate, async (req: Request, res: Response) => {
  try {
    const { queueName } = req.params;

    if (!Object.values(QueueName).includes(queueName as QueueName)) {
      return res.status(400).json({
        success: false,
        error: 'Invalid queue name',
      });
    }

    await queueManager.pauseQueue(queueName as QueueName);

    res.json({
      success: true,
      message: 'Queue paused successfully',
    });
  } catch (error) {
    logger.error('Failed to pause queue', { error, queueName: req.params.queueName });
    res.status(500).json({
      success: false,
      error: 'Failed to pause queue',
    });
  }
});

/**
 * Resume a queue
 * POST /api/v1/queues/:queueName/resume
 */
router.post('/:queueName/resume', authenticate, async (req: Request, res: Response) => {
  try {
    const { queueName } = req.params;

    if (!Object.values(QueueName).includes(queueName as QueueName)) {
      return res.status(400).json({
        success: false,
        error: 'Invalid queue name',
      });
    }

    await queueManager.resumeQueue(queueName as QueueName);

    res.json({
      success: true,
      message: 'Queue resumed successfully',
    });
  } catch (error) {
    logger.error('Failed to resume queue', { error, queueName: req.params.queueName });
    res.status(500).json({
      success: false,
      error: 'Failed to resume queue',
    });
  }
});

/**
 * Clean a queue (remove old completed/failed jobs)
 * POST /api/v1/queues/:queueName/clean
 */
router.post('/:queueName/clean', authenticate, async (req: Request, res: Response) => {
  try {
    const { queueName } = req.params;
    const { gracePeriod } = req.body; // Grace period in seconds

    if (!Object.values(QueueName).includes(queueName as QueueName)) {
      return res.status(400).json({
        success: false,
        error: 'Invalid queue name',
      });
    }

    await queueManager.cleanQueue(
      queueName as QueueName,
      gracePeriod || 3600 // Default 1 hour
    );

    res.json({
      success: true,
      message: 'Queue cleaned successfully',
    });
  } catch (error) {
    logger.error('Failed to clean queue', { error, queueName: req.params.queueName });
    res.status(500).json({
      success: false,
      error: 'Failed to clean queue',
    });
  }
});

// ============================================
// DEAD LETTER QUEUE (DLQ) MANAGEMENT
// ============================================

/**
 * Get all Dead Letter Queue jobs
 * GET /api/v1/queues/dlq
 */
router.get('/dlq', authenticate, async (req: Request, res: Response) => {
  try {
    const limit = parseInt(req.query.limit as string) || 100;
    const offset = parseInt(req.query.offset as string) || 0;
    
    const jobs = await queueManager.getDeadLetterJobs(limit + offset);
    const paginatedJobs = jobs.slice(offset, offset + limit);
    
    res.json({
      success: true,
      data: paginatedJobs,
      meta: {
        total: jobs.length,
        limit,
        offset,
        hasMore: jobs.length > offset + limit,
      },
    });
  } catch (error) {
    logger.error('Failed to get DLQ jobs', { error });
    res.status(500).json({
      success: false,
      error: 'Failed to get dead letter queue jobs',
    });
  }
});

/**
 * Get DLQ metrics
 * GET /api/v1/queues/dlq/metrics
 */
router.get('/dlq/metrics', authenticate, async (req: Request, res: Response) => {
  try {
    const metrics = await queueManager.getQueueMetrics(QueueName.DEAD_LETTER);
    
    res.json({
      success: true,
      data: metrics || { waiting: 0, active: 0, completed: 0, failed: 0 },
    });
  } catch (error) {
    logger.error('Failed to get DLQ metrics', { error });
    res.status(500).json({
      success: false,
      error: 'Failed to get dead letter queue metrics',
    });
  }
});

/**
 * Retry a specific DLQ job (reprocess)
 * POST /api/v1/queues/dlq/:jobId/retry
 */
router.post('/dlq/:jobId/retry', authenticate, async (req: Request, res: Response) => {
  try {
    const { jobId } = req.params;
    
    const success = await queueManager.retryDeadLetterJob(jobId);
    
    if (!success) {
      return res.status(404).json({
        success: false,
        error: 'DLQ job not found or could not be retried',
      });
    }
    
    logger.info('DLQ job retried', { jobId, userId: (req as any).user?.id });
    
    res.json({
      success: true,
      message: 'Job requeued successfully',
    });
  } catch (error) {
    logger.error('Failed to retry DLQ job', { error, jobId: req.params.jobId });
    res.status(500).json({
      success: false,
      error: 'Failed to retry dead letter job',
    });
  }
});

/**
 * Bulk retry multiple DLQ jobs
 * POST /api/v1/queues/dlq/retry-bulk
 */
router.post('/dlq/retry-bulk', authenticate, async (req: Request, res: Response) => {
  try {
    const { jobIds, retryAll } = req.body;
    
    let jobsToRetry: string[] = [];
    
    if (retryAll) {
      const allJobs = await queueManager.getDeadLetterJobs(1000);
      jobsToRetry = allJobs.map((job: any) => job.id);
    } else if (Array.isArray(jobIds)) {
      jobsToRetry = jobIds;
    } else {
      return res.status(400).json({
        success: false,
        error: 'Either jobIds array or retryAll flag is required',
      });
    }
    
    let retried = 0;
    let failed = 0;
    const errors: Array<{ jobId: string; error: string }> = [];
    
    for (const jobId of jobsToRetry) {
      try {
        const success = await queueManager.retryDeadLetterJob(jobId);
        if (success) {
          retried++;
        } else {
          failed++;
          errors.push({ jobId, error: 'Job not found or could not be retried' });
        }
      } catch (e) {
        failed++;
        errors.push({ jobId, error: (e as Error).message });
      }
    }
    
    logger.info('DLQ bulk retry completed', { 
      retried, 
      failed, 
      total: jobsToRetry.length,
      userId: (req as any).user?.id,
    });
    
    res.json({
      success: true,
      data: { retried, failed, total: jobsToRetry.length },
      errors: errors.length > 0 ? errors : undefined,
    });
  } catch (error) {
    logger.error('Failed to bulk retry DLQ jobs', { error });
    res.status(500).json({
      success: false,
      error: 'Failed to bulk retry dead letter jobs',
    });
  }
});

/**
 * Remove (acknowledge/dismiss) a DLQ job
 * DELETE /api/v1/queues/dlq/:jobId
 */
router.delete('/dlq/:jobId', authenticate, async (req: Request, res: Response) => {
  try {
    const { jobId } = req.params;
    
    const removed = await queueManager.removeJob(QueueName.DEAD_LETTER, jobId);
    
    if (!removed) {
      return res.status(404).json({
        success: false,
        error: 'DLQ job not found',
      });
    }
    
    logger.info('DLQ job removed', { jobId, userId: (req as any).user?.id });
    
    res.json({
      success: true,
      message: 'DLQ job removed successfully',
    });
  } catch (error) {
    logger.error('Failed to remove DLQ job', { error, jobId: req.params.jobId });
    res.status(500).json({
      success: false,
      error: 'Failed to remove dead letter job',
    });
  }
});

export default router;
