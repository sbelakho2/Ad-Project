import Fastify, { FastifyInstance, FastifyRequest, FastifyReply } from 'fastify';
import cors from '@fastify/cors';
import pino from 'pino';
import { config } from './config';
import * as db from './database';
import * as scheduler from './scheduler';

const logger = pino({ level: config.logging.level });

// Create Fastify instance
const app: FastifyInstance = Fastify({
  logger: {
    level: config.logging.level,
  },
});

// IP whitelist middleware
app.addHook('onRequest', async (request: FastifyRequest, reply: FastifyReply) => {
  const clientIp = request.ip;
  const forwardedFor = request.headers['x-forwarded-for'];
  const realIp = typeof forwardedFor === 'string' ? forwardedFor.split(',')[0].trim() : clientIp;
  
  // Allow localhost for local testing
  const allowedIps = [...config.api.allowedIps, '127.0.0.1', '::1'];
  
  if (!allowedIps.includes(realIp)) {
    logger.warn({ ip: realIp, allowedIps }, 'Unauthorized access attempt');
    reply.code(403).send({ error: 'Forbidden' });
    return;
  }
});

// Register CORS
app.register(cors, {
  origin: config.api.allowedIps.map(ip => `http://${ip}`),
  methods: ['GET', 'POST'],
});

// ============================================================================
// Health & Status Endpoints
// ============================================================================

// GET /health - Basic health check
app.get('/health', async () => {
  return { status: 'ok', timestamp: new Date().toISOString() };
});

// GET /status - Autopilot status with health state
app.get('/status', async () => {
  try {
    const status = await db.getAutopilotStatus();
    return {
      ...status,
      uptime: process.uptime(),
      version: '1.0.0',
    };
  } catch (error) {
    return {
      state: 'FAIL',
      error: error instanceof Error ? error.message : String(error),
      uptime: process.uptime(),
      version: '1.0.0',
    };
  }
});

// POST /health/check - Run health check now
app.post('/health/check', async () => {
  const result = await db.runHealthCheck();
  return result;
});

// ============================================================================
// Metrics Endpoints
// ============================================================================

// GET /metrics - Database metrics
app.get('/metrics', async () => {
  const metrics = await db.getDatabaseMetrics();
  return metrics;
});

// GET /metrics/prometheus - Prometheus format metrics
app.get('/metrics/prometheus', async (request, reply) => {
  const metrics = await db.getDatabaseMetrics();
  const status = await db.getAutopilotStatus();
  
  let output = '# Autopilot Metrics\n';
  output += `autopilot_state{state="${status.state}"} 1\n`;
  output += `autopilot_connections_active ${metrics.connections.active}\n`;
  output += `autopilot_connections_idle ${metrics.connections.idle}\n`;
  output += `autopilot_connections_total ${metrics.connections.total}\n`;
  output += `autopilot_connections_max ${metrics.connections.max}\n`;
  output += `autopilot_connections_usage_pct ${metrics.connections.usage_pct}\n`;
  output += `autopilot_cache_hit_ratio ${metrics.performance.cache_hit_ratio}\n`;
  
  // Table metrics
  for (const table of metrics.tables) {
    const labels = `schema="${table.schema}",table="${table.name}"`;
    output += `autopilot_table_rows{${labels}} ${table.row_count}\n`;
    output += `autopilot_table_size_bytes{${labels}} ${table.size_bytes}\n`;
    output += `autopilot_table_dead_tuples{${labels}} ${table.dead_tuples}\n`;
  }
  
  // Partition metrics
  for (const partition of metrics.partitions) {
    output += `autopilot_partition_count{table="${partition.table}"} ${partition.partition_count}\n`;
  }
  
  reply.header('Content-Type', 'text/plain');
  return output;
});

// ============================================================================
// Jobs Endpoints
// ============================================================================

// GET /jobs - List recent jobs
app.get('/jobs', async (request: FastifyRequest<{ Querystring: { limit?: string } }>) => {
  const limit = parseInt(request.query.limit || '50', 10);
  const jobs = await db.getRecentJobs(limit);
  return { jobs, total: jobs.length };
});

// GET /jobs/types - List available job types
app.get('/jobs/types', async () => {
  return { types: scheduler.getAvailableJobs() };
});

// POST /jobs/trigger/:type - Trigger a job manually
app.post('/jobs/trigger/:type', async (
  request: FastifyRequest<{ Params: { type: string } }>,
  reply
) => {
  const jobType = request.params.type;
  
  if (!scheduler.getAvailableJobs().includes(jobType)) {
    reply.code(400);
    return { error: `Unknown job type: ${jobType}` };
  }
  
  const result = await scheduler.triggerJob(jobType);
  
  if (!result.success) {
    reply.code(500);
  }
  
  return result;
});

// ============================================================================
// Partition Endpoints
// ============================================================================

// GET /partitions - List partition info
app.get('/partitions', async () => {
  const metrics = await db.getDatabaseMetrics();
  return { partitions: metrics.partitions };
});

// POST /partitions/ensure - Ensure future partitions exist
app.post('/partitions/ensure', async () => {
  await db.ensureFuturePartitions();
  return { success: true, message: 'Future partitions ensured' };
});

// POST /partitions/cleanup - Run retention cleanup
app.post('/partitions/cleanup', async () => {
  const result = await db.applyRetentionPolicy();
  return { success: true, deleted: result };
});

// ============================================================================
// Aggregation Endpoints
// ============================================================================

// POST /aggregation/hourly - Run hourly aggregation
app.post('/aggregation/hourly', async (
  request: FastifyRequest<{ Body: { hour?: string } }>
) => {
  const hour = request.body?.hour ? new Date(request.body.hour) : undefined;
  const count = await db.computeHourlyMetrics(hour);
  return { success: true, rows_updated: count };
});

// POST /aggregation/daily - Run daily aggregation
app.post('/aggregation/daily', async (
  request: FastifyRequest<{ Body: { date?: string } }>
) => {
  const date = request.body?.date ? new Date(request.body.date) : undefined;
  const count = await db.computeDailyMetrics(date);
  return { success: true, rows_updated: count };
});

// ============================================================================
// Tables Endpoints
// ============================================================================

// GET /tables - List table stats
app.get('/tables', async () => {
  const metrics = await db.getDatabaseMetrics();
  return { tables: metrics.tables };
});

// GET /tables/:schema - List tables in schema
app.get('/tables/:schema', async (
  request: FastifyRequest<{ Params: { schema: string } }>
) => {
  const metrics = await db.getDatabaseMetrics();
  const filtered = metrics.tables.filter(t => t.schema === request.params.schema);
  return { schema: request.params.schema, tables: filtered };
});

// ============================================================================
// Safe Actions (whitelisted operations)
// ============================================================================

const SAFE_ACTIONS = [
  'vacuum_analyze',
  'reindex_table',
  'kill_idle_connections',
];

// GET /actions - List safe actions
app.get('/actions', async () => {
  return { actions: SAFE_ACTIONS };
});

// POST /actions/:action - Execute a safe action
app.post('/actions/:action', async (
  request: FastifyRequest<{ 
    Params: { action: string };
    Body: { table?: string; schema?: string };
  }>,
  reply
) => {
  const action = request.params.action;
  const { table, schema } = request.body || {};
  
  if (!SAFE_ACTIONS.includes(action)) {
    reply.code(400);
    return { error: `Unknown or unsafe action: ${action}` };
  }
  
  try {
    switch (action) {
      case 'vacuum_analyze':
        if (table && schema) {
          await db.query(`VACUUM ANALYZE ${schema}.${table}`);
          return { success: true, message: `VACUUM ANALYZE completed on ${schema}.${table}` };
        } else {
          await db.query('VACUUM ANALYZE');
          return { success: true, message: 'VACUUM ANALYZE completed on all tables' };
        }
      
      case 'reindex_table':
        if (!table || !schema) {
          reply.code(400);
          return { error: 'table and schema are required for reindex' };
        }
        await db.query(`REINDEX TABLE ${schema}.${table}`);
        return { success: true, message: `REINDEX completed on ${schema}.${table}` };
      
      case 'kill_idle_connections':
        const result = await db.query<{ count: number }>(`
          SELECT COUNT(*) as count FROM (
            SELECT pg_terminate_backend(pid)
            FROM pg_stat_activity
            WHERE state = 'idle'
            AND query_start < NOW() - INTERVAL '1 hour'
            AND datname = current_database()
            AND pid != pg_backend_pid()
          ) t
        `);
        return { success: true, killed: result[0]?.count || 0 };
      
      default:
        reply.code(400);
        return { error: `Unhandled action: ${action}` };
    }
  } catch (error) {
    reply.code(500);
    return { error: error instanceof Error ? error.message : String(error) };
  }
});

// ============================================================================
// Server Lifecycle
// ============================================================================

export async function startServer(): Promise<void> {
  try {
    await app.listen({
      host: config.api.host,
      port: config.api.port,
    });
    
    logger.info({ 
      host: config.api.host, 
      port: config.api.port,
      allowedIps: config.api.allowedIps,
    }, 'Autopilot API server started');
  } catch (error) {
    logger.error({ error }, 'Failed to start API server');
    throw error;
  }
}

export async function stopServer(): Promise<void> {
  await app.close();
  logger.info('API server stopped');
}

export { app };
