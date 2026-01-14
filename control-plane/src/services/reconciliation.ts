import { getReadOnlyPool, getOperatorPool } from '../database';
import { logger } from '../logger';
import { logAudit } from '../audit';

export interface BankStatementEntry {
  date: string;
  amount: number;
  currency: string;
  reference: string;
  counterparty: string;
  type: 'incoming' | 'outgoing';
}

export interface ReconciliationMatch {
  entry: BankStatementEntry;
  suggestedMatch?: {
    type: 'invoice' | 'expense';
    id: string;
    number: string;
    amount: number;
    confidence: number;
  };
}

/**
 * Bank Reconciliation Service
 * Matches bank movements with ledger/invoices/expenses
 */

/**
 * Mock import of bank statement (SEPA XML/Wise JSON)
 */
export async function processBankStatement(operator: string): Promise<ReconciliationMatch[]> {
  // In a real system, this would parse a file upload
  // Here we'll query for recently un-reconciled bank moves (if we had a table for them)
  // or simulate some moves to match
  
  const entries: BankStatementEntry[] = [
    { date: '2026-01-10', amount: 500.00, currency: 'EUR', reference: 'INV-000001', counterparty: 'Acme Corp', type: 'incoming' },
    { date: '2026-01-11', amount: -150.00, currency: 'EUR', reference: 'AWS-99231', counterparty: 'Amazon Web Services', type: 'outgoing' },
    { date: '2026-01-12', amount: 1200.50, currency: 'EUR', reference: 'Payout for Q4', counterparty: 'Publisher X', type: 'incoming' }
  ];

  const matches: ReconciliationMatch[] = [];

  for (const entry of entries) {
    let suggestedMatch;

    if (entry.type === 'incoming') {
      // Look for invoices with matching number or amount
      const invSql = `
        SELECT id, invoice_number, total_cents
        FROM invoices
        WHERE (invoice_number = $1 OR total_cents = $2)
        AND status != 'paid'
        LIMIT 1
      `;
      const invMatch = await getReadOnlyPool().query(invSql, [entry.reference, Math.abs(entry.amount * 100)]);
      if (invMatch.rows.length > 0) {
        suggestedMatch = {
          type: 'invoice' as const,
          id: invMatch.rows[0].id,
          number: invMatch.rows[0].invoice_number,
          amount: invMatch.rows[0].total_cents / 100,
          confidence: invMatch.rows[0].invoice_number === entry.reference ? 1.0 : 0.7
        };
      }
    } else {
      // Look for expenses
      const expSql = `
        SELECT id, description, amount_eur_cents
        FROM expenses
        WHERE (description ILIKE $1 OR amount_eur_cents = $2)
        AND status != 'paid'
        LIMIT 1
      `;
      const expMatch = await getReadOnlyPool().query(expSql, [`%${entry.reference}%`, Math.abs(entry.amount * 100)]);
      if (expMatch.rows.length > 0) {
        suggestedMatch = {
          type: 'expense' as const,
          id: expMatch.rows[0].id,
          number: expMatch.rows[0].description,
          amount: expMatch.rows[0].amount_eur_cents / 100,
          confidence: 0.8
        };
      }
    }

    matches.push({ entry, suggestedMatch });
  }

  return matches;
}

/**
 * Confirm a reconciliation match
 * Updates the invoice/expense status and records the payment
 */
export async function confirmReconciliation(
  match: { entry: BankStatementEntry; targetType: 'invoice' | 'expense'; targetId: string },
  actor: string
): Promise<boolean> {
  const pool = getOperatorPool();
  
  try {
    if (match.targetType === 'invoice') {
      await pool.query(
        `UPDATE invoices SET status = 'paid', paid_at = NOW(), payment_reference = $1 WHERE id = $2`,
        [match.entry.reference, match.targetId]
      );
    } else {
      await pool.query(
        `UPDATE expenses SET status = 'paid', payment_date = NOW(), payment_reference = $1 WHERE id = $2`,
        [match.entry.reference, match.targetId]
      );
    }

    await logAudit({
      actor,
      action: 'reconcile_payment',
      target: match.targetId,
      result: 'success',
      reason: `Reconciled ${match.targetType} with bank entry ${match.entry.reference}`
    });

    return true;
  } catch (err) {
    logger.error('Failed to confirm reconciliation', { error: (err as Error).message });
    return false;
  }
}
