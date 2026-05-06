# Joseph And The Amazing Technicolor Task Graph

A durable task-tree control plane for long-running agentic engineering work.

The core idea is simple: Restate owns durable time and replay, Rust owns policy and state, Codex owns bounded code execution, and specialized workers produce structured evidence for the coordinator to validate.

## Quick Start

```sh
cargo test --workspace
cargo run -p jattg-domain --bin generate-schemas -- schemas
cargo run -p jattg-cli -- init
```

Run the local stack:

```sh
docker compose -f infra/compose/docker-compose.yml up --build
```

Submit a stub goal through Restate ingress:

```sh
cargo run -p jattg-cli -- goal submit \
  --title "Smoke goal" \
  --objective "Prove the durable task tree can accept and validate a task"
```

## Services

- `jattg-coordinator`: Restate workflow, agent runner stub, validation handler.
- `jattg-runner-registry`: distributed runner registration, heartbeat, and task dispatch decisions.
- `jattg-notifier`: notification and human-feedback delivery stub.
- `jattg-validator`: standalone validation service.
- `jattg-sandbox-runner`: workspace lifecycle and snapshot placeholder.
- `jattg-tool-registry`: HTTP and MCP-shaped tool registry placeholder.
- `jattg`: operator CLI.
- `codex-runner-ts`: Codex App Server or MCP worker boundary.
- `staff-engineer-runner-ts`: `@ctxr/agent-staff-engineer` worker boundary.

## Distributed Runners

Each durable task has an execution profile with runner selection, model candidates, persona, MCP context refs, and notification policy.

Example local vLLM runner registration:

```sh
cargo run -p jattg-cli -- runner register --file examples/runner-vllm.json
```

MCP auth is passed by reference, not by value. Runners resolve `SecretRef` entries from their local environment, Kubernetes, Vault, or another configured secret backend.

## Documentation

- Architecture: `ARCHITECTURE.md`
- Product spec: `docs/product-specs/jattg-v1.md`
- Execution plans: `docs/exec-plans/active/`
- Operations: `docs/operations/`
- Agent guide: `AGENTS.md`
