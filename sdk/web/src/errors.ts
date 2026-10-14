export type SdkErrorCode =
  | 'INIT_REQUIRED'
  | 'INVALID_OPTIONS'
  | 'NETWORK'
  | 'TIMEOUT'
  | 'BAD_RESPONSE'
  | 'VALIDATION'
  | 'NO_FILL'
  | 'UNKNOWN';

export class SdkError extends Error {
  readonly code: SdkErrorCode;
  readonly details?: Record<string, unknown>;

  constructor(code: SdkErrorCode, message: string, details?: Record<string, unknown>, cause?: unknown) {
    super(message);
    this.name = 'SdkError';
    this.code = code;
    this.details = details;
    if (cause && typeof (Error as any).captureStackTrace === 'function') {
      (Error as any).captureStackTrace(this, SdkError);
    }
    // Non-standard on the ES2020 lib target, but helpful.
    (this as { cause?: unknown }).cause = cause;
  }
}

export const Errors = {
  initRequired: () => new SdkError('INIT_REQUIRED', 'SDK must be initialized via init() first'),
  invalidOptions: (msg: string, details?: Record<string, unknown>) => new SdkError('INVALID_OPTIONS', msg, details),
  network: (msg: string, details?: Record<string, unknown>, cause?: unknown) => new SdkError('NETWORK', msg, details, cause),
  timeout: (timeoutMs: number) => new SdkError('TIMEOUT', `Request timed out after ${timeoutMs}ms`, { timeoutMs }),
  badResponse: (status: number, body?: unknown) => new SdkError('BAD_RESPONSE', `Unexpected response status ${status}`, { status, body }),
  validation: (msg: string, details?: Record<string, unknown>) => new SdkError('VALIDATION', msg, details),
  noFill: () => new SdkError('NO_FILL', 'No ad fill returned from auction'),
  unknown: (msg = 'Unknown SDK error', details?: Record<string, unknown>, cause?: unknown) => new SdkError('UNKNOWN', msg, details, cause),
};
