/**
 * SSAI Logger
 * Structured logging for SSAI operations
 */

export interface LoggerConfig {
  level: 'debug' | 'info' | 'warn' | 'error';
  onLog?: (message: string) => void;
}

export interface LogContext {
  sessionId?: string;
  podId?: string;
  partnerId?: string;
  [key: string]: unknown;
}

export class SSAILogger {
  private config: LoggerConfig;
  private context: LogContext;
  private readonly levels = ['debug', 'info', 'warn', 'error'];

  constructor(config?: Partial<LoggerConfig>, context: LogContext = {}) {
    this.config = {
      level: 'info',
      ...config,
    };
    this.context = context;
  }

  /**
   * Create child logger with additional context
   */
  child(additionalContext: LogContext): SSAILogger {
    return new SSAILogger(this.config, {
      ...this.context,
      ...additionalContext,
    });
  }

  /**
   * Debug level log
   */
  debug(message: string, context?: LogContext): void {
    this.log('debug', message, context);
  }

  /**
   * Info level log
   */
  info(message: string, context?: LogContext): void {
    this.log('info', message, context);
  }

  /**
   * Warning level log
   */
  warn(message: string, context?: LogContext): void {
    this.log('warn', message, context);
  }

  /**
   * Error level log
   */
  error(message: string, error?: Error, context?: LogContext): void {
    const errorContext = error
      ? {
          ...context,
          error: {
            message: error.message,
            stack: error.stack,
            name: error.name,
          },
        }
      : context;

    this.log('error', message, errorContext);
  }

  private log(level: string, message: string, additionalContext?: LogContext): void {
    const levelIndex = this.levels.indexOf(level);
    const configLevelIndex = this.levels.indexOf(this.config.level);

    if (levelIndex < configLevelIndex) {
      return;
    }

    const logEntry = {
      timestamp: new Date().toISOString(),
      level,
      message,
      ...this.context,
      ...additionalContext,
    };

    const formatted = JSON.stringify(logEntry);

    if (this.config.onLog) {
      this.config.onLog(formatted);
    } else {
      switch (level) {
        case 'error':
          console.error(formatted);
          break;
        case 'warn':
          console.warn(formatted);
          break;
        case 'debug':
          console.debug(formatted);
          break;
        default:
          console.log(formatted);
      }
    }
  }
}

export default SSAILogger;
