/**
 * Migration Studio Unit Constants
 * 
 * Standardized unit conversion constants to prevent scaling bugs across the codebase.
 * All monetary values should flow through these constants to ensure consistency.
 * 
 * BASE UNITS:
 * - Revenue: micros (1 dollar = 1,000,000 micros) - used for precision in aggregations
 * - eCPM: cents (1 dollar = 100 cents) - used in UI and API responses
 * - CPM: Cost Per Mille (per 1000 impressions)
 * - RPI: Revenue Per Impression
 * 
 * CONVERSION FORMULAS:
 * - Single impression revenue (micros) from CPM (dollars): cpm_dollars * 1000
 * - Single impression revenue (micros) from CPM (cents): cpm_cents * 10
 * - eCPM (cents) from micros: micros / 10000
 * - eCPM (dollars) from micros: micros / 1000000
 */

// ─────────────────────────────────────────────────────────────────────────────
// Core Unit Multipliers
// ─────────────────────────────────────────────────────────────────────────────

/** Number of cents in one dollar */
export const CENTS_PER_DOLLAR = 100;

/** Number of micros in one dollar (micro = 10^-6) */
export const MICROS_PER_DOLLAR = 1_000_000;

/** Number of micros in one cent */
export const MICROS_PER_CENT = 10_000;

/** Number of impressions per mille (CPM denominator) */
export const IMPRESSIONS_PER_MILLE = 1000;

// ─────────────────────────────────────────────────────────────────────────────
// Revenue Conversion (CPM → Single Impression)
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Multiplier to convert CPM (dollars) to single-impression revenue (micros)
 * 
 * Formula: cpm_dollars / 1000 impressions * 1,000,000 micros/dollar = cpm_dollars * 1000
 * 
 * Example: CPM of $2.50 → single impression earns $0.0025 → 2500 micros
 * Calculation: 2.50 * 1000 = 2500 micros
 */
export const CPM_DOLLARS_TO_IMPRESSION_MICROS = MICROS_PER_DOLLAR / IMPRESSIONS_PER_MILLE; // = 1000

/**
 * Multiplier to convert CPM (cents) to single-impression revenue (micros)
 * 
 * Example: CPM of 250 cents → single impression earns 2500 micros
 * Calculation: 250 * 10 = 2500 micros
 */
export const CPM_CENTS_TO_IMPRESSION_MICROS = MICROS_PER_CENT / IMPRESSIONS_PER_MILLE; // = 10

// ─────────────────────────────────────────────────────────────────────────────
// eCPM Display Conversion (micros → human-readable)
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Divisor to convert revenue per impression (micros) to eCPM (cents)
 * 
 * Formula: rpi_micros * 1000 impressions / 10,000 micros/cent = rpi_micros / 10
 * But we want eCPM per 1000 impressions, so: rpi_micros * 1000 / 10000 = rpi_micros / 10
 * 
 * Wait - let's be more careful:
 * - rpi_micros = revenue for 1 impression in micros
 * - eCPM (cents) = revenue for 1000 impressions in cents
 * - eCPM (cents) = rpi_micros * 1000 / 10000 = rpi_micros / 10
 * 
 * But typically we aggregate total_revenue_micros and total_impressions:
 * - eCPM (cents) = (total_revenue_micros / total_impressions) * 1000 / 10000
 * - eCPM (cents) = total_revenue_micros / total_impressions / 10
 */
export const RPI_MICROS_TO_ECPM_CENTS_DIVISOR = MICROS_PER_CENT / IMPRESSIONS_PER_MILLE; // = 10

/**
 * Divisor to convert revenue per impression (micros) to eCPM (dollars)
 */
export const RPI_MICROS_TO_ECPM_DOLLARS_DIVISOR = MICROS_PER_DOLLAR / IMPRESSIONS_PER_MILLE; // = 1000

// ─────────────────────────────────────────────────────────────────────────────
// Utility Functions
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Convert CPM in dollars to single-impression revenue in micros
 */
export function cpmDollarsToImpressionMicros(cpmDollars: number): number {
  return cpmDollars * CPM_DOLLARS_TO_IMPRESSION_MICROS;
}

/**
 * Convert CPM in cents to single-impression revenue in micros
 */
export function cpmCentsToImpressionMicros(cpmCents: number): number {
  return cpmCents * CPM_CENTS_TO_IMPRESSION_MICROS;
}

/**
 * Calculate eCPM in cents from total revenue (micros) and impressions
 */
export function calculateEcpmCents(revenueMicros: number, impressions: number): number {
  if (impressions === 0) return 0;
  const rpiMicros = revenueMicros / impressions;
  return rpiMicros / RPI_MICROS_TO_ECPM_CENTS_DIVISOR;
}

/**
 * Calculate eCPM in dollars from total revenue (micros) and impressions
 */
export function calculateEcpmDollars(revenueMicros: number, impressions: number): number {
  if (impressions === 0) return 0;
  const rpiMicros = revenueMicros / impressions;
  return rpiMicros / RPI_MICROS_TO_ECPM_DOLLARS_DIVISOR;
}

/**
 * Convert micros to dollars for display
 */
export function microsToDollars(micros: number): number {
  return micros / MICROS_PER_DOLLAR;
}

/**
 * Convert micros to cents for display
 */
export function microsToCents(micros: number): number {
  return micros / MICROS_PER_CENT;
}

/**
 * Convert dollars to micros for storage
 */
export function dollarsToMicros(dollars: number): number {
  return Math.round(dollars * MICROS_PER_DOLLAR);
}

/**
 * Convert cents to micros for storage
 */
export function centsToMicros(cents: number): number {
  return Math.round(cents * MICROS_PER_CENT);
}
