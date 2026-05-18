#!/bin/sh
set -eu

fail() {
  printf 'coat exercise system failed: %s\n' "$*" >&2
  exit 1
}

log() {
  printf '[coat-exercise] %s\n' "$*"
}

usage() {
  cat <<'EOF'
Usage:
  sh scripts/coat-exercise-system.sh [mode] [options]
  sh scripts/coat-exercise-system.sh --mode MODE [options]

Modes:
  quick   Syntax/reset smoke, deterministic bootstrap fixtures, runner smoke,
          and event-gateway smoke. No Compose stack is started directly here.
  demo    Start or reuse the deterministic local stub stack, then seed
          navigable live/demo goals and fixture projections for the SPA/TUI.
  e2e     Run deterministic backend scenario and task-graph validation suites.
  ui      Run browser UI E2E against the deterministic local stub stack.
  full    Run quick, e2e, ui, demo, SQS, and Compose runner exercises.

Options:
  --mode MODE        Select quick, demo, e2e, ui, or full.
  --output-dir PATH  Summary/evidence root. Default: target/coat-scenarios/latest
  --keep-going       Continue after a failed step and report all failures.
  --dry-run          Print and record the commands without executing them.
  -h, --help         Show this help.

Output:
  Always writes target/coat-scenarios/latest/system-exercise.json by default.
  Step stdout/stderr logs are written below
  target/coat-scenarios/latest/system-exercise/<timestamp>/.
EOF
}

script_dir=$(CDPATH= cd "$(dirname "$0")" && pwd)
repo_root=$(CDPATH= cd "$script_dir/.." && pwd)
cd "$repo_root"

mode=
out_root=${COAT_EXERCISE_OUT:-target/coat-scenarios/latest}
keep_going=${COAT_EXERCISE_KEEP_GOING:-0}
dry_run=${COAT_EXERCISE_DRY_RUN:-0}

if [ "$#" -gt 0 ]; then
  case "$1" in
    quick|demo|e2e|ui|full)
      mode=$1
      shift
      ;;
  esac
fi

while [ "$#" -gt 0 ]; do
  case "$1" in
    --mode)
      shift
      [ "$#" -gt 0 ] || fail "--mode requires quick, demo, e2e, ui, or full"
      mode=$1
      ;;
    --output-dir)
      shift
      [ "$#" -gt 0 ] || fail "--output-dir requires a path"
      out_root=$1
      ;;
    --keep-going)
      keep_going=1
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

[ -n "$mode" ] || mode=quick

case "$mode" in
  quick|demo|e2e|ui|full) ;;
  *) fail "unknown mode: $mode" ;;
esac

case "$keep_going" in
  0|1) ;;
  true|yes) keep_going=1 ;;
  false|no) keep_going=0 ;;
  *) fail "COAT_EXERCISE_KEEP_GOING must be 0 or 1" ;;
esac

case "$dry_run" in
  0|1) ;;
  true|yes) dry_run=1 ;;
  false|no) dry_run=0 ;;
  *) fail "COAT_EXERCISE_DRY_RUN must be 0 or 1" ;;
esac

case "$out_root" in
  ""|"/"|".") fail "refusing unsafe output directory: $out_root" ;;
esac

timestamp=$(date -u '+%Y%m%dT%H%M%SZ')
run_dir="$out_root/system-exercise/$timestamp-$$"
summary_path="$out_root/system-exercise.json"
results_file="$run_dir/steps.tsv"
overall_status=passed

mkdir -p "$run_dir"
: >"$results_file"

json_escape() {
  printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g; s/	/\\t/g'
}

command_string() {
  first=1
  for arg in "$@"; do
    if [ "$first" = "1" ]; then
      first=0
    else
      printf ' '
    fi
    printf '%s' "$arg"
  done
}

record_step() {
  step_name=$1
  step_status=$2
  step_dir=$3
  shift 3
  step_command=$(command_string "$@")
  printf '%s\t%s\t%s\t%s\n' "$step_name" "$step_status" "$step_dir" "$step_command" >>"$results_file"
}

write_summary() {
  finished_at=$(date -u '+%Y-%m-%dT%H:%M:%SZ')
  {
    printf '{\n'
    printf '  "schema_version": 1,\n'
    printf '  "mode": "%s",\n' "$(json_escape "$mode")"
    printf '  "status": "%s",\n' "$(json_escape "$overall_status")"
    printf '  "dry_run": %s,\n' "$dry_run"
    printf '  "started_at": "%s",\n' "$(json_escape "$started_at")"
    printf '  "finished_at": "%s",\n' "$(json_escape "$finished_at")"
    printf '  "repo_root": "%s",\n' "$(json_escape "$repo_root")"
    printf '  "run_dir": "%s",\n' "$(json_escape "$run_dir")"
    printf '  "steps": [\n'
    first_step=1
    while IFS='	' read -r step_name step_status step_dir step_command; do
      [ -n "$step_name" ] || continue
      if [ "$first_step" = "1" ]; then
        first_step=0
      else
        printf ',\n'
      fi
      printf '    {\n'
      printf '      "name": "%s",\n' "$(json_escape "$step_name")"
      printf '      "status": "%s",\n' "$(json_escape "$step_status")"
      printf '      "command": "%s",\n' "$(json_escape "$step_command")"
      printf '      "artifacts": "%s"\n' "$(json_escape "$step_dir")"
      printf '    }'
    done <"$results_file"
    printf '\n'
    printf '  ]\n'
    printf '}\n'
  } >"$summary_path"
}

run_step() {
  step_name=$1
  shift
  step_dir="$run_dir/$step_name"
  mkdir -p "$step_dir"
  command_string "$@" >"$step_dir/command.txt"
  printf '\n' >>"$step_dir/command.txt"

  log "$step_name: $(command_string "$@")"
  if [ "$dry_run" = "1" ]; then
    printf 'dry-run\n' >"$step_dir/stdout.log"
    : >"$step_dir/stderr.log"
    printf '0\n' >"$step_dir/status.txt"
    record_step "$step_name" skipped "$step_dir" "$@"
    return 0
  fi

  set +e
  "$@" >"$step_dir/stdout.log" 2>"$step_dir/stderr.log"
  status=$?
  set -e
  printf '%s\n' "$status" >"$step_dir/status.txt"

  if [ "$status" -eq 0 ]; then
    record_step "$step_name" passed "$step_dir" "$@"
    return 0
  fi

  overall_status=failed
  record_step "$step_name" failed "$step_dir" "$@"
  if [ "$keep_going" = "1" ]; then
    log "$step_name failed; continuing because --keep-going is set"
    return 0
  fi

  write_summary
  fail "$step_name failed; see $step_dir and $summary_path"
}

run_quick() {
  run_step reset-smoke make reset-smoke
  run_step bootstrap-scenarios make bootstrap-scenarios
  run_step runner-smoke make runner-smoke
  run_step event-gateway-smoke make event-gateway-smoke
}

run_demo() {
  run_step scenario-e2e-stack make scenario-e2e-stack SCENARIO_E2E_KEEP_STACK=1
  run_step bootstrap-goals make bootstrap-goals
}

run_e2e() {
  run_step scenario-e2e make scenario-e2e
  run_step task-graph-validation make task-graph-validation
}

run_ui() {
  run_step scenario-e2e-ui-live make scenario-e2e-ui-live
}

run_full() {
  run_quick
  run_e2e
  run_ui
  run_demo
  run_step eventops-sqs-smoke make eventops-sqs-smoke
  run_step compose-runner-smoke make compose-runner-smoke
}

started_at=$(date -u '+%Y-%m-%dT%H:%M:%SZ')

log "mode=$mode"
log "summary=$summary_path"
log "artifacts=$run_dir"

case "$mode" in
  quick) run_quick ;;
  demo) run_demo ;;
  e2e) run_e2e ;;
  ui) run_ui ;;
  full) run_full ;;
esac

write_summary

if [ "$overall_status" = "passed" ]; then
  log "complete; summary: $summary_path"
  exit 0
fi

fail "one or more steps failed; summary: $summary_path"
