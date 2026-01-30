import { parse } from 'csv-parse/sync'
import type { NormalizedMappingRow } from '../services/migrationImportConnectors'

type ColumnKey = keyof Pick<NormalizedMappingRow, 'network' | 'instanceId' | 'instanceName' | 'waterfallPosition' | 'ecpmCents'>

/**
 * eCPM Unit Detection
 * 
 * PROBLEM: Network exports vary in how they report eCPM:
 * - Some use dollars (e.g., 1.50 meaning $1.50)
 * - Some use cents (e.g., 150 meaning $1.50)
 * 
 * The system internally uses cents. This ambiguity can cause 100x over/under estimation.
 * 
 * SOLUTION: Column headers hint at the unit:
 * - Headers containing 'cents', '(cents)', '_cents' → value is in cents
 * - Headers containing 'dollars', '(usd)', '$', '_usd' → value is in dollars, convert to cents
 * - Plain 'ecpm' or 'eCPM' → assume dollars (most common network export format), convert to cents
 */
const ECPM_CENTS_ALIASES = ['ecpm_cents', 'eCPM (cents)', 'floor_cents', 'Floor (cents)', 'cpm_cents']
const ECPM_DOLLARS_ALIASES = ['ecpm', 'eCPM', 'floor', 'Floor', 'cpm', 'CPM', 'ecpm_usd', 'eCPM ($)', 'ecpm_dollars']

const COLUMN_ALIASES: Record<ColumnKey, string[]> = {
  network: ['network', 'Network', 'provider', 'Provider', 'Network Name'],
  instanceId: ['instance_id', 'instanceId', 'InstanceID', 'instance id', 'Placement Instance ID'],
  instanceName: ['instance_name', 'InstanceName', 'name', 'Name', 'Label', 'Ad Unit Name'],
  waterfallPosition: ['waterfall_position', 'position', 'Position', 'rank', 'Rank', 'placement_priority'],
  // For backward compat, keep this - but detection logic below handles the unit conversion
  ecpmCents: [...ECPM_CENTS_ALIASES, ...ECPM_DOLLARS_ALIASES],
}

const HIGH_CONFIDENCE_NETWORKS = ['ironSource', 'AppLovin', 'MAX', 'UnityAds']

function resolveColumn(row: Record<string, string>, candidateKeys: string[]): string | undefined {
  for (const key of candidateKeys) {
    if (Object.prototype.hasOwnProperty.call(row, key) && row[key]?.toString().trim()) {
      return row[key].toString().trim()
    }
  }
  return undefined
}

/**
 * Detect which eCPM column header is present and whether it implies dollars or cents
 */
function detectEcpmColumn(row: Record<string, string>): { value: string | undefined; unit: 'cents' | 'dollars' } {
  // First check for explicit cents columns
  for (const key of ECPM_CENTS_ALIASES) {
    if (Object.prototype.hasOwnProperty.call(row, key) && row[key]?.toString().trim()) {
      return { value: row[key].toString().trim(), unit: 'cents' }
    }
  }
  // Then check for dollars columns (more common in network exports)
  for (const key of ECPM_DOLLARS_ALIASES) {
    if (Object.prototype.hasOwnProperty.call(row, key) && row[key]?.toString().trim()) {
      return { value: row[key].toString().trim(), unit: 'dollars' }
    }
  }
  return { value: undefined, unit: 'dollars' }
}

function coerceNumber(value: string | undefined, field: string, rowIndex: number): number | undefined {
  if (value === undefined || value === '') return undefined
  const parsed = Number(value)
  if (!Number.isFinite(parsed)) {
    throw new Error(`Invalid numeric value for ${field} on row ${rowIndex}`)
  }
  return parsed
}

export interface ParseMigrationCsvOptions {
  /**
   * Override automatic eCPM unit detection.
   * - 'cents': Input values are already in cents
   * - 'dollars': Input values are in dollars, will be converted to cents
   * - 'auto' (default): Detect from column headers
   */
  ecpmUnit?: 'cents' | 'dollars' | 'auto'
}

export function parseMigrationCsv(buffer: Buffer, options: ParseMigrationCsvOptions = {}): NormalizedMappingRow[] {
  const csv = buffer.toString('utf8')
  if (!csv.trim()) {
    throw new Error('CSV file is empty')
  }

  const records = parse(csv, {
    columns: true,
    skip_empty_lines: true,
    trim: true,
  }) as Array<Record<string, string>>

  if (records.length === 0) {
    throw new Error('CSV has headers but no data rows')
  }

  // Detect eCPM unit from first row headers (if auto mode)
  let globalEcpmUnit: 'cents' | 'dollars' = 'dollars'
  if (options.ecpmUnit && options.ecpmUnit !== 'auto') {
    globalEcpmUnit = options.ecpmUnit
  } else if (records.length > 0) {
    const detection = detectEcpmColumn(records[0])
    globalEcpmUnit = detection.unit
  }

  return records.map((rawRow, index) => {
    const rowNumber = index + 2 // account for header row when surfacing errors

    const network = resolveColumn(rawRow, COLUMN_ALIASES.network)
    const instanceId = resolveColumn(rawRow, COLUMN_ALIASES.instanceId)

    if (!network || !instanceId) {
      throw new Error(`Missing required columns on row ${rowNumber}: expected network and instance identifier`)
    }

    const instanceName = resolveColumn(rawRow, COLUMN_ALIASES.instanceName)
    const waterfallPositionRaw = resolveColumn(rawRow, COLUMN_ALIASES.waterfallPosition)
    
    // Get eCPM value with unit detection
    const ecpmDetection = detectEcpmColumn(rawRow)
    const ecpmRaw = coerceNumber(ecpmDetection.value, 'eCPM', rowNumber)
    
    // Convert to cents if needed
    let ecpmCents: number | undefined
    if (ecpmRaw !== undefined) {
      const effectiveUnit = options.ecpmUnit && options.ecpmUnit !== 'auto' ? options.ecpmUnit : ecpmDetection.unit
      ecpmCents = effectiveUnit === 'dollars' ? Math.round(ecpmRaw * 100) : ecpmRaw
    }

    const waterfallPosition = coerceNumber(waterfallPositionRaw, 'waterfall position', rowNumber)

    const normalizedNetwork = network.replace(/_/g, ' ')
    const confidence = HIGH_CONFIDENCE_NETWORKS.some((candidate) =>
      normalizedNetwork.toLowerCase().includes(candidate.toLowerCase())
    )
      ? ('high' as const)
      : ('medium' as const)

    return {
      network: network,
      instanceId,
      instanceName,
      waterfallPosition,
      ecpmCents,
      confidence,
    }
  })
}
