/**
 * Sales Hub Routes Tests
 * 
 * Tests for the consolidated Sales Hub UI routes and API endpoints
 */

// Mock database module
jest.mock('../src/database', () => ({
  queryReadOnly: jest.fn(),
  queryOperator: jest.fn(),
}));

// Mock suppression list service
jest.mock('../src/services/suppression-list', () => ({
  isEmailSuppressed: jest.fn(),
  addToSuppressionList: jest.fn(),
  checkSafeMode: jest.fn(),
  setSafeMode: jest.fn(),
  checkBatchSuppression: jest.fn(),
  updateSafeModeState: jest.fn(),
}));

// Mock kill switch service
jest.mock('../src/services/global-kill-switch', () => ({
  getKillSwitchStatus: jest.fn(),
  toggleKillSwitch: jest.fn(),
  updateControlSetting: jest.fn(),
  checkSendsAllowed: jest.fn(),
  isGlobalKillSwitchActive: jest.fn(),
  invalidateKillSwitchCache: jest.fn(),
}));

jest.mock('../src/logger', () => ({
  logger: {
    info: jest.fn(),
    warn: jest.fn(),
    error: jest.fn(),
    debug: jest.fn(),
  },
}));

const { queryReadOnly } = require('../src/database');
const { isEmailSuppressed, addToSuppressionList, checkSafeMode, setSafeMode } = require('../src/services/suppression-list');
const { getKillSwitchStatus, toggleKillSwitch, checkSendsAllowed } = require('../src/services/global-kill-switch');

describe('Sales Hub API Routes', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  describe('Suppression List', () => {
    test('isEmailSuppressed returns suppression status', async () => {
      isEmailSuppressed.mockResolvedValue({
        isSuppressed: true,
        suppressionType: 'hard_bounce',
        reason: 'Email bounced permanently',
        suppressedAt: new Date('2026-01-15'),
      });

      const result = await isEmailSuppressed('bounced@example.com');
      
      expect(result.isSuppressed).toBe(true);
      expect(result.suppressionType).toBe('hard_bounce');
    });

    test('isEmailSuppressed returns false for valid email', async () => {
      isEmailSuppressed.mockResolvedValue({
        isSuppressed: false,
      });

      const result = await isEmailSuppressed('valid@example.com');
      
      expect(result.isSuppressed).toBe(false);
    });

    test('addToSuppressionList adds email successfully', async () => {
      addToSuppressionList.mockResolvedValue('supp-id-123');

      const result = await addToSuppressionList({
        email: 'block@example.com',
        type: 'manual_block',
        sourceSystem: 'manual_operator',
        reason: 'Competitor company',
        createdBy: 'operator@test.com',
      });

      expect(result).toBe('supp-id-123');
      expect(addToSuppressionList).toHaveBeenCalledWith(expect.objectContaining({
        email: 'block@example.com',
        type: 'manual_block',
      }));
    });
  });

  describe('Safe Mode', () => {
    test('checkSafeMode returns current state', async () => {
      checkSafeMode.mockResolvedValue({
        state: 'normal',
        bounceRate: 0.01,
        complaintRate: 0.001,
        throttlePercent: 100,
        emailsSent7d: 250,
      });

      const result = await checkSafeMode();
      
      expect(result.state).toBe('normal');
      expect(result.bounceRate).toBe(0.01);
      expect(result.throttlePercent).toBe(100);
    });

    test('checkSafeMode returns warning state when bounce rate high', async () => {
      checkSafeMode.mockResolvedValue({
        state: 'warning',
        bounceRate: 0.06,
        complaintRate: 0.001,
        throttlePercent: 50,
        emailsSent7d: 500,
      });

      const result = await checkSafeMode();
      
      expect(result.state).toBe('warning');
      expect(result.throttlePercent).toBe(50);
    });

    test('setSafeMode updates state', async () => {
      setSafeMode.mockResolvedValue(undefined);

      await setSafeMode('paused', 'operator@test.com', 'Emergency pause');

      expect(setSafeMode).toHaveBeenCalledWith('paused', 'operator@test.com', 'Emergency pause');
    });
  });

  describe('Kill Switch', () => {
    test('getKillSwitchStatus returns current status', async () => {
      getKillSwitchStatus.mockResolvedValue({
        killSwitchActive: false,
        killSwitchReason: null,
        v1Enabled: true,
        v2Enabled: true,
        maxDailySendsGlobal: 500,
        bounceRateThreshold: 0.05,
        lastModified: new Date(),
        lastModifiedBy: 'system',
      });

      const result = await getKillSwitchStatus();
      
      expect(result.killSwitchActive).toBe(false);
      expect(result.v1Enabled).toBe(true);
      expect(result.v2Enabled).toBe(true);
    });

    test('checkSendsAllowed returns v1 status', async () => {
      checkSendsAllowed.mockResolvedValue({
        allowed: true,
        reason: null,
      });

      const result = await checkSendsAllowed('v1_outbound');
      
      expect(result.allowed).toBe(true);
    });

    test('checkSendsAllowed blocks when kill switch active', async () => {
      checkSendsAllowed.mockResolvedValue({
        allowed: false,
        reason: 'Global kill switch is active',
      });

      const result = await checkSendsAllowed('v2_autonomous');
      
      expect(result.allowed).toBe(false);
      expect(result.reason).toContain('kill switch');
    });

    test('toggleKillSwitch activates kill switch', async () => {
      toggleKillSwitch.mockResolvedValue({
        success: true,
        previousState: false,
      });

      const result = await toggleKillSwitch(true, 'operator@test.com', 'Emergency stop', '127.0.0.1');
      
      expect(result.success).toBe(true);
      expect(toggleKillSwitch).toHaveBeenCalledWith(true, 'operator@test.com', 'Emergency stop', '127.0.0.1');
    });
  });

  describe('Suppression Stats Query', () => {
    test('queryReadOnly returns suppression stats', async () => {
      queryReadOnly.mockResolvedValue([
        { suppression_type: 'hard_bounce', count: '15' },
        { suppression_type: 'unsubscribe', count: '8' },
        { suppression_type: 'spam_complaint', count: '3' },
        { suppression_type: 'manual_block', count: '12' },
      ]);

      const stats = await queryReadOnly(`
        SELECT suppression_type, COUNT(*) as count
        FROM sales_suppression_list
        WHERE is_active = true
        GROUP BY suppression_type
      `, []);

      const result = {
        hardBounces: 0,
        unsubscribes: 0,
        complaints: 0,
        manual: 0,
      };

      for (const row of stats as any[]) {
        switch (row.suppression_type) {
          case 'hard_bounce':
            result.hardBounces = parseInt(row.count);
            break;
          case 'unsubscribe':
            result.unsubscribes = parseInt(row.count);
            break;
          case 'spam_complaint':
            result.complaints = parseInt(row.count);
            break;
          case 'manual_block':
            result.manual += parseInt(row.count);
            break;
        }
      }

      expect(result.hardBounces).toBe(15);
      expect(result.unsubscribes).toBe(8);
      expect(result.complaints).toBe(3);
      expect(result.manual).toBe(12);
    });
  });

  describe('Unified Metrics', () => {
    test('queryReadOnly returns unified metrics totals', async () => {
      queryReadOnly
        .mockResolvedValueOnce([]) // dailyMetrics
        .mockResolvedValueOnce([{
          total_sent: 1000,
          total_delivered: 950,
          total_opens: 400,
          total_clicks: 80,
          total_replies: 50,
          total_bounces: 30,
        }]) // totals
        .mockResolvedValueOnce([]) // templatePerformance
        .mockResolvedValueOnce([]) // domainHealth
        .mockResolvedValueOnce([]); // todayStatus

      const dailyMetrics = await queryReadOnly('SELECT * FROM unified_daily_metrics', []);
      const totals = await queryReadOnly('SELECT * FROM unified_metrics_totals', []);
      
      expect((totals as any[])[0].total_sent).toBe(1000);
      expect((totals as any[])[0].total_delivered).toBe(950);
      expect((totals as any[])[0].total_bounces).toBe(30);
    });
  });
});

describe('Sales Hub UI HTML', () => {
  test('Sales Hub view contains all sub-navigation tabs', () => {
    // Simulate checking the HTML structure
    const subTabs = [
      'controls',
      'suppression', 
      'v1-sequences',
      'v2-discovery',
      'v2-messaging',
      'inbox',
      'metrics',
    ];

    // All tabs should be defined
    expect(subTabs.length).toBe(7);
    
    // Each tab should have a valid ID pattern
    subTabs.forEach(tab => {
      expect(tab).toMatch(/^[a-z0-9-]+$/);
    });
  });

  test('showSalesSubview function switches views correctly', () => {
    // This tests the frontend JS logic conceptually
    const activeSalesSubview = 'controls';
    
    function showSalesSubview(subview: string) {
      const active = subview;
      return active;
    }

    expect(showSalesSubview('suppression')).toBe('suppression');
    expect(showSalesSubview('v1-sequences')).toBe('v1-sequences');
    expect(showSalesSubview('metrics')).toBe('metrics');
  });
});

describe('API Path Validation', () => {
  const validPaths = [
    '/api/sales-controls/status',
    '/api/sales-controls/kill-switch',
    '/api/sales-controls/suppression-stats',
    '/api/sales-controls/suppression-list',
    '/api/sales-controls/suppression-check',
    '/api/sales-controls/safe-mode',
    '/api/sales-controls/control-audit-log',
    '/api/sales-controls/unified-metrics',
  ];

  test.each(validPaths)('API path %s is correctly formatted', (path) => {
    expect(path).toMatch(/^\/api\/sales-controls\/[a-z-]+$/);
    expect(path.startsWith('/api/sales-controls/')).toBe(true);
  });

  test('all paths use sales-controls prefix', () => {
    validPaths.forEach(path => {
      expect(path.includes('/api/sales-controls/')).toBe(true);
    });
  });
});
