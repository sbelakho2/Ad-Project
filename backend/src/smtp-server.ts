import { SMTPServer } from "smtp-server";
import { simpleParser } from "mailparser";
import axios from "axios";
import { config } from "dotenv";
import logger from "./utils/logger";

// Load env vars
config();

const WEBHOOK_URL = process.env.INBOUND_EMAIL_WEBHOOK_URL || "http://localhost:4000/api/autonomous/webhook/sendgrid";
const SMTP_PORT = parseInt(process.env.SMTP_PORT || "2525");
const ALLOWED_DOMAINS = (process.env.ALLOWED_INBOUND_DOMAINS || "replies.apexmediation.ee").split(",");

export const startSmtpServer = () => {
  const server = new SMTPServer({
    // Disable authentication (we accept from public internet)
    authOptional: true,
    // We don't need STARTTLS for this simple receiver, but good to have. 
    // For now, let's keep it simple.
    disabledCommands: ["AUTH"],
    
    // Validate recipient
    onRcptTo(address, session, callback) {
        if (!address || !address.address) {
            return callback(new Error("Invalid recipient"));
        }
        
        const domain = address.address.split('@')[1];
        if (ALLOWED_DOMAINS.includes(domain)) {
            return callback(); // Accept
        }
        
        // Check if we want to be open relay? NO.
        // But for "replies.apexmediation.ee", we only accept that domain.
        return callback(new Error("Relay access denied"));
    },

    // Handle the data stream
    onData(stream, session, callback) {
      logger.info(`Receiving email from ${session.envelope.mailFrom.address} to ${session.envelope.rcptTo.map(t => t.address).join(', ')}`);
      
      simpleParser(stream)
        .then(async (parsed) => {
          try {
            // Transform to SendGrid-like payload or custom JSON
            const payload = {
              headers: parsed.headers,
              subject: parsed.subject,
              from: parsed.from?.text,
              to: parsed.to?.text,
              text: parsed.text,
              html: parsed.html,
              raw: "omitted", // or stream it if needed, but usually redundant
              // Add SendGrid compatibility fields if strictly needed
              envelope: session.envelope
            };

            logger.info(`Posting email to ${WEBHOOK_URL}`);
            
            await axios.post(WEBHOOK_URL, payload, {
                // headers: { 'X-Signature': ... } // Could add security here
            });
            
            logger.info("Email processed and webhook delivered successfully");
            callback(); // 250 OK
          } catch (err: any) {
            logger.error(`Webhook delivery failed: ${err.message}`);
            // Return temp failure (4xx) so Sender retries
            callback(new Error("Local processing error, please retry later")); 
          }
        })
        .catch((err) => {
          logger.error(`Parsing failed: ${err.message}`);
          callback(new Error("Message content rejected")); // 5xx
        });
    },
  });

  server.on("error", (err) => {
    logger.error(`SMTP Server Error: ${err.message}`);
  });

  server.listen(SMTP_PORT, () => {
    logger.info(`SMTP Server listening on port ${SMTP_PORT}`);
  });
};

if (require.main === module) {
    startSmtpServer();
}
