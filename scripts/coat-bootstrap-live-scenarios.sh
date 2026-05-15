#!/bin/sh
set -eu

fail() {
  printf 'live bootstrap goals failed: %s\n' "$*" >&2
  exit 1
}

log() {
  printf '[coat-bootstrap-live] %s\n' "$*"
}

script_dir=$(CDPATH= cd "$(dirname "$0")" && pwd)
repo_root=$(CDPATH= cd "$script_dir/.." && pwd)
cd "$repo_root"

stale_source=

target_debug_coat_selected() {
  case "$1" in
    target/debug/coat|./target/debug/coat|"$repo_root"/target/debug/coat)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

source_newer_than_binary() {
  binary=$1
  stale_source=
  for path in Cargo.toml Cargo.lock crates/cli/src crates/domain/src; do
    [ -e "$path" ] || continue
    newer=$(find "$path" -type f -newer "$binary" -print -quit 2>/dev/null || true)
    if [ -n "$newer" ]; then
      stale_source=$newer
      return 0
    fi
  done
  return 1
}

ensure_fresh_target_debug_coat() {
  binary=$1
  case "$binary" in
    target/debug/coat)
      binary="$repo_root/target/debug/coat"
      ;;
    ./target/debug/coat)
      binary="$repo_root/target/debug/coat"
      ;;
  esac
  [ -x "$binary" ] || fail "target/debug/coat is missing; run 'cargo build -p coat-cli --bin coat' or set COAT=/path/to/fresh/coat"
  if source_newer_than_binary "$binary"; then
    fail "target/debug/coat is older than $stale_source; run 'cargo build -p coat-cli --bin coat' or set COAT=/path/to/fresh/coat"
  fi
}

ensure_selected_coat_fresh() {
  selected=$1
  if target_debug_coat_selected "$selected"; then
    ensure_fresh_target_debug_coat "$selected"
    return
  fi
  resolved=$(command -v "$selected" 2>/dev/null || true)
  if [ "$resolved" = "$repo_root/target/debug/coat" ]; then
    ensure_fresh_target_debug_coat "$resolved"
  fi
}

coat=${COAT:-}
if [ -z "$coat" ]; then
  if command -v coat >/dev/null 2>&1; then
    coat=coat
  elif [ -x target/debug/coat ]; then
    coat=target/debug/coat
  else
    fail "could not find installed coat or target/debug/coat; run 'cargo build -p coat-cli --bin coat' or set COAT=/path/to/fresh/coat"
  fi
elif [ "$coat" = "coat" ] && ! command -v coat >/dev/null 2>&1; then
  if [ -x target/debug/coat ]; then
    coat=target/debug/coat
  else
    fail "COAT=coat but no installed coat or target/debug/coat was found; run 'cargo build -p coat-cli --bin coat' or set COAT=/path/to/fresh/coat"
  fi
fi
ensure_selected_coat_fresh "$coat"

out_root=${COAT_BOOTSTRAP_LIVE_OUT:-target/coat-scenarios/live-bootstrap}
restate_ingress=${COAT_BOOTSTRAP_LIVE_RESTATE_INGRESS:-${COAT_RESTATE_INGRESS:-http://localhost:8080}}
goal_store_url=${COAT_BOOTSTRAP_LIVE_GOAL_STORE_URL:-${COAT_GOAL_STORE_URL:-http://localhost:9088}}
dry_run=${COAT_BOOTSTRAP_LIVE_DRY_RUN:-0}

completed_goal_id=00000000-0000-4000-8000-000000004004
approval_goal_id=00000000-0000-4000-8000-000000004002
human_prompt_goal_id=00000000-0000-4000-8000-000000004003

mkdir -p "$out_root"

case "$dry_run" in
  0|1) ;;
  *) fail "COAT_BOOTSTRAP_LIVE_DRY_RUN must be 0 or 1" ;;
esac

print_command() {
  printf '%s:' "$1"
  shift
  for arg in "$@"; do
    printf ' %s' "$arg"
  done
  printf '\n'
}

run_capture() {
  label=$1
  shift
  output="$out_root/$label.json"
  print_command "$label" "$@" >> "$out_root/commands.txt"
  if [ "$dry_run" = "1" ]; then
    {
      printf '{\n'
      printf '  "dry_run": true,\n'
      printf '  "label": "%s"\n' "$label"
      printf '}\n'
    } > "$output"
    return 0
  fi
  log "running $label"
  "$@" > "$output"
}

if [ "$dry_run" != "1" ]; then
  command -v curl >/dev/null 2>&1 || fail "curl is required for goal-store health checks"
  curl -fsS "$goal_store_url/healthz" > "$out_root/goal-store-health.txt" \
    || fail "goal-store is not reachable at $goal_store_url; start the local stack with 'coat deploy local up --allow-stub-runners'"
fi

: > "$out_root/commands.txt"

log "using $coat"
log "Restate ingress: $restate_ingress"
log "goal-store: $goal_store_url"
log "writing evidence under $out_root"

run_capture completed-submit \
  "$coat" goal submit --file examples/bootstrap-live/completed-executor-lifecycle.json --restate-ingress "$restate_ingress"
run_capture approval-submit \
  "$coat" goal submit --file examples/bootstrap-live/approval-pending-task.json --restate-ingress "$restate_ingress"
run_capture human-prompt-submit \
  "$coat" goal submit --file examples/bootstrap-live/human-prompt-pending-task.json --restate-ingress "$restate_ingress"
run_capture human-prompt-thunk-create \
  "$coat" goal thunk create --goal-id "$human_prompt_goal_id" --file examples/bootstrap-live/human-input-thunk.json --restate-ingress "$restate_ingress"

run_capture goal-list \
  "$coat" store goals --goal-store-url "$goal_store_url"
run_capture completed-goal \
  "$coat" store goal --goal-store-url "$goal_store_url" --goal-id "$completed_goal_id"
run_capture completed-tasks \
  "$coat" store tasks --goal-store-url "$goal_store_url" --goal-id "$completed_goal_id"
run_capture approval-goal \
  "$coat" store goal --goal-store-url "$goal_store_url" --goal-id "$approval_goal_id"
run_capture approval-tasks \
  "$coat" store tasks --goal-store-url "$goal_store_url" --goal-id "$approval_goal_id"
run_capture approval-records \
  "$coat" store goal-approvals --goal-store-url "$goal_store_url" --goal-id "$approval_goal_id"
run_capture human-prompt-goal \
  "$coat" store goal --goal-store-url "$goal_store_url" --goal-id "$human_prompt_goal_id"
run_capture human-prompt-tasks \
  "$coat" store tasks --goal-store-url "$goal_store_url" --goal-id "$human_prompt_goal_id"
run_capture human-prompt-compute-graph \
  "$coat" goal compute-graph --goal-id "$human_prompt_goal_id" --restate-ingress "$restate_ingress"

cat > "$out_root/summary.txt" <<EOF
Created or refreshed live bootstrap goals through the COAT CLI and coordinator:

- Completed executor lifecycle: $completed_goal_id
- Approval pending task: $approval_goal_id
- Human prompt pending thunk: $human_prompt_goal_id

Open the SPA/TUI goal picker and select these goals to inspect completed work,
approval actions, pending human prompts, task graph state, and evidence.
EOF

log "complete"
