# syntax=docker/dockerfile:1.7

FROM rust:1.95.0-slim AS builder

ARG CARGO_BUILD_JOBS=8
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
    --mount=type=cache,target=/app/target,sharing=locked \
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

FROM debian:bookworm-slim

ARG BIN

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /usr/local/bin/coat-services/ /usr/local/bin/coat-services/
RUN cp "/usr/local/bin/coat-services/${BIN}" /usr/local/bin/service \
    && rm -rf /usr/local/bin/coat-services
EXPOSE 9080
CMD ["/usr/local/bin/service"]
