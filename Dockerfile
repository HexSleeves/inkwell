# syntax=docker/dockerfile:1

# ---- Stage 1: builder ----------------------------------------------------
# Full toolchain (incl. devDependencies + TypeScript) to compile src -> dist.
FROM node:20 AS builder

WORKDIR /app

# Install dependencies against the lockfile first so this layer caches across
# source-only changes.
COPY package.json package-lock.json ./
RUN npm ci

# Compile TypeScript to dist/ (tsc -p tsconfig.build.json).
COPY tsconfig.json tsconfig.build.json ./
COPY src ./src
RUN npm run build

# Drop devDependencies so the node_modules we ship to runtime is prod-only.
RUN npm prune --omit=dev

# ---- Stage 2: runtime ----------------------------------------------------
# Slim image with only the compiled output and production dependencies.
FROM node:20-slim AS runtime

ENV NODE_ENV=production
WORKDIR /app

# package.json is needed at runtime: "type": "module" governs ESM resolution
# and the db:* scripts resolve the migration CLI.
COPY --from=builder /app/package.json ./package.json
COPY --from=builder /app/node_modules ./node_modules
COPY --from=builder /app/dist ./dist

# Run as the unprivileged user baked into the node image.
USER node

EXPOSE 3000

# Server entrypoint (npm start === node dist/main.js). Reads DATABASE_URL,
# PORT (default 3000), HOST (default 0.0.0.0), and INKWELL_API_KEY from the env.
CMD ["node", "dist/main.js"]
