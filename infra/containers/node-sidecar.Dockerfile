FROM node:24-slim AS builder

ARG SIDECAR_DIR
WORKDIR /app
COPY ${SIDECAR_DIR}/ ./
COPY docs/exec-plans/active ./docs/exec-plans/active
RUN npm install && npm run build

FROM node:24-slim

WORKDIR /app
COPY --from=builder /app/package.json ./
COPY --from=builder /app/node_modules ./node_modules
COPY --from=builder /app/dist ./dist
COPY --from=builder /app/docs ./docs
CMD ["npm", "start"]
