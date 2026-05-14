# syntax=docker/dockerfile:1.7

FROM node:24-slim AS builder

ARG SIDECAR_DIR
ARG COAT_SOURCE_FINGERPRINT=local
WORKDIR /app
# Keep sidecar images aligned with the local checkout when `coat deploy local
# up` is run, without disabling npm cache mounts.
RUN printf '%s\n' "$COAT_SOURCE_FINGERPRINT" > /tmp/coat-source-fingerprint
COPY ${SIDECAR_DIR}/ ./
COPY docs/exec-plans/active ./docs/exec-plans/active
RUN --mount=type=cache,id=coat-node-sidecar-npm,target=/root/.npm,sharing=locked \
    npm ci && npm run build && npm prune --omit=dev

FROM node:24-slim

WORKDIR /app
COPY --from=builder /app/package.json ./
COPY --from=builder /app/node_modules ./node_modules
COPY --from=builder /app/dist ./dist
COPY --from=builder /app/docs ./docs
CMD ["npm", "start"]
