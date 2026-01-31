/**
 * PM2 ECOSYSTEM CONFIG FOR SALES API (TEST SERVER)
 * 
 * SECURITY: All sensitive values MUST be read from environment variables
 * on the server. NEVER commit actual secrets to this file.
 * 
 * On the Test Server, create /opt/adproject/control-plane/.env.production
 * with the actual values for all variables marked as process.env.XXX
 * 
 * @see docs/Internal/Deployment/HETZNER_PRODUCTION_CHECKLIST.md
 */
module.exports = {
  apps: [{
    name: "sales-api",
    script: "/opt/adproject/control-plane/dist/index.js",
    cwd: "/opt/adproject/control-plane",
    env: {
      NODE_ENV: "production",
      PORT: "4001",
      SALES_AUTOMATION_ROLE: "execution",
      
      // All secrets MUST be read from environment on the server
      // Configure these in /opt/adproject/control-plane/.env.production
      // or export them before running PM2
      
      // Read-only pool (base access)
      DB_HOST: process.env.DB_HOST || "127.0.0.1",
      DB_PORT: process.env.DB_PORT || "6432",
      DB_NAME: process.env.DB_NAME || "adproject",
      DB_USER: process.env.DB_USER || "adproject",
      DB_PASSWORD: process.env.DB_PASSWORD, // REQUIRED: Set in server .env
      DB_SSL: process.env.DB_SSL || "false",
      
      // Operator pool
      DB_OPERATOR_USER: process.env.DB_OPERATOR_USER,
      DB_OPERATOR_PASSWORD: process.env.DB_OPERATOR_PASSWORD,
      
      // Break-glass pool
      DB_BREAKGLASS_USER: process.env.DB_BREAKGLASS_USER,
      DB_BREAKGLASS_PASSWORD: process.env.DB_BREAKGLASS_PASSWORD,
      
      DATABASE_URL: process.env.DATABASE_URL, // REQUIRED: Set in server .env
      DATABASE_SSL: process.env.DATABASE_SSL || "false",
      
      REDIS_URL: process.env.REDIS_URL, // REQUIRED: Set in server .env
      JWT_SECRET: process.env.JWT_SECRET, // REQUIRED: Set in server .env
      OPERATOR_JWT_SECRET: process.env.OPERATOR_JWT_SECRET, // REQUIRED: Set in server .env
      AUTONOMOUS_API_KEY: process.env.AUTONOMOUS_API_KEY, // REQUIRED: Set in server .env
      EDGE_WORKER_URL: process.env.EDGE_WORKER_URL || "http://localhost:3500",
      EDGE_WORKER_SECRET: process.env.EDGE_WORKER_SECRET, // REQUIRED: Set in server .env
      
      // ApexMail (self-hosted email - replaces Resend)
      SMTP_HOST: process.env.SMTP_HOST,
      SMTP_PORT: process.env.SMTP_PORT || "587",
      SMTP_SECURE: process.env.SMTP_SECURE || "false",
      SMTP_USER: process.env.SMTP_USER,
      SMTP_PASS: process.env.SMTP_PASS,
      EMAIL_FROM: process.env.EMAIL_FROM || "contact@apexmediation.ee",
      EMAIL_FROM_NAME: process.env.EMAIL_FROM_NAME || "ApexMediation",
      APEXMAIL_API_KEY: process.env.APEXMAIL_API_KEY, // For cross-service calls
      
      PUBLIC_ORIGIN: process.env.PUBLIC_ORIGIN || "https://admin.apexmediation.ee"
    },
    watch: false,
    max_memory_restart: "500M",
    error_file: "/opt/adproject/control-plane/logs/error.log",
    out_file: "/opt/adproject/control-plane/logs/out.log",
    merge_logs: true,
    time: true
  }]
};
