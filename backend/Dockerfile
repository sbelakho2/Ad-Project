# Stage 1: Build dependencies
FROM node:20-alpine AS deps
WORKDIR /app
COPY package.json package-lock.json ./
# Prefer deterministic install; if lockfile incompatible, fall back to npm install
RUN if [ -f package-lock.json ]; then npm ci || npm install; else npm install; fi
# Ensure @aws-sdk/credential-providers is present for verifyStorage build; install if missing
RUN node -e "try{const p=require('./package.json'); if(!(p.dependencies&&p.dependencies['@aws-sdk/credential-providers'])) process.exit(1);}catch(e){process.exit(1)}" \
    || npm install @aws-sdk/credential-providers@^3.922.0 --save
# Ensure type definitions are present (may be hoisted in workspace context)
RUN npm install @types/node@^20.19.0 @types/jest@^29.5.0 --save-dev 2>/dev/null || true

# Stage 2: Build source
FROM node:20-alpine AS build
WORKDIR /app
COPY --from=deps /app/node_modules ./node_modules
COPY . .
RUN npm run build

# Stage 3: Production
FROM node:20-alpine AS production
WORKDIR /app
ENV NODE_ENV=production
# Install curl for HEALTHCHECKs (alpine images do not include wget/curl by default)
RUN apk add --no-cache curl
COPY --from=deps /app/node_modules ./node_modules
COPY --from=build /app/dist ./dist
COPY --from=build /app/package.json ./
EXPOSE 8080
CMD ["node", "dist/src/index.js"]

HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 \
  CMD curl -fsS http://localhost:8080/health > /dev/null || exit 1
