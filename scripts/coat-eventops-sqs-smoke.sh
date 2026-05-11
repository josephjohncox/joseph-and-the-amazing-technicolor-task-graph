#!/bin/sh
set -eu

# Optional LocalStack-backed proof for the first EventOps SQS slice.
#
# The fast event-gateway smoke stays no-Docker. This script starts LocalStack
# when Docker is available, creates inbound and outbound queues, proves
# coat-event-gateway can poll/delete an inbound SQS event, and proves
# coat-notifier can deliver a notification envelope to outbound SQS.

fail() {
  printf 'eventops SQS smoke failed: %s\n' "$*" >&2
  exit 1
}

skip() {
  printf '[eventops-sqs-smoke] SKIP: %s\n' "$*"
  exit 0
}

log() {
  printf '[eventops-sqs-smoke] %s\n' "$*"
}

need_command() {
  command -v "$1" >/dev/null 2>&1 || fail "missing required command: $1"
}

require_localstack=${COAT_EVENTOPS_SQS_SMOKE_REQUIRE_LOCALSTACK:-0}

skip_or_fail() {
  if [ "$require_localstack" = "1" ]; then
    fail "$*"
  fi
  skip "$*"
}

need_command curl
need_command python3

if ! command -v docker >/dev/null 2>&1; then
  skip_or_fail "Docker is not installed; LocalStack SQS smoke is optional"
fi

if ! docker info >/dev/null 2>&1; then
  skip_or_fail "Docker daemon is not available; LocalStack SQS smoke is optional"
fi

profile=${COAT_BUILD_PROFILE:-debug}
case "$profile" in
  debug)
    bin_dir=target/debug
    build_args=
    ;;
  release)
    bin_dir=target/release
    build_args=--release
    ;;
  *)
    fail "COAT_BUILD_PROFILE must be debug or release, got $profile"
    ;;
esac

if [ "${COAT_EVENTOPS_SQS_SMOKE_SKIP_BUILD:-0}" != "1" ]; then
  need_command cargo
  log "building coat-event-gateway, coat-goal-store, and coat-notifier"
  cargo build -p coat-event-gateway -p coat-goal-store -p coat-notifier $build_args
fi

event_gateway_bin=${COAT_EVENT_GATEWAY_BIN:-$bin_dir/coat-event-gateway}
goal_store_bin=${COAT_GOAL_STORE_BIN:-$bin_dir/coat-goal-store}
notifier_bin=${COAT_NOTIFIER_BIN:-$bin_dir/coat-notifier}

[ -x "$event_gateway_bin" ] || fail "missing coat-event-gateway binary at $event_gateway_bin"
[ -x "$goal_store_bin" ] || fail "missing coat-goal-store binary at $goal_store_bin"
[ -x "$notifier_bin" ] || fail "missing coat-notifier binary at $notifier_bin"

tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/coat-eventops-sqs-smoke.XXXXXX")
container_name="coat-eventops-sqs-smoke-$$"
goal_store_pid=
event_gateway_pid=
notifier_pid=
localstack_started=0

goal_store_log="$tmpdir/goal-store.log"
event_gateway_log="$tmpdir/event-gateway.log"
notifier_log="$tmpdir/notifier.log"
localstack_log="$tmpdir/localstack.log"

cleanup() {
  status=$?
  if [ -n "${notifier_pid:-}" ] && kill -0 "$notifier_pid" >/dev/null 2>&1; then
    kill "$notifier_pid" >/dev/null 2>&1 || true
    wait "$notifier_pid" >/dev/null 2>&1 || true
  fi
  if [ -n "${event_gateway_pid:-}" ] && kill -0 "$event_gateway_pid" >/dev/null 2>&1; then
    kill "$event_gateway_pid" >/dev/null 2>&1 || true
    wait "$event_gateway_pid" >/dev/null 2>&1 || true
  fi
  if [ -n "${goal_store_pid:-}" ] && kill -0 "$goal_store_pid" >/dev/null 2>&1; then
    kill "$goal_store_pid" >/dev/null 2>&1 || true
    wait "$goal_store_pid" >/dev/null 2>&1 || true
  fi
  if [ "$localstack_started" = "1" ]; then
    docker rm -f "$container_name" >/dev/null 2>&1 || true
  fi
  if [ "$status" -ne 0 ]; then
    for log_file in "$goal_store_log" "$event_gateway_log" "$notifier_log" "$localstack_log"; do
      if [ -f "$log_file" ]; then
        printf '\n%s:\n' "$log_file" >&2
        sed -n '1,140p' "$log_file" >&2 || true
      fi
    done
    printf '\neventops SQS smoke artifacts: %s\n' "$tmpdir" >&2
  else
    rm -rf "$tmpdir"
  fi
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
    for _ in range(4):
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
    skip_or_fail "$(sed -n '1p' "$ports_error")"
  fi
  fail "$(sed -n '1p' "$ports_error")"
fi

localstack_port=$(sed -n '1p' "$ports_file")
goal_store_port=$(sed -n '2p' "$ports_file")
event_gateway_port=$(sed -n '3p' "$ports_file")
notifier_port=$(sed -n '4p' "$ports_file")

localstack_url="http://127.0.0.1:$localstack_port"
goal_store_url="http://127.0.0.1:$goal_store_port"
event_gateway_url="http://127.0.0.1:$event_gateway_port"
notifier_url="http://127.0.0.1:$notifier_port"
inbound_queue_url="$localstack_url/000000000000/coat-inbound-events"
outbound_queue_url="$localstack_url/000000000000/coat-notifications"
inbound_queue_internal_url="http://localhost:4566/000000000000/coat-inbound-events"
outbound_queue_internal_url="http://localhost:4566/000000000000/coat-notifications"
region=${COAT_SQS_REGION:-us-east-1}
localstack_image=${COAT_LOCALSTACK_IMAGE:-localstack/localstack:3.8.1}

log "starting LocalStack SQS on $localstack_url"
if ! docker run -d \
  --name "$container_name" \
  -e SERVICES=sqs \
  -e AWS_DEFAULT_REGION="$region" \
  -p "127.0.0.1:$localstack_port:4566" \
  "$localstack_image" >"$tmpdir/localstack.cid" 2>"$localstack_log"; then
  skip_or_fail "LocalStack container could not start; image may be unavailable"
fi
localstack_started=1

localstack_sqs() {
  docker exec \
    -e AWS_ACCESS_KEY_ID=test \
    -e AWS_SECRET_ACCESS_KEY=test \
    -e AWS_DEFAULT_REGION="$region" \
    "$container_name" awslocal sqs "$@"
}

attempt=1
while [ "$attempt" -le 80 ]; do
  if localstack_sqs list-queues >/dev/null 2>&1; then
    break
  fi
  if ! docker ps --format '{{.Names}}' | grep -Fx "$container_name" >/dev/null 2>&1; then
    docker logs "$container_name" >"$localstack_log" 2>&1 || true
    skip_or_fail "LocalStack container exited before SQS became ready"
  fi
  sleep 0.25
  attempt=$((attempt + 1))
done
if [ "$attempt" -gt 80 ]; then
  docker logs "$container_name" >"$localstack_log" 2>&1 || true
  skip_or_fail "LocalStack SQS did not become ready"
fi

localstack_sqs create-queue --queue-name coat-inbound-events >/dev/null
localstack_sqs create-queue --queue-name coat-notifications >/dev/null

wait_for_health() {
  name=$1
  url=$2
  pid=$3
  log_file=$4
  attempt=1
  while [ "$attempt" -le 60 ]; do
    if curl -fsS "$url/healthz" >/dev/null 2>&1; then
      return 0
    fi
    if ! kill -0 "$pid" >/dev/null 2>&1; then
      if [ -f "$log_file" ] && grep -Eiq 'permission denied|operation not permitted|EACCES|EPERM' "$log_file"; then
        skip_or_fail "$name could not bind its localhost port; local port bind is unavailable"
      fi
      fail "$name process exited before health check passed"
    fi
    sleep 0.1
    attempt=$((attempt + 1))
  done
  fail "$name did not become healthy at $url"
}

goal_store_journal="$tmpdir/goal-store.jsonl"
event_gateway_journal="$tmpdir/event-gateway.jsonl"
notifier_journal="$tmpdir/notifier.jsonl"

log "starting goal-store, event-gateway, and notifier"
BIND_ADDR="127.0.0.1:$goal_store_port" \
  COAT_GOAL_STORE_JOURNAL_PATH="$goal_store_journal" \
  "$goal_store_bin" >"$goal_store_log" 2>&1 &
goal_store_pid=$!
wait_for_health "goal-store" "$goal_store_url" "$goal_store_pid" "$goal_store_log"

AWS_ACCESS_KEY_ID=test \
AWS_SECRET_ACCESS_KEY=test \
AWS_DEFAULT_REGION="$region" \
COAT_SQS_REGION="$region" \
COAT_SQS_ENDPOINT_URL="$localstack_url" \
BIND_ADDR="127.0.0.1:$event_gateway_port" \
  COAT_EVENT_GATEWAY_JOURNAL_PATH="$event_gateway_journal" \
  COAT_GOAL_STORE_URL="$goal_store_url" \
  "$event_gateway_bin" >"$event_gateway_log" 2>&1 &
event_gateway_pid=$!
wait_for_health "event-gateway" "$event_gateway_url" "$event_gateway_pid" "$event_gateway_log"

AWS_ACCESS_KEY_ID=test \
AWS_SECRET_ACCESS_KEY=test \
AWS_DEFAULT_REGION="$region" \
COAT_SQS_REGION="$region" \
COAT_SQS_ENDPOINT_URL="$localstack_url" \
BIND_ADDR="127.0.0.1:$notifier_port" \
  COAT_NOTIFIER_JOURNAL_PATH="$notifier_journal" \
  "$notifier_bin" >"$notifier_log" 2>&1 &
notifier_pid=$!
wait_for_health "notifier" "$notifier_url" "$notifier_pid" "$notifier_log"

python3 - "$tmpdir" "$inbound_queue_url" "$outbound_queue_url" "$localstack_url" "$region" <<'PY'
import json
import pathlib
import sys

tmpdir = pathlib.Path(sys.argv[1])
inbound_queue_url = sys.argv[2]
outbound_queue_url = sys.argv[3]
endpoint = sys.argv[4]
region = sys.argv[5]

source = json.loads(pathlib.Path("examples/event-source-sqs-notifications.json").read_text())
source["enabled"] = True
source["sqs"]["queue_url"] = inbound_queue_url
source["sqs"]["region"] = region
source["sqs"]["endpoint"] = endpoint
source["sqs"]["wait_time_seconds"] = 0
source["sqs"]["max_messages"] = 10
source["sqs"]["delete_on_success"] = True
(tmpdir / "event-source-sqs.json").write_text(json.dumps(source, indent=2) + "\n")

notification = json.loads(pathlib.Path("examples/notification-sqs.json").read_text())
notification["policy"]["targets"][0]["address"] = outbound_queue_url
(tmpdir / "notification-sqs.json").write_text(json.dumps(notification, indent=2) + "\n")

inbound = {
    "id": "sqs-inbound-smoke-1",
    "event": "human_feedback_requested",
    "request": {
        "message": "LocalStack inbound SQS smoke event for EventOps proof",
        "thread_key": "durable-ops-queue",
    },
    "delivery_id": "eventops:sqs:inbound:1",
}
(tmpdir / "inbound-message.json").write_text(json.dumps(inbound, separators=(",", ":")))
PY

log "registering LocalStack-backed inbound SQS event source"
curl -fsS -X POST "$event_gateway_url/event-sources" \
  -H 'content-type: application/json' \
  --data @"$tmpdir/event-source-sqs.json" >"$tmpdir/register-source.json"

log "sending inbound event to LocalStack SQS"
message_body=$(sed -n '1p' "$tmpdir/inbound-message.json")
localstack_sqs send-message \
  --queue-url "$inbound_queue_internal_url" \
  --message-body "$message_body" >"$tmpdir/inbound-send.json"

log "polling inbound SQS through event-gateway"
curl -fsS -X POST "$event_gateway_url/events/sqs/sqs-notifications/poll?max_messages=10&route=true" \
  >"$tmpdir/sqs-poll.json"
curl -fsS -X POST "$event_gateway_url/events/sqs/sqs-notifications/poll?max_messages=10&route=true" \
  >"$tmpdir/sqs-poll-empty.json"
curl -fsS "$event_gateway_url/events?source_id=sqs-notifications" >"$tmpdir/events.json"
curl -fsS "$event_gateway_url/triggers" >"$tmpdir/triggers.json"

log "delivering outbound notification to LocalStack SQS"
curl -fsS -X POST "$notifier_url/notify" \
  -H 'content-type: application/json' \
  --data @"$tmpdir/notification-sqs.json" >"$tmpdir/notifier-report.json"
curl -fsS "$notifier_url/queue" >"$tmpdir/notifier-queue.json"
localstack_sqs receive-message \
  --queue-url "$outbound_queue_internal_url" \
  --max-number-of-messages 1 \
  --wait-time-seconds 1 \
  --output json >"$tmpdir/outbound-receive.json"

python3 - "$tmpdir" "$event_gateway_journal" "$goal_store_journal" "$notifier_journal" <<'PY'
import json
import pathlib
import sys

tmpdir = pathlib.Path(sys.argv[1])
event_gateway_journal = pathlib.Path(sys.argv[2])
goal_store_journal = pathlib.Path(sys.argv[3])
notifier_journal = pathlib.Path(sys.argv[4])

def load(name):
    with (tmpdir / name).open() as handle:
        text = handle.read().strip()
        if not text:
            return {}
        return json.loads(text)

def require(condition, message):
    if not condition:
        raise SystemExit(message)

registered = load("register-source.json")
require(registered["id"] == "sqs-notifications", registered)
require(registered["enabled"] is True, registered)
require(registered["sqs"]["delete_on_success"] is True, registered)

poll = load("sqs-poll.json")
require(poll["source_id"] == "sqs-notifications", poll)
require(poll["received"] == 1, poll)
require(poll["accepted"] == 1, poll)
require(poll["deduped"] == 0, poll)
require(poll["deleted"] == 1, poll)
require(poll["failures"] == [], poll)
require(len(poll["events"]) == 1, poll)
event = poll["events"][0]
require(event["id"] == "sqs-inbound-smoke-1", event)
require(event["source_id"] == "sqs-notifications", event)
require(event["source_kind"] == "sqs", event)
require(event["event_type"] == "human_feedback_requested", event)
require(event["subject"] == "LocalStack inbound SQS smoke event for EventOps proof", event)
require(event["payload"]["_sqs"]["message_id"], event)
require(len(poll["routes"]) == 1, poll)
require(poll["routes"][0]["status"] == "awaiting_human_review", poll["routes"][0])

empty = load("sqs-poll-empty.json")
require(empty["received"] == 0, empty)
require(empty["accepted"] == 0, empty)

events = load("events.json")
require(len(events) == 1, events)
require(events[0]["id"] == "sqs-inbound-smoke-1", events)

triggers = load("triggers.json")
require(len(triggers) == 1, triggers)
require(triggers[0]["event_id"] == "sqs-inbound-smoke-1", triggers)

reports = load("notifier-report.json")
require(len(reports) == 1, reports)
report = reports[0]
require(report["delivered"] is True, report)
require(str(report["external_ref"]).startswith("sqs://message/"), report)
require(report["error"] is None, report)
require(report["target"]["kind"] == "sqs", report)
require(report["target"]["require_ack"] is True, report)

queue = load("notifier-queue.json")
require(len(queue) == 1, queue)
require(queue[0]["thread_key"] == "durable-ops-queue", queue)
require(queue[0]["require_ack"] is True, queue)

outbound = load("outbound-receive.json")
messages = outbound.get("Messages", [])
require(len(messages) == 1, outbound)
envelope = json.loads(messages[0]["Body"])
require(envelope["provider"] == "sqs", envelope)
require(envelope["event"] == "human_feedback_requested", envelope)
require(envelope["require_ack"] is True, envelope)
require(envelope["request"]["message"].startswith("A durable goal is waiting"), envelope)
require(envelope["request"]["policy"]["feedback_thread_key"] == "durable-ops-queue", envelope)

gateway_entries = [
    json.loads(line)
    for line in event_gateway_journal.read_text().splitlines()
    if line.strip()
]
require([entry["type"] for entry in gateway_entries] == ["source", "event", "trigger"], gateway_entries)
require(gateway_entries[1]["id"] == "sqs-inbound-smoke-1", gateway_entries[1])
require(gateway_entries[2]["status"] == "awaiting_human_review", gateway_entries[2])

store_entries = [
    json.loads(line)
    for line in (goal_store_journal.read_text() if goal_store_journal.exists() else "").splitlines()
    if line.strip()
]
require(store_entries == [], store_entries)

notifier_entries = [
    json.loads(line)
    for line in notifier_journal.read_text().splitlines()
    if line.strip()
]
require([entry["type"] for entry in notifier_entries] == ["outbox", "thread"], notifier_entries)
require(notifier_entries[0]["status"] == "awaiting_ack", notifier_entries[0])
require(notifier_entries[0]["external_ref"].startswith("sqs://message/"), notifier_entries[0])
require(notifier_entries[1]["thread_key"] == "durable-ops-queue", notifier_entries[1])

print("eventops SQS LocalStack smoke assertions passed")
PY

log "passed"
