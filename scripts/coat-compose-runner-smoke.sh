#!/bin/sh
set -eu

# Optional Compose-backed smoke for the default runner registry and sidecar pool.
#
# This intentionally stays separate from scripts/coat-runner-registry-smoke.sh:
# the registry smoke is fast and no-Docker, while this check proves the Compose
# service names, sidecar auto-registration, dispatch routing, and capacity-plan
# endpoint work together.

fail() {
  printf 'compose runner smoke failed: %s\n' "$*" >&2
  exit 1
}

log() {
  printf '[compose-runner-smoke] %s\n' "$*"
}

skip() {
  if [ "${COAT_COMPOSE_RUNNER_SMOKE_REQUIRE_DOCKER:-0}" = "1" ]; then
    fail "$*"
  fi
  log "SKIP: $*"
  exit 0
}

need_command() {
  command -v "$1" >/dev/null 2>&1 || fail "missing required command: $1"
}

script_dir=$(CDPATH= cd "$(dirname "$0")" && pwd)
repo_root=$(CDPATH= cd "$script_dir/.." && pwd)
cd "$repo_root"

if [ "${COAT_COMPOSE_RUNNER_SMOKE_SKIP:-0}" = "1" ]; then
  skip "COAT_COMPOSE_RUNNER_SMOKE_SKIP=1"
fi

if [ ! -f infra/compose/docker-compose.yml ]; then
  fail "missing infra/compose/docker-compose.yml; run from a COAT checkout"
fi

if ! command -v docker >/dev/null 2>&1; then
  skip "docker is not installed"
fi

if ! docker ps >/dev/null 2>&1; then
  skip "Docker daemon is not available"
fi

if ! docker compose version >/dev/null 2>&1; then
  skip "docker compose plugin is not available"
fi

need_command curl
need_command grep
need_command python3

project=${COAT_COMPOSE_RUNNER_SMOKE_PROJECT:-coat-runner-smoke}
compose_file=infra/compose/docker-compose.yml
registry_url=${COAT_COMPOSE_RUNNER_SMOKE_REGISTRY_URL:-http://127.0.0.1:9085}
services=${COAT_COMPOSE_RUNNER_SMOKE_SERVICES:-"runner-registry codex-runner codex-reviewer-runner claude-code-runner model-provider-runner model-provider-research-runner model-provider-local-runner staff-engineer-runner"}
expected_runner_ids=${COAT_COMPOSE_RUNNER_SMOKE_RUNNERS:-"codex-runner-ts codex-reviewer-runner-ts claude-code-runner-ts model-provider-runner-ts model-provider-research-runner-ts model-provider-local-runner-ts staff-engineer-runner-ts"}

tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/coat-compose-runner-smoke.XXXXXX")
started=0

compose() {
  CODEX_RUNNER_MODE=stub \
    CODEX_REVIEW_RUNNER_MODE=stub \
    CLAUDE_CODE_RUNNER_MODE=stub \
    MODEL_PROVIDER_RUNNER_MODE=stub \
    MODEL_PROVIDER_RESEARCH_RUNNER_MODE=stub \
    MODEL_PROVIDER_LOCAL_RUNNER_MODE=stub \
    STAFF_ENGINEER_RUNNER_MODE=stub \
    COAT_WEB_SEARCH_ENABLED=false \
    docker compose --project-name "$project" -f "$compose_file" "$@"
}

cleanup() {
  status=$?
  if [ "$status" -ne 0 ]; then
    printf '\ncompose runner smoke artifacts: %s\n' "$tmpdir" >&2
    if [ "$started" = "1" ]; then
      printf '\nrecent Compose logs:\n' >&2
      compose logs --no-color --tail=160 $services >&2 || true
    fi
  fi
  if [ "$started" = "1" ] && [ "${COAT_COMPOSE_RUNNER_SMOKE_KEEP:-0}" != "1" ]; then
    compose down -v >/dev/null 2>&1 || true
  fi
  rm -rf "$tmpdir"
}
trap cleanup EXIT INT TERM

log "validating Compose service names in $compose_file"
compose config --services >"$tmpdir/services.txt"
for service in $services; do
  grep -qx "$service" "$tmpdir/services.txt" || fail "Compose service is missing: $service"
done

if [ "${COAT_COMPOSE_RUNNER_SMOKE_SKIP_UP:-0}" != "1" ]; then
  log "starting Compose services in project $project"
  started=1
  compose up --build --detach $services
else
  log "using already-running Compose services because COAT_COMPOSE_RUNNER_SMOKE_SKIP_UP=1"
fi

wait_http() {
  name=$1
  url=$2
  attempts=${3:-90}
  attempt=1
  while [ "$attempt" -le "$attempts" ]; do
    if curl -fsS "$url" >"$tmpdir/$name.json" 2>"$tmpdir/$name.err"; then
      log "$name responded at $url"
      return 0
    fi
    sleep 2
    attempt=$((attempt + 1))
  done
  if [ -s "$tmpdir/$name.err" ]; then
    printf '\nlast %s curl error:\n' "$name" >&2
    sed -n '1,40p' "$tmpdir/$name.err" >&2 || true
  fi
  fail "$name did not respond at $url"
}

wait_registry_status() {
  attempts=${1:-90}
  attempt=1
  while [ "$attempt" -le "$attempts" ]; do
    if curl -fsS "$registry_url/runners/status" >"$tmpdir/status.json" 2>"$tmpdir/status.err" \
      && python3 - "$tmpdir/status.json" $expected_runner_ids <<'PY'
import json
import sys

with open(sys.argv[1]) as handle:
    statuses = {
        item["registration"]["runner_id"]: item
        for item in json.load(handle)
    }
expected = sys.argv[2:]
missing = [runner_id for runner_id in expected if runner_id not in statuses]
not_dispatchable = [
    runner_id
    for runner_id in expected
    if runner_id in statuses and not statuses[runner_id].get("dispatchable")
]
sys.exit(0 if not missing and not not_dispatchable else 1)
PY
    then
      log "registry reports the expected sidecar runners as dispatchable"
      return 0
    fi
    sleep 2
    attempt=$((attempt + 1))
  done
  python3 - "$tmpdir/status.json" $expected_runner_ids <<'PY'
import json
import sys

with open(sys.argv[1]) as handle:
    statuses = {
        item["registration"]["runner_id"]: item
        for item in json.load(handle)
    }
expected = sys.argv[2:]
missing = [runner_id for runner_id in expected if runner_id not in statuses]
not_dispatchable = [
    runner_id
    for runner_id in expected
    if runner_id in statuses and not statuses[runner_id].get("dispatchable")
]
raise SystemExit(
    f"missing runners={missing}; not_dispatchable={not_dispatchable}; seen={sorted(statuses)}"
)
PY
}

wait_http runner-registry-health "$registry_url/healthz" 90
wait_http codex-capabilities http://127.0.0.1:9091/capabilities 90
wait_http staff-engineer-capabilities http://127.0.0.1:9092/capabilities 90
wait_http model-provider-capabilities http://127.0.0.1:9093/capabilities 90
wait_http claude-code-capabilities http://127.0.0.1:9094/capabilities 90
wait_registry_status 90

cat >"$tmpdir/dispatch.json" <<'JSON'
{
  "goal_id": "018f8f2f-1fd8-7688-bb12-8bfb6b756601",
  "coordinator_node_id": "compose-smoke-control-plane",
  "task": {
    "id": "018f8f2f-1fd8-7688-bb12-8bfb6b756602",
    "parent_id": null,
    "goal_id": "018f8f2f-1fd8-7688-bb12-8bfb6b756601",
    "depth": 0,
    "status": "runnable",
    "role": "planner",
    "purpose": {
      "kind": "work"
    },
    "execution": {
      "runner": {
        "worker": "planner",
        "runner_id": "codex-runner-ts",
        "required_capabilities": ["code", "mcp_tools"],
        "required_labels": {
          "pool": "default",
          "runtime": "codex"
        },
        "locality": "any_node"
      },
      "model": {
        "strategy": "first_available",
        "required_features": ["tool_use", "json_schema"],
        "candidates": [
          {
            "provider": "codex",
            "model": "codex-default",
            "endpoint": null,
            "priority": 100,
            "weight": 1,
            "context_window": null,
            "features": ["tool_use", "json_schema"],
            "labels": {}
          }
        ],
        "fallback": "disallow_fallback"
      },
      "persona": {
        "name": "planner",
        "instructions_ref": null,
        "inline_instructions": [],
        "risk_tolerance": "conservative"
      },
      "mcp": {
        "context_id": null,
        "servers": [],
        "secret_refs": [],
        "propagation": "coordinator_issued",
        "token_ttl_seconds": 900
      },
      "notifications": {
        "events": [],
        "targets": [],
        "feedback_thread_key": "compose-runner-smoke",
        "escalation_seconds": 600
      }
    },
    "prompt": "Prove the Compose runner registry routes to the requested Codex runner.",
    "dependencies": [],
    "children": [],
    "budget": {
      "max_tokens": 200000,
      "remaining_tokens": 200000,
      "max_runtime_seconds": 600,
      "remaining_runtime_seconds": 600,
      "max_tool_calls": 50,
      "remaining_tool_calls": 50,
      "max_child_tasks": 8,
      "remaining_child_tasks": 8,
      "max_patch_size": 50000
    },
    "sandbox": {
      "filesystem": "workspace_write",
      "network": "restricted",
      "approval_policy": "on_request",
      "isolated_runner": true
    },
    "done_criteria": {
      "tests_pass": false,
      "artifact_exists": true,
      "validator_score_min": 0.85
    },
    "result": null,
    "attempts": 0
  },
  "registered_runners": []
}
JSON

cat >"$tmpdir/capacity-plan.json" <<'JSON'
{
  "generated_at_unix_seconds": 1,
  "policy": {
    "enabled": true,
    "mode": "provision_ephemeral",
    "max_runners": 12,
    "slots_per_runner": 1,
    "target_backlog_per_runner": 2,
    "max_scale_up_step": 2,
    "scale_from_events": true,
    "event_weight": 1
  },
  "demands": [
    {
      "pool_key": "default",
      "worker": "planner",
      "required_capabilities": ["code"],
      "required_labels": {
        "pool": "default"
      },
      "queued_tasks": 30,
      "running_tasks": 0,
      "blocked_tasks": 0,
      "unmatched_tasks": 1,
      "event_backlog": 1,
      "priority_boost": 0
    }
  ],
  "supplies": []
}
JSON

log "checking registry dispatch and capacity-plan behavior"
curl -fsS -X POST "$registry_url/dispatch" \
  -H 'content-type: application/json' \
  --data @"$tmpdir/dispatch.json" >"$tmpdir/dispatch-response.json"
curl -fsS -X POST "$registry_url/capacity/plan" \
  -H 'content-type: application/json' \
  --data @"$tmpdir/capacity-plan.json" >"$tmpdir/capacity-response.json"

python3 - "$tmpdir" $expected_runner_ids <<'PY'
import json
import pathlib
import sys

tmpdir = pathlib.Path(sys.argv[1])
expected = sys.argv[2:]

def load(name):
    with (tmpdir / name).open() as handle:
        return json.load(handle)

def require(condition, message):
    if not condition:
        raise SystemExit(message)

status_items = load("status.json")
statuses = {
    item["registration"]["runner_id"]: item
    for item in status_items
}
for runner_id in expected:
    require(runner_id in statuses, f"missing runner status for {runner_id}")
    require(statuses[runner_id]["stale"] is False, statuses[runner_id])
    require(statuses[runner_id]["full"] is False, statuses[runner_id])
    require(statuses[runner_id]["dispatchable"] is True, statuses[runner_id])

for name, runner_id, runtime in [
    ("codex-capabilities.json", "codex-runner-ts", "codex"),
    ("staff-engineer-capabilities.json", "staff-engineer-runner-ts", "staff-engineer"),
    ("model-provider-capabilities.json", "model-provider-runner-ts", "model-provider"),
    ("claude-code-capabilities.json", "claude-code-runner-ts", "claude-code"),
]:
    capabilities = load(name)
    require(capabilities["runner_id"] == runner_id, capabilities)
    require(capabilities["mode"] == "stub", capabilities)
    endpoints = capabilities.get("endpoints", [])
    require("/run-task" in endpoints and "/capabilities" in endpoints, capabilities)
    labels = capabilities["registration"]["labels"]
    require(labels.get("runtime") == runtime, labels)

dispatch = load("dispatch-response.json")
require(dispatch["status"] == "matched", dispatch)
require(dispatch["runner_id"] == "codex-runner-ts", dispatch)
require(dispatch["runner_endpoint"] == "http://codex-runner:9091", dispatch)
require(dispatch["candidates"][0]["runner_id"] == "codex-runner-ts", dispatch["candidates"])
require(any("selected runner codex-runner-ts" in reason for reason in dispatch["reasons"]), dispatch)

capacity = load("capacity-response.json")
require(capacity["mode"] == "provision_ephemeral", capacity)
require(capacity["status"] != "noop", capacity)
default_pool = next(
    (pool for pool in capacity["pool_decisions"] if pool["pool_key"] == "default"),
    None,
)
require(default_pool is not None, capacity)
require(default_pool["current_runners"] >= len(expected), default_pool)
require(default_pool["current_slots"] >= len(expected), default_pool)
require(default_pool["demand_units"] == 32, default_pool)

print("compose runner smoke assertions passed")
PY

log "passed"
