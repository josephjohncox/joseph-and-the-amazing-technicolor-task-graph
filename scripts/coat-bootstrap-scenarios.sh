#!/bin/sh
set -eu

fail() {
  printf 'bootstrap scenarios failed: %s\n' "$*" >&2
  exit 1
}

log() {
  printf '[coat-bootstrap-scenarios] %s\n' "$*"
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

out_root=${COAT_BOOTSTRAP_SCENARIO_OUT:-target/coat-scenarios/bootstrap}
gateway_url=${COAT_BOOTSTRAP_SCENARIO_GATEWAY_URL:-http://127.0.0.1:0}
goal_store_url=${COAT_BOOTSTRAP_GOAL_STORE_URL:-${COAT_GOAL_STORE_URL:-http://127.0.0.1:9088}}
seed_goals=${COAT_BOOTSTRAP_SEED_GOALS:-false}
scenario_args=${COAT_BOOTSTRAP_SCENARIO_ARGS:-}
specs=${COAT_BOOTSTRAP_SCENARIO_SPECS:-"scenarios/e2e/bootstrap_basic.json scenarios/e2e/bootstrap_human_input_thunk_resume.json scenarios/e2e/bootstrap_approval.json scenarios/e2e/bootstrap_fanout.json scenarios/e2e/bootstrap_fork_join.json scenarios/e2e/bootstrap_signal_driven.json scenarios/e2e/bootstrap_blocked_retry_recovery.json scenarios/e2e/bootstrap_cancelled_queue_history.json scenarios/e2e/bootstrap_memory_research_evidence.json scenarios/e2e/blocked_and_resumed.json scenarios/e2e/goal_lifecycle_basic.json"}

mkdir -p "$out_root"

goal_store_reachable() {
  command -v curl >/dev/null 2>&1 || return 1
  curl -fsS "$goal_store_url/healthz" >/dev/null 2>&1
}

log "using $coat"
log "writing evidence under $out_root"
case "$seed_goals" in
  true|1|yes)
    log "seeding scenario projections into $goal_store_url"
    ;;
  auto)
    if goal_store_reachable; then
      seed_goals=true
      log "goal-store is reachable; seeding scenario projections into $goal_store_url"
    else
      seed_goals=false
      log "goal-store is not reachable; running evidence-only bootstrap"
    fi
    ;;
  false|0|no)
    seed_goals=false
    log "goal-store seeding disabled"
    ;;
  *)
    fail "COAT_BOOTSTRAP_SEED_GOALS must be auto, true, or false"
    ;;
esac

for spec in $specs; do
  [ -f "$spec" ] || fail "missing scenario spec: $spec"
  log "running $spec"
  # shellcheck disable=SC2086
  "$coat" scenario run --file "$spec" --gateway-url "$gateway_url" --output-dir "$out_root" $scenario_args
  if [ "$seed_goals" = true ]; then
    "$coat" scenario seed --file "$spec" --goal-store-url "$goal_store_url"
  fi
done

log "complete"
