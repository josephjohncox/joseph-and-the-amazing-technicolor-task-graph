# Local Development

## Build

```sh
cargo check --workspace
cargo test --workspace
cargo run -p jattg-domain --bin generate-schemas -- schemas
```

## Compose

```sh
docker compose -f infra/compose/docker-compose.yml config
docker compose -f infra/compose/docker-compose.yml up --build
```

Restate ingress is exposed on `http://localhost:8080`.
The coordinator service listens internally on `http://coordinator:9080`.

## CLI

```sh
cargo run -p jattg-cli -- init
cargo run -p jattg-cli -- goal submit --title "Smoke" --objective "Run a stub task"
cargo run -p jattg-cli -- runner register --file examples/runner-vllm.json
cargo run -p jattg-cli -- notify --file examples/notification-approval.json
cargo run -p jattg-cli -- k8s render --output infra/k8s/rendered.yaml
```

## Live Agent Gates

Use stub sidecars until credentials and local daemons are configured.

- `CODEX_RUNNER_MODE=stub`
- `STAFF_ENGINEER_RUNNER_MODE=stub`

Do not enable live code execution without isolated workspaces and an explicit sandbox profile.
