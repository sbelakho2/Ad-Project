/**
 * Tests for Global Kill Switch and Unified Events
 * 
 * These tests verify:
 * 1. Global kill switch blocks all sends when active
 * 2. Per-system controls work independently
 * 3. Unified events are recorded and deduplicated correctly
 * 4. Control changes are audit-logged
 */

import {
  isGlobalKillSwitchActive,
  checkSendsAllowed,
  toggleKillSwitch,
  updateControlSetting,
  getKillSwitchStatus,
  invalidateKillSwitchCache,
  getControlSettings,
} from '../src/services/global-kill-switch';

import {
  recordUnifiedEvent,
  recordUnifiedEventsBatch,
  getUnifiedMetricsSummary,
  getDomainEngagementPatterns,
} from '../src/services/unified-events';

// Mock the database module
jest.mock('../src/database', () => ({
  queryReadOnly: jest.fn(),
  queryOperator: jest.fn(),
}));

jest.mock('../src/logger', () => ({
  logger: {
    info: jest.fn(),
    warn: jest.fn(),
    error: jest.fn(),
    debug: jest.fn(),
  },
}));

const { queryReadOnly, queryOperator } = require('../src/database');

describe('Global Kill Switch', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    invalidateKillSwitchCache();
  });

  describe('isGlobalKillSwitchActive', () => {
    it('should return false when kill switch is inactive', async () => {
      queryReadOnly.mockResolvedValueOnce([{ control_value: 'false' }]);
      
      const result = await isGlobalKillSwitchActive();
      
      expect(result).toBe(false);
    });

    it('should return true when kill switch is active', async () => {
      queryReadOnly.mockResolvedValueOnce([{ control_value: 'true' }]);
      
      const result = await isGlobalKillSwitchActive();
      
      expect(result).toBe(true);
    });

    it('should return false when table does not exist', async () => {
      queryReadOnly.mockRejectedValueOnce({ code: '42P01' });
      
      const result = await isGlobalKillSwitchActive();
      
      expect(result).toBe(false);
    });

    it('should return false when no row found', async () => {
      queryReadOnly.mockResolvedValueOnce([]);
      
      const result = await isGlobalKillSwitchActive();
      
      expect(result).toBe(false);
    });
  });

  describe('checkSendsAllowed', () => {
    it('should block all sends when global kill switch is active', async () => {
      queryReadOnly.mockResolvedValueOnce([
        { control_key: 'global_kill_switch', control_value: 'true' },
        { control_key: 'v1_outbound_enabled', control_value: 'true' },
        { control_key: 'v2_autonomous_enabled', control_value: 'true' },
      ]);
      
      const result = await checkSendsAllowed('any');
      
      expect(result.allowed).toBe(false);
      expect(result.reason).toContain('GLOBAL KILL SWITCH ACTIVE');
    });

    it('should allow sends when kill switch is inactive', async () => {
      queryReadOnly.mockResolvedValueOnce([
        { control_key: 'global_kill_switch', control_value: 'false' },
        { control_key: 'v1_outbound_enabled', control_value: 'true' },
        { control_key: 'v2_autonomous_enabled', control_value: 'true' },
      ]);
      
      const result = await checkSendsAllowed('any');
      
      expect(result.allowed).toBe(true);
    });

    it('should block V1 sends when V1 is disabled but kill switch is off', async () => {
      queryReadOnly.mockResolvedValueOnce([
        { control_key: 'global_kill_switch', control_value: 'false' },
        { control_key: 'v1_outbound_enabled', control_value: 'false' },
        { control_key: 'v2_autonomous_enabled', control_value: 'true' },
      ]);
      
      const result = await checkSendsAllowed('v1_outbound');
      
      expect(result.allowed).toBe(false);
      expect(result.reason).toContain('V1 outbound');
    });

    it('should block V2 sends when V2 is disabled but kill switch is off', async () => {
      queryReadOnly.mockResolvedValueOnce([
        { control_key: 'global_kill_switch', control_value: 'false' },
        { control_key: 'v1_outbound_enabled', control_value: 'true' },
        { control_key: 'v2_autonomous_enabled', control_value: 'false' },
      ]);
      
      const result = await checkSendsAllowed('v2_autonomous');
      
      expect(result.allowed).toBe(false);
      expect(result.reason).toContain('V2 autonomous');
    });
  });

  describe('toggleKillSwitch', () => {
    it('should toggle kill switch and audit the change', async () => {
      // First call for isGlobalKillSwitchActive
      queryReadOnly.mockResolvedValueOnce([{ control_value: 'false' }]);
      // Second call for update_sales_control
      queryOperator.mockResolvedValueOnce([{ update_sales_control: true }]);
      
      const result = await toggleKillSwitch(true, 'test_user', 'Testing kill switch');
      
      expect(result.success).toBe(true);
      expect(result.previousState).toBe(false);
      expect(queryOperator).toHaveBeenCalledWith(
        expect.stringContaining('update_sales_control'),
        expect.arrayContaining(['true', 'test_user', 'Testing kill switch']),
        'test_user',
        expect.any(String)
      );
    });
  });

  describe('getControlSettings with cache', () => {
    it('should cache results', async () => {
      queryReadOnly.mockResolvedValue([
        { control_key: 'global_kill_switch', control_value: 'false' },
        { control_key: 'v1_outbound_enabled', control_value: 'true' },
        { control_key: 'v2_autonomous_enabled', control_value: 'true' },
        { control_key: 'max_daily_sends_global', control_value: '500' },
        { control_key: 'bounce_rate_threshold', control_value: '7' },
      ]);
      
      // First call should hit DB
      await getControlSettings();
      // Second call should use cache
      await getControlSettings();
      
      expect(queryReadOnly).toHaveBeenCalledTimes(1);
    });

    it('should refresh after invalidation', async () => {
      queryReadOnly.mockResolvedValue([
        { control_key: 'global_kill_switch', control_value: 'false' },
      ]);
      
      await getControlSettings();
      invalidateKillSwitchCache();
      await getControlSettings();
      
      expect(queryReadOnly).toHaveBeenCalledTimes(2);
    });
  });
});

describe('Unified Engagement Events', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  describe('recordUnifiedEvent', () => {
    it('should record event and return ID', async () => {
      queryOperator.mockResolvedValueOnce([{ id: 'event-123' }]);
      
      const result = await recordUnifiedEvent({
        sourceSystem: 'v1_outbound',
        eventType: 'opened',
        contactEmail: 'user@example.com',
        sourceMessageId: 'msg-456',
      });
      
      expect(result).toBe('event-123');
      expect(queryOperator).toHaveBeenCalledWith(
        expect.stringContaining('INSERT INTO unified_engagement_events'),
        expect.arrayContaining(['v1_outbound', 'msg-456', 'user@example.com', 'example.com', 'opened']),
        'unified-events',
        expect.any(String)
      );
    });

    it('should return null for deduplicated events', async () => {
      queryOperator.mockResolvedValueOnce([]);
      
      const result = await recordUnifiedEvent({
        sourceSystem: 'v1_outbound',
        eventType: 'opened',
        contactEmail: 'user@example.com',
      });
      
      expect(result).toBeNull();
    });

    it('should handle missing table gracefully', async () => {
      queryOperator.mockRejectedValueOnce({ code: '42P01' });
      
      const result = await recordUnifiedEvent({
        sourceSystem: 'v1_outbound',
        eventType: 'opened',
        contactEmail: 'user@example.com',
      });
      
      expect(result).toBeNull();
    });

    it('should extract domain from email', async () => {
      queryOperator.mockResolvedValueOnce([{ id: 'event-123' }]);
      
      await recordUnifiedEvent({
        sourceSystem: 'v2_autonomous',
        eventType: 'clicked',
        contactEmail: 'test@acme-corp.com',
      });
      
      // Check that domain was extracted
      expect(queryOperator).toHaveBeenCalledWith(
        expect.any(String),
        expect.arrayContaining(['acme-corp.com']),
        expect.any(String),
        expect.any(String)
      );
    });
  });

  describe('recordUnifiedEventsBatch', () => {
    it('should record multiple events', async () => {
      queryOperator
        .mockResolvedValueOnce([{ id: 'event-1' }])
        .mockResolvedValueOnce([{ id: 'event-2' }])
        .mockResolvedValueOnce([]);
      
      const result = await recordUnifiedEventsBatch([
        { sourceSystem: 'v1_outbound', eventType: 'sent', contactEmail: 'a@test.com' },
        { sourceSystem: 'v1_outbound', eventType: 'opened', contactEmail: 'b@test.com' },
        { sourceSystem: 'v2_autonomous', eventType: 'clicked', contactEmail: 'c@test.com' },
      ]);
      
      expect(result.recorded).toBe(2);
      expect(result.deduplicated).toBe(1);
      expect(result.errors).toBe(0);
    });
  });

  describe('getUnifiedMetricsSummary', () => {
    it('should return aggregated metrics', async () => {
      queryReadOnly.mockResolvedValueOnce([
        { source_system: 'v1_outbound', event_type: 'sent', count: '100' },
        { source_system: 'v1_outbound', event_type: 'opened', count: '40' },
        { source_system: 'v2_autonomous', event_type: 'sent', count: '50' },
        { source_system: 'v2_autonomous', event_type: 'opened', count: '25' },
        { source_system: 'v2_autonomous', event_type: 'replied', count: '5' },
      ]);
      
      const result = await getUnifiedMetricsSummary(30);
      
      expect(result.totalSent).toBe(150);
      expect(result.totalOpened).toBe(65);
      expect(result.totalReplied).toBe(5);
      expect(result.openRate).toBeCloseTo(43.33, 1);
      expect(result.bySystem['v1_outbound'].sent).toBe(100);
      expect(result.bySystem['v2_autonomous'].sent).toBe(50);
    });

    it('should handle missing table gracefully', async () => {
      queryReadOnly.mockRejectedValueOnce({ code: '42P01' });
      
      const result = await getUnifiedMetricsSummary();
      
      expect(result.totalSent).toBe(0);
      expect(result.openRate).toBe(0);
      expect(result.bySystem).toEqual({});
    });
  });

  describe('getDomainEngagementPatterns', () => {
    it('should return domain-specific patterns', async () => {
      // Basic metrics
      queryReadOnly
        .mockResolvedValueOnce([
          { event_type: 'sent', count: '20' },
          { event_type: 'opened', count: '10' },
          { event_type: 'clicked', count: '5' },
        ])
        // Template performance
        .mockResolvedValueOnce([
          { template_name: 'Cold Outreach v2', sent: '15', opened: '8' },
        ])
        // Day performance
        .mockResolvedValueOnce([
          { day_of_week: '2', sent: '10', opened: '6' },
        ]);
      
      const result = await getDomainEngagementPatterns('acme.com');
      
      expect(result.totalEmails).toBe(20);
      expect(result.totalOpens).toBe(10);
      expect(result.openRate).toBe(50);
      expect(result.bestPerformingTemplates).toHaveLength(1);
      expect(result.bestSendDays).toHaveLength(1);
    });
  });
});

describe('Integration: Kill Switch + Unified Events', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    invalidateKillSwitchCache();
  });

  it('should record event but block send when kill switch is active', async () => {
    // Simulate active kill switch
    queryReadOnly.mockResolvedValueOnce([
      { control_key: 'global_kill_switch', control_value: 'true' },
    ]);
    
    const sendCheck = await checkSendsAllowed('v1_outbound');
    expect(sendCheck.allowed).toBe(false);
    
    // But we can still record the attempt for auditing
    queryOperator.mockResolvedValueOnce([{ id: 'event-blocked' }]);
    
    const event = await recordUnifiedEvent({
      sourceSystem: 'v1_outbound',
      eventType: 'sent',
      contactEmail: 'user@example.com',
      eventMetadata: { blocked: true, reason: sendCheck.reason },
    });
    
    expect(event).toBe('event-blocked');
  });
});
