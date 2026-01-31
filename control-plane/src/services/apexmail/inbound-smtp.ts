import { SMTPServer, SMTPServerSession, SMTPServerAddress } from 'smtp-server';
import { simpleParser, AddressObject } from 'mailparser';
import { Readable } from 'stream';
import { logger } from '../../logger';

// ============================================================================
// CONFIGURATION INTERFACE
// ============================================================================
export interface InboundSMTPConfig {
  port: number;
  host: string;
  webhookUrl: string;
  acceptedDomains: string[];
  maxMessageSize: number;
  webhookTimeoutMs: number;
  apiKey?: string;
}

const DEFAULT_CONFIG: InboundSMTPConfig = {
  port: process.env.SMTP_PORT ? parseInt(process.env.SMTP_PORT) : 2525,
  host: '0.0.0.0',
  webhookUrl: process.env.INBOUND_EMAIL_WEBHOOK_URL || 'http://127.0.0.1:4001/api/autonomous/webhook/sendgrid',
  acceptedDomains: (process.env.SMTP_ACCEPTED_DOMAINS || 'replies.apexmediation.ee').split(',').map(d => d.trim().toLowerCase()),
  maxMessageSize: 10 * 1024 * 1024, // 10MB
  webhookTimeoutMs: 30000, // 30 seconds
  apiKey: process.env.AUTONOMOUS_API_KEY || process.env.SALES_AUTOMATION_API_KEY,
};

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

function extractEmail(input: string): string {
  const match = input.match(/<([^>]+)>/);
  return (match ? match[1] : input).trim().toLowerCase();
}

function extractDomain(email: string): string {
  const parts = email.split('@');
  return parts.length > 1 ? parts[1].toLowerCase() : '';
}

function addressToString(addr: AddressObject | AddressObject[] | undefined): string {
  if (!addr) return '';
  if (Array.isArray(addr)) {
    return addr.map(a => a.text).join(', ');
  }
  return addr.text || '';
}

function sanitizeForLog(text: string | undefined, maxLen: number = 100): string {
  if (!text) return '';
  const sanitized = text.replace(/[\n\r]+/g, ' ').trim();
  return sanitized.length > maxLen ? sanitized.substring(0, maxLen) + '...' : sanitized;
}

// ============================================================================
// INBOUND SMTP SERVER CLASS
// ============================================================================

export class InboundSMTPServer {
  private server: SMTPServer;
  private config: InboundSMTPConfig;

  constructor(config: Partial<InboundSMTPConfig> = {}) {
    this.config = { ...DEFAULT_CONFIG, ...config };

    this.server = new SMTPServer({
      disabledCommands: ['AUTH'],
      size: this.config.maxMessageSize,
      secure: false,
      banner: 'ApexMediation SMTP Server Ready',

      onConnect: (session, callback) => this.handleConnect(session, callback),
      onMailFrom: (address, session, callback) => this.handleMailFrom(address, session, callback),
      onRcptTo: (address, session, callback) => this.handleRcptTo(address, session, callback),
      onData: (stream, session, callback) => this.handleData(stream, session, callback),
      onClose: (session) => this.handleClose(session),
    });

    this.server.on('error', (err: Error) => {
      logger.error('[InboundSMTP] Server error', { error: err.message, stack: err.stack });
    });
  }

  public start(): void {
    this.server.listen(this.config.port, this.config.host, () => {
      logger.info(`[InboundSMTP] Server listening on ${this.config.host}:${this.config.port}`, {
        acceptedDomains: this.config.acceptedDomains,
        webhookUrl: this.config.webhookUrl
      });
    });
  }

  public async stop(): Promise<void> {
    return new Promise((resolve) => {
      this.server.close(() => {
        logger.info('[InboundSMTP] Server stopped');
        resolve();
      });
    });
  }

  private handleConnect(session: SMTPServerSession, callback: (err?: Error | null) => void) {
    logger.info('[InboundSMTP] Client connected', { 
      remoteAddress: session.remoteAddress,
      sessionId: session.id 
    });
    callback();
  }

  private handleMailFrom(address: SMTPServerAddress, session: SMTPServerSession, callback: (err?: Error | null) => void) {
    const senderEmail = extractEmail(address.address);
    if (!senderEmail || !senderEmail.includes('@')) {
      logger.warn('[InboundSMTP] Rejected invalid sender', { sender: address.address, sessionId: session.id });
      return callback(new Error('Invalid sender address'));
    }
    callback();
  }

  private handleRcptTo(address: SMTPServerAddress, session: SMTPServerSession, callback: (err?: Error | null) => void) {
    const recipientEmail = extractEmail(address.address);
    const domain = extractDomain(recipientEmail);
    
    if (!this.config.acceptedDomains.includes(domain)) {
      logger.warn('[InboundSMTP] Rejected email for unknown domain', { 
        recipient: recipientEmail, 
        domain, 
        sessionId: session.id 
      });
      const err = new Error(`Relay access denied for domain: ${domain}`);
      (err as any).responseCode = 550;
      return callback(err);
    }
    
    callback();
  }

  private handleData(stream: Readable, session: SMTPServerSession, callback: (err?: Error | null) => void) {
    const sessionId = session.id;
    
    simpleParser(stream)
      .then(async (parsed) => {
        const fromText = parsed.from?.text || '';
        const toText = addressToString(parsed.to);
        const subject = parsed.subject || '(no subject)';
        const textBody = parsed.text || '';
        const htmlBody = parsed.html || '';
        
        const payload = {
          from: fromText,
          to: toText,
          subject: subject,
          text: textBody,
          html: htmlBody,
          headers: Object.fromEntries(parsed.headers),
          date: parsed.date?.toISOString() || new Date().toISOString(),
          messageId: parsed.messageId || '',
          inReplyTo: parsed.inReplyTo || '',
          references: parsed.references || [],
          attachmentCount: parsed.attachments?.length || 0,
        };

        if (!this.config.apiKey) {
          logger.error('[InboundSMTP] API Key not configured - refusing to forward email', { sessionId });
          const secErr = new Error('Server misconfiguration - missing API key');
          (secErr as any).responseCode = 451;
          return callback(secErr);
        }
        
        try {
          const controller = new AbortController();
          const timeoutId = setTimeout(() => controller.abort(), this.config.webhookTimeoutMs);
          
          const response = await fetch(this.config.webhookUrl, {
            method: 'POST',
            signal: controller.signal,
            headers: {
              'Content-Type': 'application/json',
              'X-Inbound-SMTP-Session': sessionId,
              'X-API-Key': this.config.apiKey!,
            },
            body: JSON.stringify(payload),
          });
          
          clearTimeout(timeoutId);
          
          if (!response.ok) {
            throw new Error(`HTTP ${response.status}: ${response.statusText}`);
          }
          
          logger.info('[InboundSMTP] Webhook delivery successful', { 
            sessionId,
            status: response.status 
          });
          callback();
          
        } catch (err: unknown) {
          logger.error('[InboundSMTP] Webhook request failed', { 
            sessionId, 
            error: err instanceof Error ? err.message : String(err) 
          });
          
          const tempErr = new Error('Temporary processing error - please retry later');
          (tempErr as any).responseCode = 451;
          callback(tempErr);
        }
      })
      .catch((parseErr: Error) => {
        logger.error('[InboundSMTP] Email parsing failed', { sessionId, error: parseErr.message });
        const permErr = new Error('Unable to parse email content');
        (permErr as any).responseCode = 550;
        callback(permErr);
      });
  }

  private handleClose(session: SMTPServerSession) {
    logger.info('[InboundSMTP] Client disconnected', { sessionId: session.id });
  }
}
