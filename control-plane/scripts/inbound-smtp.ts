import { SMTPServer, SMTPServerSession, SMTPServerAddress } from 'smtp-server';
import { simpleParser, AddressObject } from 'mailparser';
import axios from 'axios';
import { Readable } from 'stream';

// ============================================================================
// CONFIGURATION
// ============================================================================
const SMTP_PORT = process.env.SMTP_PORT ? parseInt(process.env.SMTP_PORT) : 2525;
const HOST = '0.0.0.0';
// Changed to default to autonomous API port (4001) since this runs on Test Server
const WEBHOOK_URL = process.env.INBOUND_EMAIL_WEBHOOK_URL || 'http://127.0.0.1:4001/api/autonomous/webhook/sendgrid';
const ACCEPTED_DOMAINS = (process.env.SMTP_ACCEPTED_DOMAINS || 'replies.apexmediation.ee').split(',').map(d => d.trim().toLowerCase());
const MAX_MESSAGE_SIZE = 10 * 1024 * 1024; // 10MB
const WEBHOOK_TIMEOUT_MS = 30000; // 30 seconds

// Logger with timestamps
const logger = {
  info: (msg: string, meta?: Record<string, unknown>) => {
    console.log(`[${new Date().toISOString()}] INFO: ${msg}`, meta ? JSON.stringify(meta) : '');
  },
  warn: (msg: string, meta?: Record<string, unknown>) => {
    console.warn(`[${new Date().toISOString()}] WARN: ${msg}`, meta ? JSON.stringify(meta) : '');
  },
  error: (msg: string, meta?: Record<string, unknown>) => {
    console.error(`[${new Date().toISOString()}] ERROR: ${msg}`, meta ? JSON.stringify(meta) : '');
  },
};

logger.info('Starting Inbound SMTP Server...', { port: SMTP_PORT, acceptedDomains: ACCEPTED_DOMAINS });

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

/**
 * Extract email address from "Name <email@domain.com>" or plain "email@domain.com"
 */
function extractEmail(input: string): string {
  const match = input.match(/<([^>]+)>/);
  return (match ? match[1] : input).trim().toLowerCase();
}

/**
 * Extract domain from email address
 */
function extractDomain(email: string): string {
  const parts = email.split('@');
  return parts.length > 1 ? parts[1].toLowerCase() : '';
}

/**
 * Convert AddressObject or AddressObject[] to a simple string
 */
function addressToString(addr: AddressObject | AddressObject[] | undefined): string {
  if (!addr) return '';
  if (Array.isArray(addr)) {
    return addr.map(a => a.text).join(', ');
  }
  return addr.text || '';
}

/**
 * Sanitize text for safe logging (truncate long content)
 */
function sanitizeForLog(text: string | undefined, maxLen: number = 100): string {
  if (!text) return '';
  const sanitized = text.replace(/[\n\r]+/g, ' ').trim();
  return sanitized.length > maxLen ? sanitized.substring(0, maxLen) + '...' : sanitized;
}

// ============================================================================
// SMTP SERVER
// ============================================================================

/**
 * INBOUND SMTP SERVER (Self-Hosted "Parse" Service)
 * 
 * Receives emails on port 2525 (mapped from 25 via iptables), parses them,
 * and POSTs them to the Control Plane webhook.
 * 
 * Architecture:
 *   [Internet] --(Port 25)--> [iptables] --(Port 2525)--> [This Script] --(POST)--> [Control Plane API]
 * 
 * Production Features:
 *   - Domain validation (only accepts emails for configured domains)
 *   - Proper error codes (4xx for temporary, 5xx for permanent failures)
 *   - Webhook timeout handling
 *   - Graceful shutdown
 */
const server = new SMTPServer({
  // Disable authentication (we accept mail from anyone for our subdomain)
  disabledCommands: ['AUTH'],
  
  // Size limit
  size: MAX_MESSAGE_SIZE,

  // Allow connections without STARTTLS (required for public mail reception)
  secure: false,
  
  // Banner
  banner: 'ApexMediation SMTP Server Ready',

  // -------------------------------------------------------------------------
  // 1. CONNECTION HANDLER (optional logging)
  // -------------------------------------------------------------------------
  onConnect(session: SMTPServerSession, callback: (err?: Error | null) => void) {
    logger.info('Client connected', { 
      remoteAddress: session.remoteAddress,
      clientHostname: session.clientHostname 
    });
    callback();
  },

  // -------------------------------------------------------------------------
  // 2. MAIL FROM HANDLER (sender validation)
  // -------------------------------------------------------------------------
  onMailFrom(address: SMTPServerAddress, session: SMTPServerSession, callback: (err?: Error | null) => void) {
    const senderEmail = extractEmail(address.address);
    logger.info('MAIL FROM received', { sender: senderEmail });
    
    // Basic sender validation - reject obviously invalid addresses
    if (!senderEmail || !senderEmail.includes('@')) {
      logger.warn('Rejected invalid sender', { sender: address.address });
      return callback(new Error('Invalid sender address'));
    }
    
    callback();
  },

  // -------------------------------------------------------------------------
  // 3. RCPT TO HANDLER (recipient domain validation)
  // -------------------------------------------------------------------------
  onRcptTo(address: SMTPServerAddress, session: SMTPServerSession, callback: (err?: Error | null) => void) {
    const recipientEmail = extractEmail(address.address);
    const domain = extractDomain(recipientEmail);
    
    logger.info('RCPT TO received', { recipient: recipientEmail, domain });
    
    // Validate domain
    if (!ACCEPTED_DOMAINS.includes(domain)) {
      logger.warn('Rejected email for unknown domain', { 
        recipient: recipientEmail, 
        domain, 
        acceptedDomains: ACCEPTED_DOMAINS 
      });
      // 550 = permanent failure (don't retry)
      const err = new Error(`Relay access denied for domain: ${domain}`);
      (err as any).responseCode = 550;
      return callback(err);
    }
    
    callback();
  },

  // -------------------------------------------------------------------------
  // 4. DATA HANDLER (parse email and forward to webhook)
  // -------------------------------------------------------------------------
  onData(stream: Readable, session: SMTPServerSession, callback: (err?: Error | null) => void) {
    const sessionId = session.id;
    logger.info('DATA stream started', { sessionId });
    
    simpleParser(stream)
      .then(async (parsed) => {
        const fromText = parsed.from?.text || '';
        const toText = addressToString(parsed.to);
        const subject = parsed.subject || '(no subject)';
        const textBody = parsed.text || '';
        const htmlBody = parsed.html || '';
        
        // Build SendGrid-compatible payload
        const payload = {
          from: fromText,
          to: toText,
          subject: subject,
          text: textBody,
          html: htmlBody,
          // Additional metadata
          headers: Object.fromEntries(parsed.headers),
          date: parsed.date?.toISOString() || new Date().toISOString(),
          messageId: parsed.messageId || '',
          inReplyTo: parsed.inReplyTo || '',
          references: parsed.references || [],
          // Attachments count (we don't forward binary data, just metadata)
          attachmentCount: parsed.attachments?.length || 0,
        };

        logger.info('Email parsed successfully', { 
          sessionId,
          from: sanitizeForLog(fromText),
          to: sanitizeForLog(toText),
          subject: sanitizeForLog(subject),
          bodyLength: textBody.length,
          attachmentCount: payload.attachmentCount,
        });
        
        // SECURITY: Require API key in production - no fallback to insecure default
        const apiKey = process.env.AUTONOMOUS_API_KEY || process.env.SALES_AUTOMATION_API_KEY;
        if (!apiKey) {
          logger.error('AUTONOMOUS_API_KEY not configured - refusing to forward email without authentication', { sessionId });
          const secErr = new Error('Server misconfiguration - missing API key');
          (secErr as any).responseCode = 451; // Temporary failure - admin should fix config
          return callback(secErr);
        }
        
        try {
          // POST to our internal API with timeout
          const response = await axios.post(WEBHOOK_URL, payload, {
            timeout: WEBHOOK_TIMEOUT_MS,
            headers: {
              'Content-Type': 'application/json',
              'X-Inbound-SMTP-Session': sessionId,
              'X-API-Key': apiKey,
            },
          });
          
          logger.info('Webhook delivery successful', { 
            sessionId,
            status: response.status,
            webhookResponse: response.data 
          });
          
          // 250 OK - email accepted
          callback();
          
        } catch (err: unknown) {
          const axiosErr = err as import('axios').AxiosError;
          if (axios.isAxiosError(err)) {
            if (axiosErr.code === 'ECONNREFUSED') {
              logger.error('Webhook connection refused - is control plane running?', { 
                sessionId,
                webhookUrl: WEBHOOK_URL 
              });
            } else if (axiosErr.code === 'ETIMEDOUT' || axiosErr.code === 'ECONNABORTED') {
              logger.error('Webhook timeout', { sessionId, timeout: WEBHOOK_TIMEOUT_MS });
            } else if (axiosErr.response) {
              logger.error('Webhook returned error', { 
                sessionId,
                status: axiosErr.response.status,
                data: axiosErr.response.data 
              });
            } else {
              logger.error('Webhook request failed', { sessionId, code: axiosErr.code, message: axiosErr.message });
            }
          } else {
            logger.error('Unexpected error during webhook delivery', { 
              sessionId, 
              error: err instanceof Error ? err.message : String(err) 
            });
          }
          
          // 451 = temporary failure (sender should retry later)
          const tempErr = new Error('Temporary processing error - please retry later');
          (tempErr as any).responseCode = 451;
          callback(tempErr);
        }
      })
      .catch((parseErr: Error) => {
        logger.error('Email parsing failed', { sessionId, error: parseErr.message });
        
        // 550 = permanent failure (malformed email, don't retry)
        const permErr = new Error('Unable to parse email content');
        (permErr as any).responseCode = 550;
        callback(permErr);
      });
  },

  // -------------------------------------------------------------------------
  // 5. CLOSE HANDLER (cleanup)
  // -------------------------------------------------------------------------
  onClose(session: SMTPServerSession) {
    logger.info('Client disconnected', { sessionId: session.id });
  },
});

// Error handler
server.on('error', (err: Error) => {
  logger.error('SMTP server error', { error: err.message, stack: err.stack });
});

// Start listening
server.listen(SMTP_PORT, HOST, () => {
  logger.info(`✅ Inbound SMTP Server listening on ${HOST}:${SMTP_PORT}`);
  logger.info(`   Accepted domains: ${ACCEPTED_DOMAINS.join(', ')}`);
  logger.info(`   Forwarding to: ${WEBHOOK_URL}`);
});

// ============================================================================
// GRACEFUL SHUTDOWN
// ============================================================================
const shutdown = (signal: string) => {
  logger.info(`Received ${signal}, shutting down gracefully...`);
  server.close(() => {
    logger.info('SMTP server closed');
    process.exit(0);
  });
  
  // Force exit after 10 seconds
  setTimeout(() => {
    logger.warn('Forced shutdown after timeout');
    process.exit(1);
  }, 10000);
};

process.on('SIGTERM', () => shutdown('SIGTERM'));
process.on('SIGINT', () => shutdown('SIGINT'));
