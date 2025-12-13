import pino from 'pino';
import { config } from './config';
import * as db from './database';
import * as scheduler from './scheduler';
import { startServer, stopServer } from './api';

const logger = pino({ level: config.logging.level });

// Graceful shutdown handling
let isShuttingDown = false;

async function shutdown(signal: string): Promise<void> {
  if (isShuttingDown) {
    logger.warn('Shutdown already in progress');
    return;
  }
  
  isShuttingDown = true;
  logger.info({ signal }, 'Received shutdown signal');
  
  try {
    // Stop scheduler first
    scheduler.stopScheduler();
    
    // Stop API server
    await stopServer();
    
    // Close database pool
    await db.closePool();
    
    logger.info('Graceful shutdown complete');
    process.exit(0);
  } catch (error) {
    logger.error({ error }, 'Error during shutdown');
    process.exit(1);
  }
}

async function main(): Promise<void> {
  logger.info({
    version: '1.0.0',
    node: process.version,
    config: {
      apiPort: config.api.port,
      allowedIps: config.api.allowedIps,
      healthCheckInterval: config.scheduler.healthCheckIntervalSeconds,
    },
  }, 'Starting Autopilot service');
  
  try {
    // Run initial health check
    logger.info('Running initial health check');
    const healthResult = await db.runHealthCheck();
    logger.info({ state: healthResult.state }, 'Initial health check complete');
    
    // Ensure partitions exist
    logger.info('Ensuring partitions exist');
    await db.ensureFuturePartitions();
    logger.info('Partitions verified');
    
    // Start scheduler
    scheduler.initializeScheduler();
    
    // Start API server
    await startServer();
    
    logger.info('Autopilot service started successfully');
  } catch (error) {
    logger.error({ error }, 'Failed to start Autopilot service');
    process.exit(1);
  }
}

// Handle signals
process.on('SIGTERM', () => shutdown('SIGTERM'));
process.on('SIGINT', () => shutdown('SIGINT'));

// Handle uncaught errors
process.on('uncaughtException', (error) => {
  logger.fatal({ error }, 'Uncaught exception');
  shutdown('uncaughtException');
});

process.on('unhandledRejection', (reason, promise) => {
  logger.error({ reason, promise }, 'Unhandled rejection');
});

// Start the service
main();
