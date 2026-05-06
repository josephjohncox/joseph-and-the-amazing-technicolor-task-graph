FROM rust:1.95-slim AS builder

ARG PACKAGE
ARG BIN

WORKDIR /app
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

COPY . .
RUN cargo build --release -p "${PACKAGE}" \
    && cp "target/release/${BIN}" /usr/local/bin/service

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /usr/local/bin/service /usr/local/bin/service
EXPOSE 9080
CMD ["/usr/local/bin/service"]
