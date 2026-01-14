import request from 'supertest';
import express from 'express';
import router from '../src/routes';
import { 
  getVatReports, 
  generateAnnualReport, 
  generateTsdReport, 
  getComplianceStatus,
  COMPANY_INFO 
} from '../src/services/estonian-compliance';

// Mock auth middleware
jest.mock('../src/auth', () => ({
  requireOperator: (req: any, res: any, next: any) => next(),
  verifyOperatorCredentials: jest.fn(),
  issueOperatorToken: jest.fn(),
}));

// Mock logger
jest.mock('../src/logger', () => ({
  logger: {
    info: jest.fn(),
    error: jest.fn(),
    warn: jest.fn(),
  },
  logAudit: jest.fn(),
}));

// Mock database (needed because routes.ts imports other things that might use it)
jest.mock('../src/database', () => ({
  getReadOnlyPool: jest.fn(),
  getOperatorPool: jest.fn(),
}));

// Mock other adapters to avoid errors
jest.mock('../src/adapters/postgres-telemetry', () => ({
  getTopQueries: jest.fn(),
  getActiveSessions: jest.fn(),
  getBlockingSessions: jest.fn(),
  getLongTransactions: jest.fn(),
  getDatabaseStats: jest.fn(),
  getWALStats: jest.fn(),
  getTableMaintenanceStats: jest.fn(),
  getReplicationStatus: jest.fn(),
}));

jest.mock('../src/adapters/host-telemetry', () => ({
  getHostTelemetry: jest.fn(),
}));

jest.mock('../src/adapters/autopilot', () => ({
  getAutopilotHealth: jest.fn(),
  getMigrationStudioMetrics: jest.fn(),
  getVRAExperimentStatus: jest.fn(),
}));

jest.mock('../src/adapters/app-telemetry', () => ({
  getAppTelemetry: jest.fn(),
  getSlowEndpoints: jest.fn(),
  getSaturatedPools: jest.fn(),
  getHighErrorEndpoints: jest.fn(),
}));

jest.mock('../src/adapters/partition-manager', () => ({
  getAllPartitionStatus: jest.fn(),
  getPartitionStatus: jest.fn(),
  createFuturePartitions: jest.fn(),
  dropOldPartitions: jest.fn(),
  checkPartitionHealth: jest.fn(),
  autoMaintainPartitions: jest.fn(),
  getAllRetentionPolicies: jest.fn(),
}));

jest.mock('../src/adapters/action-gateway', () => ({
  captureIncidentSnapshot: jest.fn(),
  runDeepHealthcheck: jest.fn(),
  enterDegradedMode: jest.fn(),
  exitDegradedMode: jest.fn(),
  terminateSession: jest.fn(),
  dropOldPartition: jest.fn(),
}));

// Mock the compliance service
jest.mock('../src/services/estonian-compliance', () => ({
  getVatReports: jest.fn(),
  generateAnnualReport: jest.fn(),
  generateTsdReport: jest.fn(),
  getComplianceStatus: jest.fn(),
  COMPANY_INFO: { name: 'Mock Company' },
}));

const app = express();
app.use(express.json());
app.use(router);

describe('Compliance API Routes', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  describe('GET /api/v1/compliance/status', () => {
    it('should return compliance status', async () => {
      const mockStatus = {
        companyInfo: COMPANY_INFO,
        vatStatus: { pendingReports: 1, nextDeadline: '2026-01-20' },
        annualReportStatus: { lastSubmittedYear: 2024, nextDeadline: '2026-06-30' }
      };
      (getComplianceStatus as jest.Mock).mockResolvedValue(mockStatus);

      const res = await request(app).get('/api/v1/compliance/status');

      expect(res.status).toBe(200);
      expect(res.body).toEqual(mockStatus);
      expect(getComplianceStatus).toHaveBeenCalled();
    });
  });

  describe('GET /api/v1/compliance/vat-reports', () => {
    it('should return list of VAT reports', async () => {
      const mockReports = [{ id: '1', year: 2025, quarter: 4 }];
      (getVatReports as jest.Mock).mockResolvedValue(mockReports);

      const res = await request(app).get('/api/v1/compliance/vat-reports');

      expect(res.status).toBe(200);
      expect(res.body).toEqual(mockReports);
      expect(getVatReports).toHaveBeenCalled();
    });
  });

  describe('GET /api/v1/compliance/annual-report/:year', () => {
    it('should return annual report for valid year', async () => {
      const mockReport = { year: 2025, financials: { revenueEur: 1000 } };
      (generateAnnualReport as jest.Mock).mockResolvedValue(mockReport);

      const res = await request(app).get('/api/v1/compliance/annual-report/2025');

      expect(res.status).toBe(200);
      expect(res.body).toEqual(mockReport);
      expect(generateAnnualReport).toHaveBeenCalledWith(2025, undefined);
    });

    it('should return 400 for invalid year', async () => {
      const res = await request(app).get('/api/v1/compliance/annual-report/invalid');
      expect(res.status).toBe(400);
    });
  });

  describe('GET /api/v1/compliance/tsd-report/:year', () => {
    it('should return TSD report for valid year', async () => {
      const mockReport = { year: 2025, employees: 5 };
      (generateTsdReport as jest.Mock).mockResolvedValue(mockReport);

      const res = await request(app).get('/api/v1/compliance/tsd-report/2025');

      expect(res.status).toBe(200);
      expect(res.body).toEqual(mockReport);
      expect(generateTsdReport).toHaveBeenCalledWith(2025, undefined);
    });

    it('should handle month query param', async () => {
      const mockReport = { year: 2025, month: 1, employees: 5 };
      (generateTsdReport as jest.Mock).mockResolvedValue(mockReport);

      const res = await request(app).get('/api/v1/compliance/tsd-report/2025?month=1');

      expect(res.status).toBe(200);
      expect(res.body).toEqual(mockReport);
      expect(generateTsdReport).toHaveBeenCalledWith(2025, 1);
    });
  });
});
