#!/usr/bin/env sh
set -eu

root="${1:-.}"

required_paths="
AGENTS.md
Agent.md
ARCHITECTURE.md
README.md
docs/README.md
docs/product-specs/coat-v1.md
docs/operations/goal-authoring.md
docs/design-docs/030-distributed-memory-knowledgebases.md
docs/design-docs/060-result-channels-git-object-storage.md
docs/design-docs/070-protobuf-goal-store-protocols.md
docs/design-docs/080-events-webhooks-schedules.md
docs/design-docs/090-review-doctrine-stdlib.md
docs/design-docs/100-strong-sandboxing-guardrails.md
docs/design-docs/110-control-gateway-spa.md
docs/design-docs/120-durable-planning-mode.md
docs/operations/restate-cloud.md
docs/operations/model-runner-clusters.md
proto/coat/v1/common.proto
proto/coat/v1/goal_store.proto
proto/coat/v1/eventing.proto
infra/db/migrations/001_goal_store.sql
"

for path in $required_paths; do
  if [ ! -f "$root/$path" ]; then
    printf 'missing required documentation or contract path: %s\n' "$path" >&2
    exit 1
  fi
done

if rg -n "[jJ][aA][tT][tT][gG]" "$root" \
  --glob '!target/**' \
  --glob '!sidecars/**/node_modules/**' \
  --glob '!ui/control-plane-web/node_modules/**' \
  --glob '!ui/control-plane-web/dist/**' >/tmp/coat-doc-gardener-stale.txt; then
  cat /tmp/coat-doc-gardener-stale.txt >&2
  printf 'stale pre-coat slug references found\n' >&2
  exit 1
fi

plan_count="$(find "$root/docs/exec-plans/active" -maxdepth 1 -type f -name '*.md' | wc -l | tr -d ' ')"
if [ "$plan_count" -lt 9 ]; then
  printf 'expected active execution plans, found only %s\n' "$plan_count" >&2
  exit 1
fi

documented_entrypoints="
crates/cli/src/main.rs
crates/coordinator/src/main.rs
crates/domain/src/lib.rs
crates/domain/src/bin/generate_schemas.rs
crates/event-gateway/src/main.rs
crates/goal-store/src/main.rs
crates/memory-gateway/src/main.rs
crates/notifier/src/main.rs
crates/runner-registry/src/main.rs
crates/sandbox-runner/src/main.rs
crates/tool-registry/src/main.rs
crates/validator/src/main.rs
sidecars/codex-runner-ts/src/index.ts
sidecars/staff-engineer-runner-ts/src/index.ts
ui/control-plane-web/src/server.ts
ui/control-plane-web/src/client.ts
"

for path in $documented_entrypoints; do
  if ! grep -q "Architecture reference" "$root/$path"; then
    printf 'missing architecture reference header in: %s\n' "$path" >&2
    exit 1
  fi
done

printf 'coat doc gardening checks passed\n'
