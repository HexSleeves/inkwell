# syntax=docker/dockerfile:1

# ---- Stage 1: builder ----------------------------------------------------
# Full toolchain (incl. devDependencies + TypeScript) to compile src -> dist.
FROM node:20 AS builder

WORKDIR /app

# Corepack ships the pnpm version pinned in package.json's packageManager.
RUN corepack enable

# Install dependencies against the lockfile first so this layer caches across
# source-only changes.
COPY package.json pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile

# Compile TypeScript to dist/ (tsc -p tsconfig.build.json).
COPY tsconfig.json tsconfig.build.json ./
COPY src ./src
RUN pnpm run build

# ---- Stage 2: runtime ----------------------------------------------------
# Slim image with only the compiled output and production dependencies.
FROM node:20-slim AS runtime

ENV NODE_ENV=production
WORKDIR /app

RUN corepack enable

# package.json is needed at runtime: "type": "module" governs ESM resolution
# and the db:* scripts resolve the migration CLI. Installing prod deps here
# (instead of copying from the builder) keeps pnpm's virtual-store symlinks
# self-contained — Docker's COPY dereferences symlinks, which would break the
# .pnpm layout if node_modules were copied across stages.
COPY package.json pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile --prod

COPY --from=builder /app/dist ./dist

# Run as the unprivileged user baked into the node image.
USER node

EXPOSE 3000

# Server entrypoint (pnpm start === node dist/main.js). Reads DATABASE_URL,
# PORT (default 3000), HOST (default 0.0.0.0), and INKWELL_API_KEY from the env.
CMD ["node", "dist/main.js"]
