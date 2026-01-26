/**
 * PM2 Ecosystem Configuration for App Server
 * 
 * Manages PM2 services on the production app server:
 *   - backend: Main API server (port 8080)
 *   - console: Next.js customer console (port 3000)
 * 
 * NOTE: Control Plane runs via systemd (control-plane.service), NOT PM2.
 *       See: infrastructure/hetzner/control-plane.service
 * 
 * Usage:
 *   pm2 start ecosystem.config.cjs
 *   pm2 restart all
 *   pm2 logs
 *   pm2 monit
 */

module.exports = {
  apps: [
    // =========================================================================
    // Backend API Server
    // =========================================================================
    {
      name: 'backend',
      script: 'dist/index.js',
      cwd: '/opt/adproject/backend',
      interpreter: 'node',
      exec_mode: 'fork',
      instances: 1,
      env: {
        NODE_ENV: 'production',
        PORT: 8080,
      },
      // Load from .env file
      env_file: '/opt/adproject/backend/.env',
      
      // Process management
      watch: false,
      autorestart: true,
      max_restarts: 10,
      restart_delay: 5000,
      exp_backoff_restart_delay: 100,
      
      // Memory management
      max_memory_restart: '1G',
      
      // Logging
      time: true,
      log_date_format: 'YYYY-MM-DD HH:mm:ss.SSS',
      error_file: '/var/log/adproject/backend-error.log',
      out_file: '/var/log/adproject/backend-out.log',
      merge_logs: true,
      
      // Graceful shutdown
      kill_timeout: 30000,
      wait_ready: true,
      listen_timeout: 10000,
    },

    // =========================================================================
    // Console (Next.js Customer Portal)
    // =========================================================================
    {
      name: 'console',
      script: 'server.js',
      cwd: '/opt/adproject/console-run',
      interpreter: 'node',
      exec_mode: 'fork',
      instances: 1,
      env: {
        NODE_ENV: 'production',
        PORT: 3000,
      },
      env_file: '/opt/adproject/console/.env',
      
      // Process management
      watch: false,
      autorestart: true,
      max_restarts: 10,
      restart_delay: 5000,
      
      // Memory management
      max_memory_restart: '800M',
      
      // Logging
      time: true,
      log_date_format: 'YYYY-MM-DD HH:mm:ss.SSS',
      error_file: '/var/log/adproject/console-error.log',
      out_file: '/var/log/adproject/console-out.log',
      merge_logs: true,
      
      // Graceful shutdown
      kill_timeout: 15000,
    },

    // =========================================================================
    // Website (Next.js Marketing Site)
    // =========================================================================
    {
      name: 'website',
      script: 'server.js',
      cwd: '/opt/adproject/website-run',
      interpreter: 'node',
      exec_mode: 'fork',
      instances: 1,
      env: {
        NODE_ENV: 'production',
        PORT: 3001,
      },
      env_file: '/opt/adproject/website/.env',
      watch: false,
      autorestart: true,
      max_restarts: 10,
      restart_delay: 5000,
      max_memory_restart: '500M',
      time: true,
      log_date_format: 'YYYY-MM-DD HH:mm:ss.SSS',
      error_file: '/var/log/adproject/website-error.log',
      out_file: '/var/log/adproject/website-out.log',
      merge_logs: true,
      kill_timeout: 15000,
    },
  ],
};
