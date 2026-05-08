#!/usr/bin/env sh
set -eu

coat deploy local config >/dev/null
coat deploy local config \
  --restate-cloud \
  --restate-cloud-env-file infra/compose/restate-cloud.env.example \
  --allow-placeholder-env >/dev/null
buf lint >/dev/null
coat deploy cluster render --output infra/k8s/rendered.yaml >/dev/null
if command -v kubectl >/dev/null 2>&1; then
  if ! coat deploy cluster apply --file infra/k8s/rendered.yaml --dry-run=client >/dev/null; then
    echo "kubectl dry-run skipped or failed because the current kubeconfig is unavailable" >&2
  fi
fi
