#!/usr/bin/env sh
set -eu

docker compose -f infra/compose/docker-compose.yml config >/dev/null
cargo run -p jattg-cli -- k8s render --output infra/k8s/rendered.yaml >/dev/null
if command -v kubectl >/dev/null 2>&1; then
  if ! kubectl apply --dry-run=client -f infra/k8s/rendered.yaml >/dev/null; then
    echo "kubectl dry-run skipped or failed because the current kubeconfig is unavailable" >&2
  fi
fi
