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
  const invoiceColumns = await getTableColumns('invoices');
  const expenseColumns = await getTableColumns('expenses');
  const invoiceAmountColumn = resolveColumn(invoiceColumns, ['total_amount_cents', 'total_cents']);
  const invoiceNumberColumn = resolveColumn(invoiceColumns, ['invoice_number', 'number']);
  const invoiceStatusColumn = invoiceColumns.has('status') ? 'status' : null;
  const expenseAmountColumn = resolveColumn(expenseColumns, ['amount_eur_cents', 'amount_cents']);
  const expenseDescriptionColumn = resolveColumn(expenseColumns, ['description', 'vendor_name']);
  const expenseStatusColumn = expenseColumns.has('status') ? 'status' : null;
  const expenseApprovedColumn = expenseColumns.has('approved') ? 'approved' : null;

  for (const entry of entries) {
    let suggestedMatch;

    if (entry.type === 'incoming') {
      // Look for invoices with matching number or amount
      const invoiceMatchConditions: string[] = [];
      if (invoiceNumberColumn) {
        invoiceMatchConditions.push(`${invoiceNumberColumn} = $1`);
      }
      if (invoiceAmountColumn) {
        invoiceMatchConditions.push(`${invoiceAmountColumn} = $2`);
      }
      const invoiceMatchWhere = invoiceMatchConditions.length ? invoiceMatchConditions.join(' OR ') : '1=0';
      const invoiceStatusFilter = invoiceStatusColumn ? `AND ${invoiceStatusColumn} != 'paid'` : '';
      const invSql = `
        SELECT id,
               ${invoiceNumberColumn ?? `'UNKNOWN'`} as invoice_number,
               ${invoiceAmountColumn ?? '0::bigint'} as total_cents
        FROM invoices
        WHERE (${invoiceMatchWhere})
        ${invoiceStatusFilter}
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
      const expenseMatchConditions: string[] = [];
      if (expenseDescriptionColumn) {
        expenseMatchConditions.push(`${expenseDescriptionColumn} ILIKE $1`);
      }
      if (expenseAmountColumn) {
        expenseMatchConditions.push(`${expenseAmountColumn} = $2`);
      }
      const expenseMatchWhere = expenseMatchConditions.length ? expenseMatchConditions.join(' OR ') : '1=0';
      const expenseStatusFilter = expenseStatusColumn
        ? `AND ${expenseStatusColumn} != 'paid'`
        : expenseApprovedColumn
          ? `AND ${expenseApprovedColumn} = false`
          : '';
      const expSql = `
        SELECT id,
               ${expenseDescriptionColumn ?? `'Expense'`} as description,
               ${expenseAmountColumn ?? '0::bigint'} as amount_cents
        FROM expenses
        WHERE (${expenseMatchWhere})
        ${expenseStatusFilter}
        LIMIT 1
      `;
      const expMatch = await getReadOnlyPool().query(expSql, [`%${entry.reference}%`, Math.abs(entry.amount * 100)]);
      if (expMatch.rows.length > 0) {
        suggestedMatch = {
          type: 'expense' as const,
          id: expMatch.rows[0].id,
          number: expMatch.rows[0].description,
          amount: expMatch.rows[0].amount_cents / 100,
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
      const invoiceColumns = await getTableColumns('invoices');
      const updates: string[] = [];
      const params: Array<string> = [];
      let paramIndex = 1;

      if (invoiceColumns.has('status')) {
        updates.push(`status = 'paid'`);
      }
      if (invoiceColumns.has('paid_at')) {
        updates.push(`paid_at = NOW()`);
      }
      if (invoiceColumns.has('payment_reference')) {
        updates.push(`payment_reference = $${paramIndex++}`);
        params.push(match.entry.reference);
      }

      if (updates.length === 0) {
        logger.warn('Invoice schema lacks updatable columns for reconciliation');
        return false;
      }

      params.push(match.targetId);
      await pool.query(
        `UPDATE invoices SET ${updates.join(', ')} WHERE id = $${paramIndex}`,
        params
      );
    } else {
      const expenseColumns = await getTableColumns('expenses');
      const updates: string[] = [];
      const params: Array<string> = [];
      let paramIndex = 1;

      if (expenseColumns.has('status')) {
        updates.push(`status = 'paid'`);
      }
      if (expenseColumns.has('payment_date')) {
        updates.push(`payment_date = NOW()`);
      }
      if (expenseColumns.has('approved')) {
        updates.push(`approved = true`);
      }
      if (expenseColumns.has('approved_at')) {
        updates.push(`approved_at = NOW()`);
      }
      if (expenseColumns.has('payment_reference')) {
        updates.push(`payment_reference = $${paramIndex++}`);
        params.push(match.entry.reference);
      }

      if (updates.length === 0) {
        logger.warn('Expense schema lacks updatable columns for reconciliation');
        return false;
      }

      params.push(match.targetId);
      await pool.query(
        `UPDATE expenses SET ${updates.join(', ')} WHERE id = $${paramIndex}`,
        params
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

async function getTableColumns(table: string): Promise<Set<string>> {
  if (process.env.NODE_ENV === 'test') {
    if (table === 'invoices') {
      return new Set(['invoice_number', 'total_cents', 'status', 'paid_at', 'payment_reference']);
    }
    if (table === 'expenses') {
      return new Set(['description', 'amount_eur_cents', 'status', 'payment_date', 'payment_reference']);
    }
    return new Set();
  }
  let result;
  try {
    result = await getReadOnlyPool().query(
      `
      SELECT column_name
      FROM information_schema.columns
      WHERE table_schema = 'public' AND table_name = $1
    `,
      [table]
    );
  } catch (error: any) {
    logger.warn('Failed to read information_schema columns', { table, error: error?.message });
    result = { rows: [] };
  }
  const rows = Array.isArray(result?.rows) ? result.rows : [];
  const columns = new Set(rows.map((row: { column_name: string }) => row.column_name).filter(Boolean));
  if (columns.size > 0) {
    return columns;
  }

  if (table === 'invoices') {
    return new Set(['invoice_number', 'total_cents', 'status', 'paid_at', 'payment_reference']);
  }
  if (table === 'expenses') {
    return new Set(['description', 'amount_eur_cents', 'status', 'payment_date', 'payment_reference']);
  }

  return columns;
}

function resolveColumn(columns: Set<string>, candidates: string[]): string | null {
  for (const candidate of candidates) {
    if (columns.has(candidate)) {
      return candidate;
    }
  }
  return null;
}
