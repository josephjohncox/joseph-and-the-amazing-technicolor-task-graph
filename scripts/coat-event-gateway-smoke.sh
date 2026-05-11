#!/bin/sh
set -eu

# Bounded local smoke for event-gateway ingress and goal-store projection.
#
# The script runs coat-goal-store and coat-event-gateway on ephemeral localhost
# ports with JSONL journals. It registers a risky generic CI source with an
# activation approval, verifies the approval projects into goal-store, emits and
# dedupes a generic event through the gateway, and inspects the projected/local
# event state. It does not require Docker or live provider credentials.

fail() {
  printf 'event-gateway smoke failed: %s\n' "$*" >&2
  exit 1
}

skip() {
  printf '[event-gateway-smoke] SKIP: %s\n' "$*"
  exit 0
}

log() {
  printf '[event-gateway-smoke] %s\n' "$*"
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

if [ "${COAT_EVENT_GATEWAY_SMOKE_SKIP_BUILD:-0}" != "1" ]; then
  need_command cargo
  log "building coat-event-gateway and coat-goal-store"
  if [ "$profile" = "release" ]; then
    cargo build -p coat-event-gateway -p coat-goal-store --release
  else
    cargo build -p coat-event-gateway -p coat-goal-store
  fi
fi

event_gateway_bin=${COAT_EVENT_GATEWAY_BIN:-$bin_dir/coat-event-gateway}
goal_store_bin=${COAT_GOAL_STORE_BIN:-$bin_dir/coat-goal-store}

[ -x "$event_gateway_bin" ] || fail "missing coat-event-gateway binary at $event_gateway_bin"
[ -x "$goal_store_bin" ] || fail "missing coat-goal-store binary at $goal_store_bin"

tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/coat-event-gateway-smoke.XXXXXX")
goal_store_pid=
event_gateway_pid=
goal_store_log="$tmpdir/goal-store.log"
event_gateway_log="$tmpdir/event-gateway.log"

cleanup() {
  status=$?
  if [ -n "${event_gateway_pid:-}" ] && kill -0 "$event_gateway_pid" >/dev/null 2>&1; then
    kill "$event_gateway_pid" >/dev/null 2>&1 || true
    wait "$event_gateway_pid" >/dev/null 2>&1 || true
  fi
  if [ -n "${goal_store_pid:-}" ] && kill -0 "$goal_store_pid" >/dev/null 2>&1; then
    kill "$goal_store_pid" >/dev/null 2>&1 || true
    wait "$goal_store_pid" >/dev/null 2>&1 || true
  fi
  if [ "$status" -ne 0 ]; then
    if [ -f "$goal_store_log" ]; then
      printf '\ngoal-store log:\n' >&2
      sed -n '1,120p' "$goal_store_log" >&2 || true
    fi
    if [ -f "$event_gateway_log" ]; then
      printf '\nevent-gateway log:\n' >&2
      sed -n '1,120p' "$event_gateway_log" >&2 || true
    fi
  fi
  rm -rf "$tmpdir"
}
trap cleanup EXIT INT TERM

ports_file="$tmpdir/ports.txt"
ports_error="$tmpdir/ports.err"
if python3 >"$ports_file" 2>"$ports_error" <<'PY'
import errno
import socket
import sys

sockets = []
try:
    for _ in range(2):
        sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        sock.bind(("127.0.0.1", 0))
        sockets.append(sock)
    for sock in sockets:
        print(sock.getsockname()[1])
except OSError as exc:
    print(f"unable to allocate localhost ports: {exc}", file=sys.stderr)
    if exc.errno in (errno.EACCES, errno.EPERM):
        sys.exit(42)
    sys.exit(1)
finally:
    for sock in sockets:
        sock.close()
PY
then
  :
else
  status=$?
  if [ "$status" -eq 42 ]; then
    skip "$(sed -n '1p' "$ports_error")"
  fi
  fail "$(sed -n '1p' "$ports_error")"
fi

goal_store_port=$(sed -n '1p' "$ports_file")
event_gateway_port=$(sed -n '2p' "$ports_file")
[ -n "$goal_store_port" ] || fail "did not allocate goal-store port"
[ -n "$event_gateway_port" ] || fail "did not allocate event-gateway port"

goal_store_url="http://127.0.0.1:$goal_store_port"
event_gateway_url="http://127.0.0.1:$event_gateway_port"
goal_store_journal="$tmpdir/goal-store.jsonl"
event_gateway_journal="$tmpdir/event-gateway.jsonl"

bind_permission_error() {
  [ -f "$1" ] && grep -Eiq 'permission denied|operation not permitted|EACCES|EPERM' "$1"
}

wait_for_health() {
  name=$1
  url=$2
  pid=$3
  log_file=$4
  attempt=1
  while [ "$attempt" -le 50 ]; do
    if curl -fsS "$url/healthz" >/dev/null 2>&1; then
      return 0
    fi
    if ! kill -0 "$pid" >/dev/null 2>&1; then
      if bind_permission_error "$log_file"; then
        skip "$name could not bind its localhost port; local port bind is unavailable"
      fi
      fail "$name process exited before health check passed"
    fi
    sleep 0.1
    attempt=$((attempt + 1))
  done
  if bind_permission_error "$log_file"; then
    skip "$name could not bind its localhost port; local port bind is unavailable"
  fi
  fail "$name did not become healthy at $url"
}

log "starting goal-store on $goal_store_url"
BIND_ADDR="127.0.0.1:$goal_store_port" \
  COAT_GOAL_STORE_JOURNAL_PATH="$goal_store_journal" \
  "$goal_store_bin" >"$goal_store_log" 2>&1 &
goal_store_pid=$!
wait_for_health "goal-store" "$goal_store_url" "$goal_store_pid" "$goal_store_log"

log "starting event-gateway on $event_gateway_url with goal-store projection"
BIND_ADDR="127.0.0.1:$event_gateway_port" \
  COAT_EVENT_GATEWAY_JOURNAL_PATH="$event_gateway_journal" \
  COAT_GOAL_STORE_URL="$goal_store_url" \
  COAT_REQUIRE_EVENT_SOURCE_APPROVAL=true \
  "$event_gateway_bin" >"$event_gateway_log" 2>&1 &
event_gateway_pid=$!
wait_for_health "event-gateway" "$event_gateway_url" "$event_gateway_pid" "$event_gateway_log"

cat >"$tmpdir/event-source.json" <<'JSON'
{
  "id": "ci-events",
  "kind": "ci",
  "enabled": true,
  "description": "Generic CI event source for workflow status changes from any CI provider or event bus.",
  "namespace": "coat-smoke",
  "webhook": null,
  "generic": {
    "auth": {
      "kind": "none",
      "secret_ref": null,
      "header_name": null
    },
    "accepts_cloudevents": true,
    "max_payload_bytes": 1048576,
    "allowed_event_types": [
      "ci.workflow.failed",
      "ci.workflow.completed"
    ],
    "id_json_pointer": "/id",
    "type_json_pointer": "/type",
    "subject_json_pointer": "/subject",
    "dedupe_json_pointer": "/delivery_id",
    "dedupe_header": "ce-id",
    "payload_schema": null,
    "mcp_context": null
  },
  "sqs": null,
  "schedule": null,
  "calendar": null,
  "route": {
    "mode": "create_goal",
    "goal_template": {
      "title_template": "Investigate {{event_type}} from {{source_id}}",
      "objective_template": "Investigate event {{event_id}} from {{source_id}}: {{subject}}",
      "repo": null
    },
    "target_goal_id": null,
    "steering_directive": null,
    "require_approval": false,
    "dedupe_window_seconds": 3600
  }
}
JSON

cat >"$tmpdir/generic-event.json" <<'JSON'
{
  "id": "ci-run-12345",
  "type": "ci.workflow.failed",
  "subject": "example/repo pull request 42 failed required checks",
  "delivery_id": "ci:example-repo:run-12345",
  "time": "2026-05-06T22:15:00Z",
  "repository": "example/repo",
  "pull_request": 42,
  "workflow": "cargo-test",
  "run_url": "https://ci.example.com/example/repo/runs/12345",
  "summary": "cargo test --workspace failed in coat-event-gateway tests"
}
JSON

log "registering approved risky generic source"
curl -fsS -X POST "$event_gateway_url/event-sources" \
  -H 'content-type: application/json' \
  -H 'x-coat-approval-id: smoke-approval-120' \
  -H 'x-coat-operator: local-smoke' \
  --data @"$tmpdir/event-source.json" >"$tmpdir/register-source.json"

log "emitting generic event and duplicate delivery"
curl -fsS -X POST "$event_gateway_url/events/generic/ci-events" \
  -H 'content-type: application/json' \
  --data @"$tmpdir/generic-event.json" >"$tmpdir/emit-response.json"
curl -fsS -X POST "$event_gateway_url/events/generic/ci-events" \
  -H 'content-type: application/json' \
  --data @"$tmpdir/generic-event.json" >"$tmpdir/emit-duplicate-response.json"

log "inspecting gateway and goal-store state"
curl -fsS "$event_gateway_url/healthz" >"$tmpdir/event-gateway-health.json"
curl -fsS "$goal_store_url/healthz" >"$tmpdir/goal-store-health.json"
curl -fsS "$event_gateway_url/events?source_id=ci-events" >"$tmpdir/events.json"
curl -fsS "$event_gateway_url/triggers" >"$tmpdir/triggers.json"
curl -fsS "$goal_store_url/goal-store/event-source-approvals?source_id=ci-events" >"$tmpdir/event-source-approvals.json"
goal_id=$(python3 - "$tmpdir/emit-response.json" <<'PY'
import json
import sys

with open(sys.argv[1]) as handle:
    response = json.load(handle)
goal_id = response.get("goal_id")
if not goal_id:
    raise SystemExit("emit response did not include projected goal_id")
print(goal_id)
PY
)
curl -fsS "$goal_store_url/goal-store/goals/$goal_id/events" >"$tmpdir/goal-events.json"

python3 - "$tmpdir" "$event_gateway_journal" "$goal_store_journal" <<'PY'
import json
import pathlib
import sys

tmpdir = pathlib.Path(sys.argv[1])
event_gateway_journal = pathlib.Path(sys.argv[2])
goal_store_journal = pathlib.Path(sys.argv[3])

def load(name):
    with (tmpdir / name).open() as handle:
        return json.load(handle)

def require(condition, message):
    if not condition:
        raise SystemExit(message)

gateway_health = load("event-gateway-health.json")
require(gateway_health["status"] == "ok", gateway_health)
require(gateway_health["backend"] == "jsonl", gateway_health)
require(gateway_health["goal_store_projection_enabled"] is True, gateway_health)

goal_store_health = load("goal-store-health.json")
require(goal_store_health["status"] == "ok", goal_store_health)
require(goal_store_health["backend"] == "jsonl", goal_store_health)

registered = load("register-source.json")
require(registered["id"] == "ci-events", registered)
require(registered["enabled"] is True, registered)
require(registered["route"]["mode"] == "create_goal", registered)
require(registered["route"]["require_approval"] is False, registered)
require(registered["route"]["goal_template"]["title_template"].startswith("Investigate"), registered)

approval_records = load("event-source-approvals.json")["records"]
require(len(approval_records) == 1, approval_records)
approval = approval_records[0]
require(approval["source_id"] == "ci-events", approval)
require(approval["approval_ref"] == "smoke-approval-120", approval)
require(approval["operator"] == "local-smoke", approval)
require(approval["risky"] is True, approval)
require(approval["status"] == "provided", approval)
require(approval["payload_json"]["route_mode"] == "create_goal", approval)
require(approval["payload_json"]["route_requires_approval"] is False, approval)

emit = load("emit-response.json")
require(emit["accepted"] is True, emit)
require(emit["status"] == "recorded", emit)
require(emit["event_id"] == "ci-run-12345", emit)
require(emit["deduped"] is False, emit)
goal_id = emit["goal_id"]
require(goal_id, emit)
require(
    emit["diagnostics"] == ["COAT_RESTATE_INGRESS is not configured; goal recorded only"],
    emit,
)

duplicate = load("emit-duplicate-response.json")
require(duplicate["accepted"] is True, duplicate)
require(duplicate["status"] == "recorded", duplicate)
require(duplicate["event_id"] == "ci-run-12345", duplicate)
require(duplicate["deduped"] is True, duplicate)
require(duplicate["goal_id"] == goal_id, duplicate)

events = load("events.json")
require(len(events) == 1, events)
event = events[0]
require(event["id"] == "ci-run-12345", event)
require(event["source_id"] == "ci-events", event)
require(event["source_kind"] == "ci", event)
require(event["event_type"] == "ci.workflow.failed", event)
require(event["subject"] == "example/repo pull request 42 failed required checks", event)
require(event["dedupe_key"] == "ci:example-repo:run-12345", event)
require(event["occurred_at"] == "2026-05-06T22:15:00Z", event)
require(event["payload"]["workflow"] == "cargo-test", event)

triggers = load("triggers.json")
require([trigger["status"] for trigger in triggers] == ["recorded"], triggers)
require([trigger["event_id"] for trigger in triggers] == ["ci-run-12345"], triggers)
require(triggers[0]["goal_id"] == goal_id, triggers)

goal_events = load("goal-events.json")
require(goal_events["goal_id"] == goal_id, goal_events)
require(len(goal_events["events"]) == 1, goal_events)
projection = goal_events["events"][0]
require(projection["goal_id"] == goal_id, projection)
require(projection["task_id"] is None, projection)
require(projection["kind"] == "state_projected", projection)
require(projection["actor"] == "coat-event-gateway", projection)
require(projection["message"] == "event_gateway_trigger_recorded:ci-run-12345", projection)
require(projection["idempotency_key"].startswith("event-gateway:trigger:"), projection)
require(projection["payload_json"]["projection_source"] == "event_gateway", projection)
require(projection["payload_json"]["trigger"]["event_id"] == "ci-run-12345", projection)
require(projection["payload_json"]["trigger"]["status"] == "recorded", projection)
require(projection["payload_json"]["trigger"]["goal_id"] == goal_id, projection)

gateway_entries = [
    json.loads(line)
    for line in event_gateway_journal.read_text().splitlines()
    if line.strip()
]
require([entry["type"] for entry in gateway_entries] == ["source", "event", "trigger"], gateway_entries)
require(gateway_entries[1]["id"] == "ci-run-12345", gateway_entries[1])
require(gateway_entries[2]["status"] == "recorded", gateway_entries[2])
require(gateway_entries[2]["goal_id"] == goal_id, gateway_entries[2])

store_entries = [
    json.loads(line)
    for line in goal_store_journal.read_text().splitlines()
    if line.strip()
]
require(len(store_entries) == 2, store_entries)
require(store_entries[0]["type"] == "event_source_approval", store_entries)
require(store_entries[0]["source_id"] == "ci-events", store_entries[0])
require(store_entries[1]["type"] == "event", store_entries)
require(store_entries[1]["event"]["goal_id"] == goal_id, store_entries[1])
require(store_entries[1]["event"]["payload_json"]["trigger"]["status"] == "recorded", store_entries[1])

print("event gateway smoke assertions passed")
PY

log "passed"
