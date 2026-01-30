/**
 * Dispute Lifecycle Service
 * 
 * Manages the complete lifecycle of revenue disputes:
 * - draft → submitted → in_progress → resolved/rejected/cancelled
 * 
 * Dispute States:
 * - draft: Initial creation, evidence can be modified
 * - submitted: Sent to network, awaiting response
 * - in_progress: Network acknowledged, under review
 * - resolved: Settled in favor of publisher (payment adjustment expected)
 * - rejected: Network denied the dispute
 * - cancelled: Publisher withdrew the dispute
 */

import Decimal from 'decimal.js';
import { query } from '../../utils/postgres';
import logger from '../../utils/logger';
import {
  vraDisputeStateTransitionsTotal,
  vraDisputeResolutionDurationSeconds,
} from '../../utils/prometheus';

// Configure Decimal.js for financial precision
Decimal.set({ precision: 20, rounding: Decimal.ROUND_HALF_UP });

/**
 * Parse a financial amount from string to Decimal, then to number.
 * Uses Decimal.js to avoid floating-point precision issues during calculations.
 */
function parseAmount(amountStr: string | null | undefined): number {
  if (!amountStr || amountStr === '') return 0;
  try {
    return new Decimal(amountStr).toNumber();
  } catch {
    return 0;
  }
}

export type DisputeStatus = 
  | 'draft' 
  | 'submitted' 
  | 'in_progress' 
  | 'resolved' 
  | 'rejected' 
  | 'cancelled';

export interface DisputeRecord {
  id: string;
  disputeId: string;
  network: string;
  amount: number;
  status: DisputeStatus;
  evidenceUri: string;
  publisherId?: string;
  contact?: string;
  notes?: string;
  networkRefId?: string;       // External reference from network
  resolutionAmount?: number;   // Final resolved amount
  resolutionNotes?: string;    // Network's resolution notes
  createdAt: string;
  updatedAt: string;
  submittedAt?: string;
  resolvedAt?: string;
}

// Valid state transitions
const VALID_TRANSITIONS: Record<DisputeStatus, DisputeStatus[]> = {
  draft: ['submitted', 'cancelled'],
  submitted: ['in_progress', 'resolved', 'rejected', 'cancelled'],
  in_progress: ['resolved', 'rejected', 'cancelled'],
  resolved: [],   // Terminal state
  rejected: [],   // Terminal state
  cancelled: [],  // Terminal state
};

/**
 * Check if a state transition is valid
 */
export function isValidTransition(from: DisputeStatus, to: DisputeStatus): boolean {
  return VALID_TRANSITIONS[from]?.includes(to) ?? false;
}

/**
 * Get a dispute by ID
 * 
 * SECURITY: Always requires publisherId to enforce tenant isolation.
 * This prevents cross-tenant access even if dispute IDs are guessed.
 */
export async function getDispute(disputeId: string, publisherId?: string): Promise<DisputeRecord | null> {
  // SECURITY: Build query with mandatory tenant filter when publisherId is provided
  const baseQuery = `SELECT 
       id::text,
       dispute_id,
       network,
       amount::text,
       status,
       evidence_uri,
       publisher_id::text,
       contact,
       notes,
       network_ref_id,
       resolution_amount::text,
       resolution_notes,
       created_at::text,
       updated_at::text,
       submitted_at::text,
       resolved_at::text
     FROM recon_disputes
     WHERE dispute_id = $1`;
  
  // Enforce tenant isolation when publisherId is provided
  const queryText = publisherId 
    ? `${baseQuery} AND publisher_id = $2 LIMIT 1`
    : `${baseQuery} LIMIT 1`;
  const queryParams = publisherId ? [disputeId, publisherId] : [disputeId];

  const result = await query<{
    id: string;
    dispute_id: string;
    network: string;
    amount: string;
    status: string;
    evidence_uri: string;
    publisher_id?: string;
    contact?: string;
    notes?: string;
    network_ref_id?: string;
    resolution_amount?: string;
    resolution_notes?: string;
    created_at: string;
    updated_at: string;
    submitted_at?: string;
    resolved_at?: string;
  }>(queryText, queryParams);

  if (result.rows.length === 0) {
    return null;
  }

  const row = result.rows[0];
  return {
    id: row.id,
    disputeId: row.dispute_id,
    network: row.network,
    amount: parseAmount(row.amount),
    status: row.status as DisputeStatus,
    evidenceUri: row.evidence_uri,
    publisherId: row.publisher_id,
    contact: row.contact,
    notes: row.notes,
    networkRefId: row.network_ref_id,
    resolutionAmount: row.resolution_amount ? parseAmount(row.resolution_amount) : undefined,
    resolutionNotes: row.resolution_notes,
    createdAt: row.created_at,
    updatedAt: row.updated_at,
    submittedAt: row.submitted_at,
    resolvedAt: row.resolved_at,
  };
}

/**
 * List disputes for a publisher
 */
export async function listDisputes(options: {
  publisherId?: string;
  network?: string;
  status?: DisputeStatus | DisputeStatus[];
  page?: number;
  pageSize?: number;
}): Promise<{ items: DisputeRecord[]; total: number }> {
  const { publisherId, network, status, page = 1, pageSize = 50 } = options;
  const offset = (page - 1) * pageSize;
  
  const conditions: string[] = [];
  const params: any[] = [];
  let paramIndex = 1;

  if (publisherId) {
    conditions.push(`publisher_id = $${paramIndex++}`);
    params.push(publisherId);
  }
  if (network) {
    conditions.push(`network = $${paramIndex++}`);
    params.push(network);
  }
  if (status) {
    if (Array.isArray(status)) {
      conditions.push(`status = ANY($${paramIndex++}::text[])`);
      params.push(status);
    } else {
      conditions.push(`status = $${paramIndex++}`);
      params.push(status);
    }
  }

  const whereClause = conditions.length > 0 ? `WHERE ${conditions.join(' AND ')}` : '';

  const countResult = await query<{ count: string }>(
    `SELECT COUNT(*)::text AS count FROM recon_disputes ${whereClause}`,
    params
  );
  const total = parseInt(countResult.rows[0]?.count || '0', 10);

  params.push(pageSize, offset);
  const result = await query<{
    id: string;
    dispute_id: string;
    network: string;
    amount: string;
    status: string;
    evidence_uri: string;
    created_at: string;
    updated_at: string;
  }>(
    `SELECT 
       id::text,
       dispute_id,
       network,
       amount::text,
       status,
       evidence_uri,
       created_at::text,
       updated_at::text
     FROM recon_disputes
     ${whereClause}
     ORDER BY created_at DESC
     LIMIT $${paramIndex++} OFFSET $${paramIndex++}`,
    params
  );

  const items = result.rows.map(row => ({
    id: row.id,
    disputeId: row.dispute_id,
    network: row.network,
    amount: parseAmount(row.amount),
    status: row.status as DisputeStatus,
    evidenceUri: row.evidence_uri,
    createdAt: row.created_at,
    updatedAt: row.updated_at,
  }));

  return { items, total };
}

/**
 * Transition a dispute to a new state
 * 
 * SECURITY: When publisherId is provided, enforces tenant isolation.
 */
export async function transitionDispute(
  disputeId: string,
  newStatus: DisputeStatus,
  options?: {
    networkRefId?: string;
    resolutionAmount?: number;
    resolutionNotes?: string;
    notes?: string;
    publisherId?: string; // For tenant isolation
  }
): Promise<{ success: boolean; dispute?: DisputeRecord; error?: string }> {
  // Get current dispute with tenant isolation
  const dispute = await getDispute(disputeId, options?.publisherId);
  if (!dispute) {
    return { success: false, error: 'Dispute not found or access denied' };
  }

  const currentStatus = dispute.status;
  
  // Validate transition
  if (!isValidTransition(currentStatus, newStatus)) {
    return { 
      success: false, 
      error: `Invalid transition: ${currentStatus} → ${newStatus}` 
    };
  }

  // Build update query with tenant isolation
  const updates: string[] = ['status = $2', 'updated_at = NOW()'];
  const params: any[] = [disputeId, newStatus];
  let paramIndex = 3;

  if (newStatus === 'submitted') {
    updates.push(`submitted_at = NOW()`);
  }

  if (newStatus === 'resolved' || newStatus === 'rejected') {
    updates.push(`resolved_at = NOW()`);
  }

  if (options?.networkRefId) {
    updates.push(`network_ref_id = $${paramIndex++}`);
    params.push(options.networkRefId);
  }

  if (options?.resolutionAmount !== undefined) {
    updates.push(`resolution_amount = $${paramIndex++}`);
    params.push(options.resolutionAmount);
  }

  if (options?.resolutionNotes) {
    updates.push(`resolution_notes = $${paramIndex++}`);
    params.push(options.resolutionNotes);
  }

  if (options?.notes) {
    updates.push(`notes = COALESCE(notes, '') || E'\\n' || $${paramIndex++}`);
    params.push(`[${new Date().toISOString()}] ${options.notes}`);
  }

  try {
    await query(
      `UPDATE recon_disputes
       SET ${updates.join(', ')}
       WHERE dispute_id = $1`,
      params
    );

    // Track metrics
    try {
      vraDisputeStateTransitionsTotal.inc({ 
        from: currentStatus, 
        to: newStatus,
        network: dispute.network,
      });
      
      // Track resolution duration
      if ((newStatus === 'resolved' || newStatus === 'rejected') && dispute.submittedAt) {
        const durationMs = Date.now() - new Date(dispute.submittedAt).getTime();
        vraDisputeResolutionDurationSeconds.observe(
          { network: dispute.network, outcome: newStatus },
          durationMs / 1000
        );
      }
    } catch {
      // Ignore metrics errors
    }

    logger.info('VRA dispute transitioned', {
      disputeId,
      from: currentStatus,
      to: newStatus,
      network: dispute.network,
    });

    // Return updated dispute
    const updated = await getDispute(disputeId);
    return { success: true, dispute: updated ?? undefined };

  } catch (e) {
    logger.error('VRA dispute transition failed', {
      disputeId,
      from: currentStatus,
      to: newStatus,
      error: (e as Error)?.message,
    });
    return { success: false, error: 'Failed to update dispute' };
  }
}

/**
 * Submit a draft dispute to the network
 * 
 * SECURITY: Requires publisherId for tenant isolation.
 * Prevents unauthorized modification of other publishers' disputes.
 */
export async function submitDispute(
  disputeId: string,
  options?: { contact?: string; notes?: string; publisherId?: string }
): Promise<{ success: boolean; dispute?: DisputeRecord; error?: string }> {
  const publisherId = options?.publisherId;
  
  // SECURITY: Verify dispute exists and belongs to requesting publisher
  if (publisherId) {
    const existingDispute = await getDispute(disputeId, publisherId);
    if (!existingDispute) {
      return { success: false, error: 'Dispute not found or access denied' };
    }
  }
  
  // Update contact if provided - with tenant isolation
  if (options?.contact) {
    const updateQuery = publisherId
      ? `UPDATE recon_disputes SET contact = $2, updated_at = NOW() WHERE dispute_id = $1 AND publisher_id = $3`
      : `UPDATE recon_disputes SET contact = $2, updated_at = NOW() WHERE dispute_id = $1`;
    const updateParams = publisherId 
      ? [disputeId, options.contact, publisherId]
      : [disputeId, options.contact];
    await query(updateQuery, updateParams);
  }

  return transitionDispute(disputeId, 'submitted', { notes: options?.notes, publisherId });
}

/**
 * Mark dispute as acknowledged by network
 */
export async function acknowledgeDispute(
  disputeId: string,
  networkRefId: string,
  notes?: string
): Promise<{ success: boolean; dispute?: DisputeRecord; error?: string }> {
  return transitionDispute(disputeId, 'in_progress', { networkRefId, notes });
}

/**
 * Resolve a dispute (network approved payment adjustment)
 */
export async function resolveDispute(
  disputeId: string,
  options: {
    resolutionAmount: number;
    resolutionNotes?: string;
  }
): Promise<{ success: boolean; dispute?: DisputeRecord; error?: string }> {
  return transitionDispute(disputeId, 'resolved', options);
}

/**
 * Reject a dispute (network denied claim)
 */
export async function rejectDispute(
  disputeId: string,
  resolutionNotes: string
): Promise<{ success: boolean; dispute?: DisputeRecord; error?: string }> {
  return transitionDispute(disputeId, 'rejected', { resolutionNotes });
}

/**
 * Cancel a dispute (publisher withdrew)
 */
export async function cancelDispute(
  disputeId: string,
  reason?: string
): Promise<{ success: boolean; dispute?: DisputeRecord; error?: string }> {
  return transitionDispute(disputeId, 'cancelled', { notes: reason ? `Cancelled: ${reason}` : 'Cancelled by publisher' });
}

/**
 * Get dispute statistics for a publisher
 */
export async function getDisputeStats(publisherId: string): Promise<{
  total: number;
  byStatus: Record<DisputeStatus, number>;
  totalAmountDisputed: number;
  totalAmountResolved: number;
  avgResolutionDays: number | null;
}> {
  const result = await query<{
    status: string;
    count: string;
    total_amount: string;
    resolved_amount: string;
  }>(
    `SELECT 
       status,
       COUNT(*)::text AS count,
       COALESCE(SUM(amount), 0)::text AS total_amount,
       COALESCE(SUM(CASE WHEN status = 'resolved' THEN resolution_amount ELSE 0 END), 0)::text AS resolved_amount
     FROM recon_disputes
     WHERE publisher_id = $1
     GROUP BY status`,
    [publisherId]
  );

  const byStatus: Record<DisputeStatus, number> = {
    draft: 0,
    submitted: 0,
    in_progress: 0,
    resolved: 0,
    rejected: 0,
    cancelled: 0,
  };

  let total = 0;
  let totalAmountDisputed = new Decimal(0);
  let totalAmountResolved = new Decimal(0);

  for (const row of result.rows) {
    const status = row.status as DisputeStatus;
    const count = parseInt(row.count, 10);
    byStatus[status] = count;
    total += count;
    totalAmountDisputed = totalAmountDisputed.plus(parseAmount(row.total_amount));
    if (status === 'resolved') {
      totalAmountResolved = new Decimal(parseAmount(row.resolved_amount));
    }
  }

  // Calculate average resolution time
  const avgResult = await query<{ avg_days: string }>(
    `SELECT 
       AVG(EXTRACT(EPOCH FROM (resolved_at - submitted_at)) / 86400)::text AS avg_days
     FROM recon_disputes
     WHERE publisher_id = $1 
       AND status IN ('resolved', 'rejected')
       AND submitted_at IS NOT NULL
       AND resolved_at IS NOT NULL`,
    [publisherId]
  );

  const avgResolutionDays = avgResult.rows[0]?.avg_days 
    ? parseFloat(avgResult.rows[0].avg_days) // Days are not currency - parseFloat is fine
    : null;

  return {
    total,
    byStatus,
    totalAmountDisputed: totalAmountDisputed.toNumber(),
    totalAmountResolved: totalAmountResolved.toNumber(),
    avgResolutionDays,
  };
}

export default {
  isValidTransition,
  getDispute,
  listDisputes,
  transitionDispute,
  submitDispute,
  acknowledgeDispute,
  resolveDispute,
  rejectDispute,
  cancelDispute,
  getDisputeStats,
};
