module.exports = {
  apps: [
    {
      name: 'website',
      script: 'server.js',
      cwd: '/opt/adproject/website_release/.next/standalone',
      interpreter: 'node',
      exec_mode: 'fork',
      instances: 1,
      env: {
        PORT: 3002,
        NODE_ENV: 'production',
      },
      watch: false,
      max_memory_restart: '300M',
      time: true,
    },
  ],
}
