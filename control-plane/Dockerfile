# Control Plane Dockerfile
# Completely separate from customer-facing backend
#
# Build: docker build -t control-plane -f control-plane/Dockerfile .
# Run: docker run -p 4000:4000 --env-file control-plane/.env control-plane

# ============================================
# BUILD STAGE
# ============================================
FROM node:20-alpine AS builder

WORKDIR /app

# Install build dependencies
RUN apk add --no-cache python3 make g++

# Copy package files
COPY control-plane/package*.json ./

# Install all dependencies (including dev)
RUN npm ci

# Copy source code
COPY control-plane/tsconfig.json ./
COPY control-plane/src ./src

# Build TypeScript
RUN npm run build

# ============================================
# PRODUCTION STAGE
# ============================================
FROM node:20-alpine AS production

WORKDIR /app

# Security: Run as non-root user
RUN addgroup -g 1001 -S controlplane && \
    adduser -S controlplane -u 1001

# Install production dependencies only
COPY control-plane/package*.json ./
RUN npm ci --only=production && npm cache clean --force

# Copy built files from builder
COPY --from=builder /app/dist ./dist

# Security: Remove unnecessary files
RUN rm -rf /root/.npm /root/.node-gyp

# Set ownership
RUN chown -R controlplane:controlplane /app

# Switch to non-root user
USER controlplane

# Expose port
EXPOSE 4000

# Health check
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD wget -qO- http://localhost:4000/health || exit 1

# Environment
ENV NODE_ENV=production
ENV PORT=4000

# Start server
CMD ["node", "dist/index.js"]
