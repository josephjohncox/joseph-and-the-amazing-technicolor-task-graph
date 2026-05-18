#!/usr/bin/env sh
set -eu

root="${1:-.}"

search_repo() {
  pattern="$1"
  output="$2"
  mode="${3:-default}"

  if command -v rg >/dev/null 2>&1; then
    case "$mode" in
      include-schemas)
        rg -n "$pattern" "$root" \
          --hidden \
          --glob '!target/**' \
          --glob '!.git/**' \
          --glob '!sidecars/**/node_modules/**' \
          --glob '!ui/control-plane-web/node_modules/**' \
          --glob '!ui/control-plane-web/dist/**' \
          --glob '!infra/k8s/rendered.yaml' \
          --glob '!scripts/coat-doc-gardener.sh' >"$output"
        ;;
      *)
        rg -n "$pattern" "$root" \
          --hidden \
          --glob '!target/**' \
          --glob '!.git/**' \
          --glob '!sidecars/**/node_modules/**' \
          --glob '!ui/control-plane-web/node_modules/**' \
          --glob '!ui/control-plane-web/dist/**' \
          --glob '!schemas/**' \
          --glob '!infra/k8s/rendered.yaml' \
          --glob '!scripts/coat-doc-gardener.sh' >"$output"
        ;;
    esac
    return $?
  fi

  case "$mode" in
    include-schemas)
      git -C "$root" grep -n -E "$pattern" -- . \
        ':(exclude)target/**' \
        ':(exclude)sidecars/**/node_modules/**' \
        ':(exclude)ui/control-plane-web/node_modules/**' \
        ':(exclude)ui/control-plane-web/dist/**' \
        ':(exclude)infra/k8s/rendered.yaml' \
        ':(exclude)scripts/coat-doc-gardener.sh' >"$output"
      ;;
    *)
      git -C "$root" grep -n -E "$pattern" -- . \
        ':(exclude)target/**' \
        ':(exclude)sidecars/**/node_modules/**' \
        ':(exclude)ui/control-plane-web/node_modules/**' \
        ':(exclude)ui/control-plane-web/dist/**' \
        ':(exclude)schemas/**' \
        ':(exclude)infra/k8s/rendered.yaml' \
        ':(exclude)scripts/coat-doc-gardener.sh' >"$output"
      ;;
  esac
}

required_paths="
AGENTS.md
Agent.md
ARCHITECTURE.md
README.md
docs/README.md
docs/product-specs/coat-v1.md
docs/operations/goal-authoring.md
docs/operations/runner-context-initialization.md
docs/operations/ephemeral-kubernetes-runners.md
docs/design-docs/030-distributed-memory-knowledgebases.md
docs/design-docs/060-result-channels-git-object-storage.md
docs/design-docs/070-protobuf-goal-store-protocols.md
docs/design-docs/080-events-webhooks-schedules.md
docs/design-docs/090-review-doctrine-stdlib.md
docs/design-docs/100-strong-sandboxing-guardrails.md
docs/design-docs/110-control-gateway-spa.md
docs/design-docs/120-durable-planning-mode.md
docs/operations/restate-cloud.md
docs/operations/local-observability.md
docs/operations/model-runner-clusters.md
proto/coat/v1/common.proto
proto/coat/v1/goal_store.proto
proto/coat/v1/eventing.proto
infra/db/migrations/001_goal_store.sql
infra/helm/jattg/values-ephemeral-example.yaml
"

for path in $required_paths; do
  if [ ! -f "$root/$path" ]; then
    printf 'missing required documentation or contract path: %s\n' "$path" >&2
    exit 1
  fi
done

agents_source_paths="$(
  awk '
    /^## Source Of Truth$/ { in_section = 1; next }
    in_section && /^Update docs/ { exit }
    in_section {
      line = $0
      while (match(line, /`[^`]+`/)) {
        print substr(line, RSTART + 1, RLENGTH - 2)
        line = substr(line, RSTART + RLENGTH)
      }
    }
  ' "$root/AGENTS.md"
)"

for path in $agents_source_paths; do
  case "$path" in
    */)
      if [ ! -d "$root/${path%/}" ]; then
        printf 'AGENTS.md Source Of Truth directory does not exist: %s\n' "$path" >&2
        exit 1
      fi
      ;;
    *)
      if [ ! -e "$root/$path" ]; then
        printf 'AGENTS.md Source Of Truth path does not exist: %s\n' "$path" >&2
        exit 1
      fi
      ;;
  esac
done

if search_repo "infra/helm/coat|coat-agent-toolbox|coat/agent-toolbox|coat-config|coat-agent-secrets|coat-sandboxes|coat-ephemeral|coat-models|coat\.dev/" /tmp/coat-doc-gardener-stale.txt include-schemas; then
  cat /tmp/coat-doc-gardener-stale.txt >&2
  printf 'stale deployment-surface coat slug references found\n' >&2
  exit 1
fi

if search_repo "coat (compose|k8s|approve|notify|follow-ups)([^[:alnum:]_-]|$)|cargo run -p (coat-cli|jattg-cli)|jattg-cli|JATTG_|COAT_RUNNER_REGISTRY([^[:alnum:]_-]|$)" /tmp/coat-doc-gardener-commands.txt; then
  cat /tmp/coat-doc-gardener-commands.txt >&2
  printf 'stale COAT command hierarchy, package, or env-var references found\n' >&2
  exit 1
fi

ambiguous_lane_pattern='(^|[^[:alnum:]_-])((runner|agent|task|model|provider|work|research|chat|embedding|implementation|smoke|fast|deep review|xhigh reasoning|speed tier)[ -]lanes?|lanes? (use|uses|selected|stub|stubbed|live|advertise|configured))([^[:alnum:]_-]|$)'
if command -v rg >/dev/null 2>&1; then
  if rg -n -i "$ambiguous_lane_pattern" \
    "$root/AGENTS.md" \
    "$root/docs" \
    "$root/crates/cli/src/main.rs" \
    "$root/ui/control-plane-web/src" \
    "$root/examples" \
    --glob '!docs/exec-plans/completed/**' \
    >/tmp/coat-doc-gardener-lanes.txt; then
    cat /tmp/coat-doc-gardener-lanes.txt >&2
    printf 'ambiguous lane terminology found; use runner, model route, task, or workstream in user-facing copy\n' >&2
    exit 1
  fi
else
  if git -C "$root" grep -n -i -E "$ambiguous_lane_pattern" -- \
    AGENTS.md \
    docs \
    crates/cli/src/main.rs \
    ui/control-plane-web/src \
    examples \
    ':(exclude)docs/exec-plans/completed/**' \
    >/tmp/coat-doc-gardener-lanes.txt; then
    cat /tmp/coat-doc-gardener-lanes.txt >&2
    printf 'ambiguous lane terminology found; use runner, model route, task, or workstream in user-facing copy\n' >&2
    exit 1
  fi
fi

check_command_line() {
  command_line="$1"
  if ! grep -Fq "$command_line" "$root/crates/cli/src/main.rs"; then
    printf 'canonical command missing from CLI command map: %s\n' "$command_line" >&2
    exit 1
  fi
  if ! grep -Fq "$command_line" "$root/docs/operations/cli.md"; then
    printf 'canonical command missing from docs/operations/cli.md: %s\n' "$command_line" >&2
    exit 1
  fi
}

check_command_line 'coat plan <draft|list|show|revise|compile|follow-ups>'
check_command_line 'coat goal <draft|lint|submit|list|progress|compute-graph|tasks|steer|vote|adversarial|mechanism|thunk|branch|restart|cancel>'
check_command_line 'coat human <approve|resume-thunk|notify>'
check_command_line 'coat deploy local <preflight|up|config|logs|down>'
check_command_line 'coat deploy cluster <render|apply|status|ephemeral-jobs|executor-job>'
check_command_line 'coat deploy chart <lint|template|upgrade|rollback|package>'
check_command_line 'coat deploy restate <cloud-env|tunnel-docker|register-cloud>'
check_command_line 'coat runner <list|status|register|dispatch|capacity-plan>'
check_command_line 'coat tool <list|call|web-search>'
check_command_line 'coat memory <write|search|context|join|retract|edit|preview-edit|repair|events>'
check_command_line 'coat event <sources|register|ingest|emit|webhook|poll-sqs|trigger|triggers>'
check_command_line 'coat store <policy|goals|plans|tasks|events|operator-events|artifacts|checkpoints|approvals>'
check_command_line 'coat scenario <list|run|seed|report>'
check_command_line 'coat setup <login|sso|model-index|config|local-auth|chat-client>'
check_command_line 'coat tui'

if [ -f "$root/.envrc" ] && grep -nE '^export COAT_(RESTATE|COORDINATOR|SANDBOX|RUNNER|NOTIFIER|MEMORY|GOAL_STORE|EVENT|CONTROL)_' "$root/.envrc" >/tmp/coat-doc-gardener-direnv.txt; then
  cat /tmp/coat-doc-gardener-direnv.txt >&2
  printf 'direnv must not duplicate COAT service endpoint defaults; use .coat/project.json or ~/.coat/config.json\n' >&2
  exit 1
fi

plan_count="$(find "$root/docs/exec-plans/active" -maxdepth 1 -type f -name '*.md' | wc -l | tr -d ' ')"
if [ "$plan_count" -gt 0 ]; then
  for plan in "$root"/docs/exec-plans/active/*.md; do
    if ! grep -q '^## Follow-Ups$' "$plan"; then
      printf 'active execution plan missing ## Follow-Ups: %s\n' "$plan" >&2
      exit 1
    fi
  done
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
sidecars/claude-code-runner-ts/src/index.ts
sidecars/model-provider-runner-ts/src/index.ts
sidecars/staff-engineer-runner-ts/src/index.ts
ui/control-plane-web/src/server.ts
ui/control-plane-web/src/spa/App.tsx
ui/control-plane-web/src/spa/api.ts
"

for path in $documented_entrypoints; do
  if ! grep -q "Architecture reference" "$root/$path"; then
    printf 'missing architecture reference header in: %s\n' "$path" >&2
    exit 1
  fi
done

printf 'coat doc gardening checks passed\n'
