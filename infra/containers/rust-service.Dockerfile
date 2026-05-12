# syntax=docker/dockerfile:1.7

# Keep the Rust builder on the same Debian suite as the service and toolbox
# runtime images. The unqualified `rust:*‑slim` tag can move to a newer Debian
# release and produce binaries that require a newer glibc than bookworm has.
FROM rust:1.95.0-slim-bookworm AS builder

ARG CARGO_BUILD_JOBS=8
ARG TARGETOS
ARG TARGETARCH
ARG TARGETVARIANT
ENV CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS} \
    CARGO_INCREMENTAL=0 \
    RUSTUP_TOOLCHAIN=1.95.0

WORKDIR /app
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,id=coat-target-bookworm-${TARGETOS}-${TARGETARCH}${TARGETVARIANT},target=/app/target,sharing=locked \
    cargo build --release --locked -j "${CARGO_BUILD_JOBS}" \
        -p coat-cli \
        -p coat-coordinator \
        -p coat-event-gateway \
        -p coat-goal-store \
        -p coat-memory-gateway \
        -p coat-notifier \
        -p coat-runner-registry \
        -p coat-sandbox-runner \
        -p coat-tool-registry \
        -p coat-validator \
    && mkdir -p /usr/local/bin/coat-services \
    && cp target/release/coat /usr/local/bin/coat-services/coat \
    && cp target/release/coat-coordinator /usr/local/bin/coat-services/coat-coordinator \
    && cp target/release/coat-event-gateway /usr/local/bin/coat-services/coat-event-gateway \
    && cp target/release/coat-goal-store /usr/local/bin/coat-services/coat-goal-store \
    && cp target/release/coat-memory-gateway /usr/local/bin/coat-services/coat-memory-gateway \
    && cp target/release/coat-notifier /usr/local/bin/coat-services/coat-notifier \
    && cp target/release/coat-runner-registry /usr/local/bin/coat-services/coat-runner-registry \
    && cp target/release/coat-sandbox-runner /usr/local/bin/coat-services/coat-sandbox-runner \
    && cp target/release/coat-tool-registry /usr/local/bin/coat-services/coat-tool-registry \
    && cp target/release/coat-validator /usr/local/bin/coat-services/coat-validator

FROM node:24-bookworm-slim AS sidecar-builder

WORKDIR /app
COPY sidecars ./sidecars
RUN --mount=type=cache,id=coat-toolbox-sidecars-npm,target=/root/.npm,sharing=locked \
    for dir in \
      sidecars/codex-runner-ts \
      sidecars/claude-code-runner-ts \
      sidecars/model-provider-runner-ts \
      sidecars/staff-engineer-runner-ts; do \
        cd "/app/${dir}" && npm ci && npm run build && npm prune --omit=dev; \
    done

FROM node:24-bookworm-slim AS agent-toolbox

ENV DEBIAN_FRONTEND=noninteractive \
    COAT_INJECTION_DIR=/opt/coat/injections \
    HOME=/workspace \
    NPM_CONFIG_CACHE=/workspace/.npm

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        bash \
        build-essential \
        ca-certificates \
        curl \
        git \
        jq \
        less \
        openssh-client \
        pkg-config \
        procps \
        python3 \
        python3-pip \
        python3-venv \
        ripgrep \
        tini \
        unzip \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /usr/local/bin/coat-services/ /usr/local/bin/
COPY --from=sidecar-builder /app/sidecars /opt/coat/sidecars
COPY infra/containers/jattg-ephemeral-entrypoint.sh /usr/local/bin/jattg-ephemeral-entrypoint

RUN chmod +x /usr/local/bin/jattg-ephemeral-entrypoint \
    && useradd --create-home --home-dir /workspace --uid 1000 --shell /usr/sbin/nologin coat \
    && mkdir -p /workspace /workspaces /opt/coat/injections/env /opt/coat/injections/bin /opt/coat/injections/init.d \
    && chown -R coat:coat /workspace /workspaces /opt/coat/injections

WORKDIR /workspace
ENTRYPOINT ["tini", "--", "/usr/local/bin/jattg-ephemeral-entrypoint"]
CMD ["bash"]

FROM debian:bookworm-slim AS service

ARG BIN

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /usr/local/bin/coat-services/ /usr/local/bin/coat-services/
RUN cp "/usr/local/bin/coat-services/${BIN}" /usr/local/bin/service \
    && rm -rf /usr/local/bin/coat-services
EXPOSE 9080
CMD ["/usr/local/bin/service"]
