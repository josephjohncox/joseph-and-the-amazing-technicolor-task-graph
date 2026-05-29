#!/bin/sh
set -eu

fail() {
  printf 'live bootstrap goals failed: %s\n' "$*" >&2
  exit 1
}

log() {
  printf '[coat-bootstrap-live] %s\n' "$*"
}

usage() {
  cat <<'EOF'
Usage:
  sh scripts/coat-bootstrap-live-scenarios.sh [options]

Purpose:
  Seed local operator demo data through COAT CLI/backend flows.

What it creates:
  - real coordinator goals where the local stack can run them:
    completed executor lifecycle, pending approval, and pending human prompt;
  - deterministic fixture projections in goal-store:
    completed, pending action, approval, thunk/resume, fanout, fork/join,
    signal-driven, blocked recovery, cancelled queue history, and memory evidence.

Options:
  --output-dir PATH          Evidence output dir. Default: target/coat-scenarios/live-bootstrap
  --restate-ingress URL      Restate ingress. Default: COAT_RESTATE_INGRESS or http://localhost:8080
  --goal-store-url URL       Goal-store URL. Default: COAT_GOAL_STORE_URL or http://localhost:9088
  --submit-live-goals BOOL   Submit fixed coordinator demo goals. Default: true
  --seed-fixtures BOOL       Seed deterministic fixture projections. Default: true
  --force-resubmit           Resubmit fixed coordinator demo goals even if goal-store already has them.
  --dry-run                  Print commands and write dry-run captures.
  -h, --help                 Show this help.

Safety and idempotency:
  Fixed coordinator demo goals are skipped when goal-store already has them,
  unless --force-resubmit is used. Fixture seeding uses scenario idempotency keys.

Examples:
  coat deploy local up --allow-stub-runners
  sh scripts/coat-bootstrap-live-scenarios.sh
  sh scripts/coat-bootstrap-live-scenarios.sh --dry-run
  sh scripts/coat-bootstrap-live-scenarios.sh --seed-fixtures true --submit-live-goals false
EOF
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
submit_live_goals=${COAT_BOOTSTRAP_LIVE_SUBMIT_GOALS:-true}
seed_fixtures=${COAT_BOOTSTRAP_LIVE_SEED_FIXTURES:-true}
force_resubmit=${COAT_BOOTSTRAP_LIVE_FORCE_RESUBMIT:-false}
fixture_specs=${COAT_BOOTSTRAP_LIVE_FIXTURE_SPECS:-"scenarios/e2e/bootstrap_basic.json scenarios/e2e/bootstrap_running.json scenarios/e2e/bootstrap_pending_action.json scenarios/e2e/bootstrap_human_input_thunk_resume.json scenarios/e2e/bootstrap_approval.json scenarios/e2e/bootstrap_fanout.json scenarios/e2e/bootstrap_fork_join.json scenarios/e2e/bootstrap_signal_driven.json scenarios/e2e/bootstrap_blocked_retry_recovery.json scenarios/e2e/bootstrap_cancelled_queue_history.json scenarios/e2e/bootstrap_memory_research_evidence.json scenarios/e2e/operator_usability_workbench.json"}
last_goal_submit_status=

while [ "$#" -gt 0 ]; do
  case "$1" in
    --output-dir)
      shift
      [ "$#" -gt 0 ] || fail "--output-dir requires a path"
      out_root=$1
      ;;
    --restate-ingress)
      shift
      [ "$#" -gt 0 ] || fail "--restate-ingress requires a URL"
      restate_ingress=$1
      ;;
    --goal-store-url)
      shift
      [ "$#" -gt 0 ] || fail "--goal-store-url requires a URL"
      goal_store_url=$1
      ;;
    --submit-live-goals)
      shift
      [ "$#" -gt 0 ] || fail "--submit-live-goals requires true or false"
      submit_live_goals=$1
      ;;
    --seed-fixtures)
      shift
      [ "$#" -gt 0 ] || fail "--seed-fixtures requires true or false"
      seed_fixtures=$1
      ;;
    --fixture-specs)
      shift
      [ "$#" -gt 0 ] || fail "--fixture-specs requires a quoted spec list"
      fixture_specs=$1
      ;;
    --force-resubmit)
      force_resubmit=true
      ;;
    --dry-run)
      dry_run=1
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      fail "unknown argument: $1"
      ;;
  esac
  shift
done

completed_goal_id=00000000-0000-4000-8000-000000004004
approval_goal_id=00000000-0000-4000-8000-000000004002
human_prompt_goal_id=00000000-0000-4000-8000-000000004003

mkdir -p "$out_root"

case "$dry_run" in
  0|1) ;;
  *) fail "COAT_BOOTSTRAP_LIVE_DRY_RUN must be 0 or 1" ;;
esac

bool_arg() {
  case "$1" in
    true|1|yes) printf 'true\n' ;;
    false|0|no) printf 'false\n' ;;
    *) fail "$2 must be true or false" ;;
  esac
}

submit_live_goals=$(bool_arg "$submit_live_goals" "COAT_BOOTSTRAP_LIVE_SUBMIT_GOALS")
seed_fixtures=$(bool_arg "$seed_fixtures" "COAT_BOOTSTRAP_LIVE_SEED_FIXTURES")
force_resubmit=$(bool_arg "$force_resubmit" "COAT_BOOTSTRAP_LIVE_FORCE_RESUBMIT")

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

goal_exists() {
  ge_goal_id=$1
  [ "$dry_run" = "0" ] || return 1
  command -v curl >/dev/null 2>&1 || return 1
  curl -fsS "$goal_store_url/goal-store/goals/$ge_goal_id" 2>/dev/null \
    | grep -q '"found"[[:space:]]*:[[:space:]]*true'
}

run_goal_submit_if_needed() {
  rgsi_label=$1
  rgsi_goal_id=$2
  rgsi_file=$3
  rgsi_output="$out_root/$rgsi_label.json"
  if [ "$force_resubmit" = "false" ] && goal_exists "$rgsi_goal_id"; then
    log "skipping $rgsi_label; goal-store already has $rgsi_goal_id"
    print_command "$rgsi_label skipped-existing" "$coat" goal submit --file "$rgsi_file" --restate-ingress "$restate_ingress" >> "$out_root/commands.txt"
    {
      printf '{\n'
      printf '  "skipped": true,\n'
      printf '  "reason": "goal already exists in goal-store",\n'
      printf '  "goal_id": "%s"\n' "$rgsi_goal_id"
      printf '}\n'
    } > "$rgsi_output"
    last_goal_submit_status=skipped
    return 0
  fi
  run_capture "$rgsi_label" "$coat" goal submit --file "$rgsi_file" --restate-ingress "$restate_ingress"
  last_goal_submit_status=submitted
}

run_fixture_seed() {
  rfs_spec=$1
  rfs_base=$(basename "$rfs_spec" .json)
  [ -f "$rfs_spec" ] || fail "missing fixture scenario spec: $rfs_spec"
  run_capture "seed-$rfs_base" "$coat" scenario seed --file "$rfs_spec" --goal-store-url "$goal_store_url"
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

if [ "$submit_live_goals" = "true" ]; then
  human_prompt_submitted=0
  run_goal_submit_if_needed completed-submit "$completed_goal_id" examples/bootstrap-live/completed-executor-lifecycle.json
  run_goal_submit_if_needed approval-submit "$approval_goal_id" examples/bootstrap-live/approval-pending-task.json
  run_goal_submit_if_needed human-prompt-submit "$human_prompt_goal_id" examples/bootstrap-live/human-prompt-pending-task.json
  if [ "$last_goal_submit_status" = "submitted" ]; then
    human_prompt_submitted=1
  fi
  if [ "$human_prompt_submitted" = "1" ] || [ "$force_resubmit" = "true" ] || [ "$dry_run" = "1" ]; then
    run_capture human-prompt-thunk-create \
      "$coat" goal thunk create --goal-id "$human_prompt_goal_id" --file examples/bootstrap-live/human-input-thunk.json --restate-ingress "$restate_ingress"
  else
    log "skipping human-prompt-thunk-create; human prompt goal already existed"
    {
      printf '{\n'
      printf '  "skipped": true,\n'
      printf '  "reason": "human prompt goal already exists; not creating a duplicate thunk",\n'
      printf '  "goal_id": "%s"\n' "$human_prompt_goal_id"
      printf '}\n'
    } > "$out_root/human-prompt-thunk-create.json"
  fi
else
  log "live coordinator goal submission disabled"
fi

if [ "$seed_fixtures" = "true" ]; then
  for spec in $fixture_specs; do
    run_fixture_seed "$spec"
  done
else
  log "fixture projection seeding disabled"
fi

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
Created or refreshed bootstrap data through the COAT CLI and backend:

- Live coordinator goal, completed executor lifecycle: $completed_goal_id
- Live coordinator goal, approval pending task: $approval_goal_id
- Live coordinator goal, human prompt pending thunk: $human_prompt_goal_id

Fixture projections seeded into goal-store when enabled:
$fixture_specs

Open the SPA/TUI goal picker and select these goals to inspect completed work,
pending actions, approval actions, thunk/resume history, fanout, fork/join,
signal-driven work, task graph state, and evidence.
EOF

log "complete"
