# Local Observability

Local COAT runs should be easy to inspect without changing service code. The
Compose profile defaults to debug-level request and task logs for COAT services
while keeping durable state in Restate and projections.

## Local Defaults

`infra/compose/docker-compose.yml` applies shared logging defaults to Rust
services and TypeScript sidecars:

- `COAT_LOG_LEVEL=debug` for local Rust service targets.
- `COAT_NODE_LOG_LEVEL=debug` for local sidecars and the control gateway.
- `COAT_LOG_FORMAT=compact` for human-readable terminal logs.
- `COAT_LOG_ANSI=true` for readable local color output.
- `COAT_RUST_LOG=...` for explicit Rust module filters when needed.

`infra/compose/local-providers.env.example` documents the same knobs so a local
env file can override them without editing Compose.

## Reading Logs

Use the CLI instead of hand-authoring the Compose command:

```sh
coat deploy local logs --tail 200 coordinator runner-registry control-web
coat deploy local logs --follow coordinator runner-registry control-web
```

The command resolves the same configured env files, Compose profiles, and
Restate Cloud overlay selection as `coat deploy local up`.

## Useful Filters

For focused local debugging:

```sh
COAT_RUST_LOG=info,tower_http=debug,coat_coordinator=trace,coat_runner_registry=debug \
  coat deploy local up --allow-stub-runners
```

For structured log collectors or test artifacts:

```sh
COAT_LOG_FORMAT=json COAT_LOG_ANSI=false coat deploy local up --allow-stub-runners
```

For source file and line numbers:

```sh
COAT_LOG_FILE=true COAT_LOG_LINE=true coat deploy local logs --follow coordinator
```

## Service Expectations

Rust services initialize tracing through `crates/observability`. Each process
logs the effective filter and format at startup, then emits request, routing,
projection, validation, approval, and worker-dispatch events through normal
`tracing` spans and fields.

TypeScript sidecars and the control gateway log request start/finish, status,
duration, runner identity, node identity, provider mode, and task start/finish.
They do not log raw provider tokens, MCP auth material, user delegated tokens, or
full prompt payloads.

## Production Posture

Local debug defaults are intentionally chatty. Cluster and Helm deployments
should set `COAT_LOG_LEVEL=info`, `COAT_NODE_LOG_LEVEL=info`, and
`COAT_LOG_ANSI=false`, then route logs through the platform collector. Use JSON
logs when a collector expects structured records.
