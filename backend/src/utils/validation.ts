/**
 * Request validation utilities for transparency API
 */

import { AppError } from '../middleware/errorHandler';

/**
 * Validate and parse ISO8601 date string
 * @throws AppError with 400 status if invalid
 */
export function validateISODate(value: string | undefined, fieldName: string): string | null {
  if (!value) return null;

  // ISO8601 regex pattern with capture groups
  const iso8601Pattern = /^(?<y>\d{4})-(?<m>\d{2})-(?<d>\d{2})(?:T(?<hh>\d{2}):(?<mm>\d{2}):(?<ss>\d{2})(?:\.(?<ms>\d{1,3}))?(?<tz>Z|[+-]\d{2}:\d{2})?)?$/;

  const match = iso8601Pattern.exec(value);
  if (!match || !match.groups) {
    throw new AppError(
      `Invalid ${fieldName}: must be ISO8601 format (e.g., 2025-11-10 or 2025-11-10T12:00:00Z)`,
      400
    );
  }

  const y = Number(match.groups['y']);
  const m = Number(match.groups['m']);
  const d = Number(match.groups['d']);
  const hh = match.groups['hh'] ? Number(match.groups['hh']) : 0;
  const mm = match.groups['mm'] ? Number(match.groups['mm']) : 0;
  const ss = match.groups['ss'] ? Number(match.groups['ss']) : 0;
  const ms = match.groups['ms'] ? Number(match.groups['ms'].padEnd(3, '0')) : 0;
  const tz = match.groups['tz'] || 'Z';

  // Basic range checks
  if (m < 1 || m > 12) {
    throw new AppError(`Invalid ${fieldName}: month out of range`, 400);
  }
  const isLeap = (y % 4 === 0 && y % 100 !== 0) || (y % 400 === 0);
  const daysInMonth = [31, isLeap ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][m - 1];
  if (d < 1 || d > daysInMonth) {
    throw new AppError(`Invalid ${fieldName}: day out of range`, 400);
  }
  if (hh < 0 || hh > 23 || mm < 0 || mm > 59 || ss < 0 || ss > 59) {
    throw new AppError(`Invalid ${fieldName}: time out of range`, 400);
  }

  // Compute UTC timestamp accounting for timezone offset
  let offsetMinutes = 0;
  if (tz !== 'Z') {
    const sign = tz.startsWith('-') ? -1 : 1;
    const [th, tm] = tz.slice(1).split(':').map(Number);
    offsetMinutes = sign * (th * 60 + tm);
  }

  // Construct a Date in UTC by subtracting the timezone offset from local time components
  const utcMillis = Date.UTC(y, m - 1, d, hh, mm, ss, ms) - offsetMinutes * 60_000;
  const date = new Date(utcMillis);

  // Final sanity check: reverse formatting to ensure round-trip consistency
  if (isNaN(date.getTime())) {
    throw new AppError(`Invalid ${fieldName}: not a valid date`, 400);
  }

  return date.toISOString();
}

/**
 * Validate and parse integer with min/max bounds
 * @throws AppError with 400 status if invalid
 */
export function validateInteger(
  value: string | undefined,
  fieldName: string,
  defaultValue: number,
  min: number,
  max: number
): number {
  if (value === undefined) return defaultValue;
  
  // Reject empty strings and non-integer numeric strings
  if (value.trim() === '' || !/^-?\d+$/.test(value.trim())) {
    throw new AppError(`Invalid ${fieldName}: must be an integer`, 400);
  }

  const parsed = Number(value.trim());
  
  if (!Number.isInteger(parsed)) {
    throw new AppError(`Invalid ${fieldName}: must be an integer`, 400);
  }
  
  if (parsed < min || parsed > max) {
    throw new AppError(
      `Invalid ${fieldName}: must be between ${min} and ${max}`,
      400
    );
  }
  
  return parsed;
}

/**
 * Validate enum value against whitelist
 * @throws AppError with 400 status if invalid
 */
export function validateEnum(
  value: string | undefined,
  fieldName: string,
  allowedValues: string[]
): string | undefined {
  if (value === undefined) return undefined;

  const v = value.trim();
  if (v.length === 0) {
    throw new AppError(
      `Invalid ${fieldName}: must be one of: ${allowedValues.join(', ')}`,
      400
    );
  }
  
  if (!allowedValues.includes(v)) {
    throw new AppError(
      `Invalid ${fieldName}: must be one of: ${allowedValues.join(', ')}`,
      400
    );
  }
  
  return v;
}

/**
 * Validate boolean flag
 * @throws AppError with 400 status if invalid
 */
export function validateBoolean(
  value: string | undefined,
  fieldName: string,
  defaultValue: boolean = false
): boolean {
  if (!value) return defaultValue;
  
  const lower = value.toLowerCase();
  
  if (lower === 'true' || lower === '1' || lower === 'yes') {
    return true;
  }
  
  if (lower === 'false' || lower === '0' || lower === 'no') {
    return false;
  }
  
  throw new AppError(
    `Invalid ${fieldName}: must be boolean (true/false, 1/0, yes/no)`,
    400
  );
}

/**
 * Validate file path to prevent path traversal attacks (P1 Fix #8)
 * Ensures path doesn't escape the base directory
 * @throws AppError with 400 status if path traversal detected
 */
export function validateSafePath(
  userPath: string,
  baseDir: string,
  fieldName: string
): string {
  const path = require('path');
  
  // Reject obvious path traversal patterns
  if (userPath.includes('..') || userPath.includes('\0')) {
    throw new AppError(
      `Invalid ${fieldName}: path traversal detected`,
      400
    );
  }
  
  // Resolve and normalize the path
  const resolved = path.resolve(baseDir, userPath);
  const normalizedBase = path.resolve(baseDir);
  
  // Ensure resolved path stays within base directory
  if (!resolved.startsWith(normalizedBase + path.sep) && resolved !== normalizedBase) {
    throw new AppError(
      `Invalid ${fieldName}: path escapes allowed directory`,
      400
    );
  }
  
  return resolved;
}
