#!/bin/sh
set -eu

# Bounded local smoke for the runner registry HTTP surface.
#
# The script uses the checkout-built `coat` and `coat-runner-registry`
# binaries. When run directly it builds them first with `make build` plus the
# registry package build. `make runner-smoke` performs the build once and sets
# COAT_RUNNER_REGISTRY_SMOKE_SKIP_BUILD=1 before invoking this script.

fail() {
  printf 'runner-registry smoke failed: %s\n' "$*" >&2
  exit 1
}

skip() {
  printf '[runner-smoke] SKIP: %s\n' "$*"
  exit 0
}

log() {
  printf '[runner-smoke] %s\n' "$*"
}

need_command() {
  command -v "$1" >/dev/null 2>&1 || fail "missing required command: $1"
}

profile=${COAT_BUILD_PROFILE:-debug}
case "$profile" in
  debug)
    bin_dir=target/debug
    ;;
  release)
    bin_dir=target/release
    ;;
  *)
    fail "COAT_BUILD_PROFILE must be debug or release, got $profile"
    ;;
esac

need_command curl
need_command python3

if [ "${COAT_RUNNER_REGISTRY_SMOKE_SKIP_BUILD:-0}" != "1" ]; then
  need_command make
  need_command cargo
  log "building coat CLI with make build"
  COAT_BUILD_PROFILE="$profile" make build
  log "building coat-runner-registry"
  if [ "$profile" = "release" ]; then
    cargo build -p coat-runner-registry --release
  else
    cargo build -p coat-runner-registry
  fi
fi

coat_bin=${COAT_BIN:-$bin_dir/coat}
registry_bin=${COAT_RUNNER_REGISTRY_BIN:-$bin_dir/coat-runner-registry}

if [ ! -x "$coat_bin" ]; then
  if [ -n "${COAT_BIN:-}" ]; then
    fail "COAT_BIN is not executable: $coat_bin"
  else
    fail "missing checkout-built coat binary at $coat_bin; run make runner-smoke or set COAT_BIN explicitly"
  fi
fi

[ -x "$registry_bin" ] || fail "missing coat-runner-registry binary at $registry_bin"

tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/coat-runner-registry-smoke.XXXXXX")
registry_pid=
registry_log="$tmpdir/runner-registry.log"

cleanup() {
  status=$?
  if [ -n "${registry_pid:-}" ] && kill -0 "$registry_pid" >/dev/null 2>&1; then
    kill "$registry_pid" >/dev/null 2>&1 || true
    wait "$registry_pid" >/dev/null 2>&1 || true
  fi
  if [ "$status" -ne 0 ] && [ -f "$registry_log" ]; then
    printf '\nrunner-registry log:\n' >&2
    sed -n '1,160p' "$registry_log" >&2 || true
  fi
  rm -rf "$tmpdir"
}
trap cleanup EXIT INT TERM

port_file="$tmpdir/port.txt"
port_error="$tmpdir/port.err"
if python3 >"$port_file" 2>"$port_error" <<'PY'
import errno
import socket
import sys

try:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        print(sock.getsockname()[1])
except OSError as exc:
    print(f"unable to allocate localhost port: {exc}", file=sys.stderr)
    if exc.errno in (errno.EACCES, errno.EPERM):
        sys.exit(42)
    sys.exit(1)
PY
then
  :
else
  status=$?
  if [ "$status" -eq 42 ]; then
    skip "$(sed -n '1p' "$port_error")"
  fi
  fail "$(sed -n '1p' "$port_error")"
fi

port=$(sed -n '1p' "$port_file")
[ -n "$port" ] || fail "did not allocate runner-registry port"
registry_url="http://127.0.0.1:$port"
journal_path="$tmpdir/runner-registry.jsonl"

bind_permission_error() {
  [ -f "$1" ] && grep -Eiq 'permission denied|operation not permitted|EACCES|EPERM' "$1"
}

log "starting registry on $registry_url"
BIND_ADDR="127.0.0.1:$port" \
  COAT_RUNNER_REGISTRY_JOURNAL_PATH="$journal_path" \
  "$registry_bin" >"$registry_log" 2>&1 &
registry_pid=$!

attempt=1
while [ "$attempt" -le 50 ]; do
  if curl -fsS "$registry_url/healthz" >/dev/null 2>&1; then
    break
  fi
  if ! kill -0 "$registry_pid" >/dev/null 2>&1; then
    if bind_permission_error "$registry_log"; then
      skip "registry could not bind its localhost port; local port bind is unavailable"
    fi
    fail "registry process exited before health check passed"
  fi
  sleep 0.1
  attempt=$((attempt + 1))
done

if [ "$attempt" -gt 50 ]; then
  if bind_permission_error "$registry_log"; then
    skip "registry could not bind its localhost port; local port bind is unavailable"
  fi
  fail "registry did not become healthy at $registry_url"
fi

cat >"$tmpdir/local-full.json" <<'JSON'
{
  "runner_id": "local-full-smoke",
  "node_id": "control-node",
  "endpoint": "http://local-full-smoke:9091",
  "roles": ["planner"],
  "capabilities": ["code", "test"],
  "models": [
    {
      "provider": "codex",
      "model": "codex-default",
      "endpoint": null,
      "priority": 100,
      "weight": 1,
      "context_window": 200000,
      "features": ["tool_use", "json_schema"],
      "labels": {}
    }
  ],
  "labels": {
    "pool": "ci-smoke",
    "region": "local"
  },
  "mcp_servers": [],
  "max_concurrency": 2,
  "lease_ttl_seconds": 300
}
JSON

cat >"$tmpdir/stale-remote.json" <<'JSON'
{
  "runner_id": "stale-remote-smoke",
  "node_id": "stale-node",
  "endpoint": "http://stale-remote-smoke:9091",
  "roles": ["planner"],
  "capabilities": ["code", "test"],
  "models": [
    {
      "provider": "codex",
      "model": "codex-default",
      "endpoint": null,
      "priority": 90,
      "weight": 1,
      "context_window": 200000,
      "features": ["tool_use", "json_schema"],
      "labels": {}
    }
  ],
  "labels": {
    "pool": "ci-smoke",
    "region": "local"
  },
  "mcp_servers": [],
  "max_concurrency": 2,
  "lease_ttl_seconds": 1
}
JSON

cat >"$tmpdir/active-remote.json" <<'JSON'
{
  "runner_id": "active-remote-smoke",
  "node_id": "active-node",
  "endpoint": "http://active-remote-smoke:9091",
  "roles": ["planner"],
  "capabilities": ["code", "test"],
  "models": [
    {
      "provider": "codex",
      "model": "codex-default",
      "endpoint": null,
      "priority": 80,
      "weight": 1,
      "context_window": 200000,
      "features": ["tool_use", "json_schema"],
      "labels": {}
    }
  ],
  "labels": {
    "pool": "ci-smoke",
    "region": "local"
  },
  "mcp_servers": [],
  "max_concurrency": 2,
  "lease_ttl_seconds": 300
}
JSON

cat >"$tmpdir/local-full-heartbeat.json" <<'JSON'
{
  "runner_id": "local-full-smoke",
  "node_id": "control-node",
  "running_tasks": 2,
  "capacity_remaining": 0
}
JSON

cat >"$tmpdir/stale-remote-heartbeat.json" <<'JSON'
{
  "runner_id": "stale-remote-smoke",
  "node_id": "stale-node",
  "running_tasks": 0,
  "capacity_remaining": 1
}
JSON

cat >"$tmpdir/dispatch.json" <<'JSON'
{
  "goal_id": "018f8f2f-1fd8-7688-bb12-8bfb6b756601",
  "coordinator_node_id": "control-node",
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
        "runner_id": null,
        "required_capabilities": ["code"],
        "required_labels": {
          "pool": "ci-smoke"
        },
        "locality": "remote_only"
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
            "context_window": 200000,
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
        "feedback_thread_key": "runner-registry-smoke",
        "escalation_seconds": 600
      }
    },
    "prompt": "Prove the runner registry selects only the active compatible remote runner.",
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
    "max_runners": 4,
    "slots_per_runner": 2,
    "target_backlog_per_runner": 2,
    "max_scale_up_step": 1,
    "scale_from_events": true,
    "event_weight": 1
  },
  "demands": [
    {
      "pool_key": "ci-smoke",
      "worker": "planner",
      "required_capabilities": ["code"],
      "required_labels": {
        "pool": "ci-smoke"
      },
      "queued_tasks": 6,
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

log "registering full and stale runners via coat runner HTTP client"
"$coat_bin" runner register --registry-url "$registry_url" --file "$tmpdir/local-full.json" >"$tmpdir/register-local-full.json"
"$coat_bin" runner register --registry-url "$registry_url" --file "$tmpdir/stale-remote.json" >"$tmpdir/register-stale-remote.json"

log "heartbeating full and stale scenarios"
curl -fsS -X POST "$registry_url/runners/heartbeat" \
  -H 'content-type: application/json' \
  --data @"$tmpdir/local-full-heartbeat.json" >"$tmpdir/heartbeat-local-full.json"
curl -fsS -X POST "$registry_url/runners/heartbeat" \
  -H 'content-type: application/json' \
  --data @"$tmpdir/stale-remote-heartbeat.json" >"$tmpdir/heartbeat-stale-remote.json"

sleep 2

log "registering active remote runner after stale TTL expires"
"$coat_bin" runner register --registry-url "$registry_url" --file "$tmpdir/active-remote.json" >"$tmpdir/register-active-remote.json"

log "checking status, dispatch, and capacity plan"
"$coat_bin" runner status --registry-url "$registry_url" >"$tmpdir/status.json"
"$coat_bin" runner dispatch --registry-url "$registry_url" --file "$tmpdir/dispatch.json" >"$tmpdir/dispatch-response.json"
"$coat_bin" runner capacity-plan --registry-url "$registry_url" --file "$tmpdir/capacity-plan.json" --ignore-config-policy >"$tmpdir/capacity-response.json"

python3 - "$tmpdir" "$journal_path" <<'PY'
import json
import pathlib
import sys

tmpdir = pathlib.Path(sys.argv[1])
journal_path = pathlib.Path(sys.argv[2])

def load(name):
    with (tmpdir / name).open() as handle:
        return json.load(handle)

def require(condition, message):
    if not condition:
        raise SystemExit(message)

for name, runner_id in [
    ("register-local-full.json", "local-full-smoke"),
    ("register-stale-remote.json", "stale-remote-smoke"),
    ("register-active-remote.json", "active-remote-smoke"),
]:
    payload = load(name)
    require(payload["runner_id"] == runner_id, f"{name} registered {payload.get('runner_id')}")

for name in ["heartbeat-local-full.json", "heartbeat-stale-remote.json"]:
    payload = load(name)
    require(payload["known"] is True, f"{name} did not heartbeat a known runner")

statuses = {
    item["registration"]["runner_id"]: item
    for item in load("status.json")
}
require(set(statuses) == {"local-full-smoke", "stale-remote-smoke", "active-remote-smoke"}, statuses)
require(statuses["local-full-smoke"]["full"] is True, statuses["local-full-smoke"])
require(statuses["local-full-smoke"]["dispatchable"] is False, statuses["local-full-smoke"])
require(statuses["stale-remote-smoke"]["stale"] is True, statuses["stale-remote-smoke"])
require(statuses["stale-remote-smoke"]["dispatchable"] is False, statuses["stale-remote-smoke"])
require(statuses["active-remote-smoke"]["dispatchable"] is True, statuses["active-remote-smoke"])

dispatch = load("dispatch-response.json")
require(dispatch["status"] == "matched", dispatch)
require(dispatch["runner_id"] == "active-remote-smoke", dispatch)
require(dispatch["runner_endpoint"] == "http://active-remote-smoke:9091", dispatch)
require(len(dispatch["candidates"]) == 1, dispatch["candidates"])
require(dispatch["candidates"][0]["runner_id"] == "active-remote-smoke", dispatch["candidates"])

capacity = load("capacity-response.json")
require(capacity["status"] == "provision_recommended", capacity)
require(capacity["mode"] == "provision_ephemeral", capacity)
require(len(capacity["pool_decisions"]) == 1, capacity)
pool = capacity["pool_decisions"][0]
require(pool["pool_key"] == "ci-smoke", pool)
require(pool["current_runners"] == 1, pool)
require(pool["provision_runners"] == 1, pool)

lines = [line for line in journal_path.read_text().splitlines() if line.strip()]
require(len(lines) >= 5, f"expected journal entries, got {len(lines)}")

print("runner registry smoke assertions passed")
PY

log "passed"
