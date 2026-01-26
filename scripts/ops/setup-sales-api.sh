#!/bin/bash
set -e

# Create .env file
cat > /opt/adproject/sales-automation/.env << 'ENVEOF'
NODE_ENV=production
PORT=4001
SALES_AUTOMATION_ROLE=execution

# Database via SSH tunnel to App Server PgBouncer (no SSL for tunnel)
DB_HOST=127.0.0.1
DB_PORT=6432
DB_NAME=adproject
DB_USER=adproject
DB_PASSWORD=xP5Qa6z+zov+besvN3Fkx2bKgqkb86qtWDYiClttXBtPB44f
DB_SSL=false

# Operator credentials for write operations
DB_OPERATOR_USER=adproject
DB_OPERATOR_PASSWORD=xP5Qa6z+zov+besvN3Fkx2bKgqkb86qtWDYiClttXBtPB44f

OPERATOR_JWT_SECRET=change-me-in-production-very-long-secret-key-32chars

# API authentication
AUTONOMOUS_API_KEY=test-sales-key-2026

# Edge Worker for ML inference
EDGE_WORKER_URL=http://localhost:3500
EDGE_WORKER_SECRET=3fc99ee8d35b2229c79d5647b34676b7a4c1a7607574ea7dc78d398f9dc6924f
ENVEOF

# Create ecosystem.config.js
cat > /opt/adproject/sales-automation/ecosystem.config.js << 'CONFIGEOF'
module.exports = {
  apps: [{
    name: "sales-api",
    script: "dist/index.js",
    cwd: "/opt/adproject/sales-automation",
    interpreter: "node",
    interpreter_args: "-r dotenv/config",
    env: {
      NODE_ENV: "production",
      PORT: "4001",
      SALES_AUTOMATION_ROLE: "execution",
      DOTENV_CONFIG_PATH: "/opt/adproject/sales-automation/.env",
      DB_HOST: "127.0.0.1",
      DB_PORT: "6432",
      DB_NAME: "adproject",
      DB_USER: "adproject",
      DB_PASSWORD: "xP5Qa6z+zov+besvN3Fkx2bKgqkb86qtWDYiClttXBtPB44f",
      DB_SSL: "false",
      DB_OPERATOR_USER: "adproject",
      DB_OPERATOR_PASSWORD: "xP5Qa6z+zov+besvN3Fkx2bKgqkb86qtWDYiClttXBtPB44f",
      OPERATOR_JWT_SECRET: "change-me-in-production-very-long-secret-key-32chars",
      AUTONOMOUS_API_KEY: "test-sales-key-2026",
      EDGE_WORKER_URL: "http://localhost:3500",
      EDGE_WORKER_SECRET: "3fc99ee8d35b2229c79d5647b34676b7a4c1a7607574ea7dc78d398f9dc6924f"
    }
  }]
};
CONFIGEOF

echo "Config files created."

# Restart PM2
pm2 delete sales-api 2>/dev/null || true
pm2 start /opt/adproject/sales-automation/ecosystem.config.js
sleep 3
pm2 list
echo "Done."
