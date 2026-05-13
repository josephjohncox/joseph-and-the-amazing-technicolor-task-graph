#!/bin/sh
set -eu

# Compose-backed EventOps proof.
#
# This uses the same deterministic local stack operators use, not isolated
# localhost service processes. It registers an event source through
# coat-event-gateway, emits a generic CI event, lets the gateway route a goal
# through Restate, then verifies the trigger projection is visible in
# coat-goal-store. The script always attempts to tear the stack down.

fail() {
  printf 'event-gateway compose smoke failed: %s\n' "$*" >&2
  exit 1
}

log() {
  printf '[event-gateway-compose-smoke] %s\n' "$*"
}

need_command() {
  command -v "$1" >/dev/null 2>&1 || fail "missing required command: $1"
}

script_dir=$(CDPATH= cd "$(dirname "$0")" && pwd)
repo_root=$(CDPATH= cd "$script_dir/.." && pwd)
cd "$repo_root"

PATH="/opt/homebrew/bin:/usr/local/bin:$PATH"
export PATH

need_command curl
need_command python3

coat=${COAT:-target/debug/coat}
if [ ! -x "$coat" ]; then
  coat=coat
fi

out_root=${COAT_EVENT_GATEWAY_COMPOSE_SMOKE_OUT:-target/coat-event-gateway-compose-smoke}
case "$out_root" in
  ""|"/"|".") fail "refusing unsafe evidence directory: $out_root" ;;
esac
run_dir="$out_root/latest"
rm -rf "$run_dir"
mkdir -p "$run_dir"

event_gateway_url=${COAT_EVENT_GATEWAY_COMPOSE_URL:-http://127.0.0.1:9089}
goal_store_url=${COAT_GOAL_STORE_COMPOSE_URL:-http://127.0.0.1:9088}
stack_env=target/coat-scenarios/latest/stack/stub-local-providers.env
started_stack=0

cleanup() {
  status=$?
  if [ "$started_stack" = "1" ]; then
    "$coat" deploy local down --env-file "$stack_env" >/dev/null 2>&1 || true
  fi
  exit "$status"
}
trap cleanup EXIT INT TERM

wait_for_http() {
  name=$1
  url=$2
  attempt=1
  while [ "$attempt" -le 60 ]; do
    if curl -fsS "$url" >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
    attempt=$((attempt + 1))
  done
  fail "$name did not become healthy at $url"
}

log "starting deterministic Compose stack"
COAT_REQUIRE_EVENT_SOURCE_APPROVAL=true \
COAT="$coat" \
COAT_SCENARIO_E2E_OUT=target/coat-scenarios \
COAT_SCENARIO_E2E_STACK=always \
COAT_SCENARIO_E2E_STACK_ONLY=1 \
COAT_SCENARIO_E2E_KEEP_STACK=1 \
sh scripts/coat-scenario-e2e.sh
started_stack=1

wait_for_http "event-gateway" "$event_gateway_url/healthz"
wait_for_http "goal-store" "$goal_store_url/healthz"

suffix=$$
source_id="compose-ci-events-$suffix"
event_id="compose-ci-run-$suffix"
dedupe_key="ci:compose-smoke:$suffix"
approval_id="compose-event-source-approval-$suffix"

cat >"$run_dir/event-source.json" <<JSON
{
  "id": "$source_id",
  "kind": "ci",
  "enabled": true,
  "description": "Compose-backed CI event source for EventOps topology proof.",
  "namespace": "coat-compose-smoke",
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

cat >"$run_dir/generic-event.json" <<JSON
{
  "id": "$event_id",
  "type": "ci.workflow.failed",
  "subject": "compose smoke pull request failed required checks",
  "delivery_id": "$dedupe_key",
  "time": "2026-05-12T20:15:00Z",
  "repository": "josephjohncox/joseph-and-the-amazing-technicolor-task-graph",
  "pull_request": 123,
  "workflow": "ci",
  "run_url": "https://github.com/josephjohncox/joseph-and-the-amazing-technicolor-task-graph/actions/runs/compose-smoke",
  "summary": "Compose topology proof generated this synthetic CI failure event."
}
JSON

log "registering approved create-goal event source $source_id"
curl -fsS -X POST "$event_gateway_url/event-sources" \
  -H 'content-type: application/json' \
  -H "x-coat-approval-id: $approval_id" \
  -H 'x-coat-operator: compose-smoke' \
  --data @"$run_dir/event-source.json" >"$run_dir/register-source.json"

log "emitting event $event_id and duplicate delivery"
curl -fsS -X POST "$event_gateway_url/events/generic/$source_id" \
  -H 'content-type: application/json' \
  --data @"$run_dir/generic-event.json" >"$run_dir/emit-response.json"
curl -fsS -X POST "$event_gateway_url/events/generic/$source_id" \
  -H 'content-type: application/json' \
  --data @"$run_dir/generic-event.json" >"$run_dir/emit-duplicate-response.json"

goal_id=$(python3 - "$run_dir/emit-response.json" <<'PY'
import json
import sys

with open(sys.argv[1]) as handle:
    response = json.load(handle)
goal_id = response.get("goal_id")
if not goal_id:
    raise SystemExit("emit response did not include goal_id")
print(goal_id)
PY
)

log "waiting for projected trigger event for goal $goal_id"
attempt=1
while [ "$attempt" -le 60 ]; do
  if curl -fsS "$goal_store_url/goal-store/goals/$goal_id/events" >"$run_dir/goal-events.json" 2>"$run_dir/goal-events.err"; then
    if python3 - "$run_dir/goal-events.json" <<'PY'
import json
import sys

with open(sys.argv[1]) as handle:
    payload = json.load(handle)
events = payload.get("events") or []
raise SystemExit(0 if events else 1)
PY
    then
      break
    fi
  fi
  sleep 1
  attempt=$((attempt + 1))
done

[ -f "$run_dir/goal-events.json" ] || fail "goal-store did not return events for $goal_id"

curl -fsS "$event_gateway_url/healthz" >"$run_dir/event-gateway-health.json"
curl -fsS "$goal_store_url/healthz" >"$run_dir/goal-store-health.json"
curl -fsS "$event_gateway_url/events?source_id=$source_id" >"$run_dir/events.json"
curl -fsS "$event_gateway_url/triggers" >"$run_dir/triggers.json"
curl -fsS "$goal_store_url/goal-store/event-source-approvals?source_id=$source_id" >"$run_dir/event-source-approvals.json"
curl -fsS "$goal_store_url/goal-store/goals?limit=100" >"$run_dir/goals.json"

python3 - "$run_dir" "$source_id" "$event_id" "$dedupe_key" "$approval_id" "$goal_id" <<'PY'
import json
import pathlib
import sys

run_dir = pathlib.Path(sys.argv[1])
source_id = sys.argv[2]
event_id = sys.argv[3]
dedupe_key = sys.argv[4]
approval_id = sys.argv[5]
goal_id = sys.argv[6]

def load(name):
    with (run_dir / name).open() as handle:
        return json.load(handle)

def require(condition, message):
    if not condition:
        raise SystemExit(message)

gateway_health = load("event-gateway-health.json")
require(gateway_health["status"] == "ok", gateway_health)
require(gateway_health["goal_store_projection_enabled"] is True, gateway_health)

goal_store_health = load("goal-store-health.json")
require(goal_store_health["status"] == "ok", goal_store_health)

registered = load("register-source.json")
require(registered["id"] == source_id, registered)
require(registered["enabled"] is True, registered)
require(registered["route"]["mode"] == "create_goal", registered)

approvals = load("event-source-approvals.json")["records"]
require(len(approvals) == 1, approvals)
approval = approvals[0]
require(approval["source_id"] == source_id, approval)
require(approval["approval_ref"] == approval_id, approval)
require(approval["operator"] == "compose-smoke", approval)
require(approval["risky"] is True, approval)
require(approval["status"] == "provided", approval)

emit = load("emit-response.json")
require(emit["accepted"] is True, emit)
require(emit["status"] == "submitted", emit)
require(emit["event_id"] == event_id, emit)
require(emit["deduped"] is False, emit)
require(emit["goal_id"] == goal_id, emit)
require(not any("not configured" in diagnostic for diagnostic in emit.get("diagnostics", [])), emit)

duplicate = load("emit-duplicate-response.json")
require(duplicate["accepted"] is True, duplicate)
require(duplicate["deduped"] is True, duplicate)
require(duplicate["goal_id"] == goal_id, duplicate)

events = load("events.json")
require(len(events) == 1, events)
event = events[0]
require(event["id"] == event_id, event)
require(event["source_id"] == source_id, event)
require(event["event_type"] == "ci.workflow.failed", event)
require(event["dedupe_key"] == dedupe_key, event)
require(event["payload"]["workflow"] == "ci", event)

triggers = [trigger for trigger in load("triggers.json") if trigger.get("event_id") == event_id]
require(len(triggers) == 1, triggers)
trigger = triggers[0]
require(trigger["status"] == "submitted", trigger)
require(trigger["goal_id"] == goal_id, trigger)

goal_events = load("goal-events.json")
require(goal_events["goal_id"] == goal_id, goal_events)
projected = [
    event
    for event in goal_events["events"]
    if event.get("actor") == "coat-event-gateway"
    and event.get("payload_json", {}).get("trigger", {}).get("event_id") == event_id
]
require(len(projected) == 1, goal_events)
projection = projected[0]
require(projection["kind"] == "submitted", projection)
require(projection["message"] == f"event_gateway_trigger_submitted:{event_id}", projection)
require(projection["payload_json"]["projection_source"] == "event_gateway", projection)
require(projection["payload_json"]["trigger"]["status"] == "submitted", projection)
require(projection["payload_json"]["trigger"]["goal_id"] == goal_id, projection)

goals_text = json.dumps(load("goals.json"))
require(goal_id in goals_text, load("goals.json"))

print("event gateway Compose topology smoke assertions passed")
PY

log "evidence written to $run_dir"
