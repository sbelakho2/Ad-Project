# apex-sandbox-web

Minimal web sandbox for exercising web SDK flows. Intended for localhost only.

Requirements
- Node.js 20+
- PNPM or NPM

Quick start
```
pnpm install  # or: npm install
pnpm dev      # or: npm run dev
```
Then open http://localhost:5173

Localhost-only guard
- The app checks `window.location.hostname` and refuses to run unless it's `localhost` or `127.0.0.1`.

Staging configuration
- Edit `src/sandboxConfig.ts` for staging endpoints and placement IDs.
