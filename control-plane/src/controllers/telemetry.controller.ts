import { Request, Response } from 'express';
import { 
  getDatabaseStats, 
  getTableMaintenanceStats, 
  getReplicationStatus 
} from '../adapters/postgres-telemetry';
import { getHostTelemetryBundle } from '../adapters/host-telemetry';
import { getAppTelemetry } from '../adapters/app-telemetry';

export const getHealthSummary = async (req: Request, res: Response) => {
  try {
    const [dbStats, hostStats, appStats] = await Promise.all([
      getDatabaseStats(),
      getHostTelemetryBundle(),
      getAppTelemetry()
    ]);

    res.json({
      status: 'healthy',
      timestamp: new Date().toISOString(),
      database: dbStats,
      infrastructure: hostStats,
      application: appStats
    });
  } catch (err) {
    res.status(500).json({ status: 'degraded', error: (err as Error).message });
  }
};

export const getDatabaseMetrics = async (req: Request, res: Response) => {
  try {
    const stats = await getDatabaseStats();
    const maintenance = await getTableMaintenanceStats();
    const replication = await getReplicationStatus();
    
    res.json({ stats, maintenance, replication });
  } catch (err) {
    res.status(500).json({ error: 'Failed to fetch database metrics' });
  }
};
