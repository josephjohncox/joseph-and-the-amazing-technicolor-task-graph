#!/bin/sh
set -eu

fail() {
  printf 'coat local reset failed: %s\n' "$*" >&2
  exit 1
}

log() {
  printf '[coat-reset] %s\n' "$*"
}

usage() {
  cat <<'EOF'
Usage:
  sh scripts/coat-local-reset.sh --mode scenario --dry-run
  sh scripts/coat-local-reset.sh --mode bootstrap --dry-run
  sh scripts/coat-local-reset.sh --mode evidence
  sh scripts/coat-local-reset.sh --mode stack
  sh scripts/coat-local-reset.sh --compose-stack --delete-volumes

Safe defaults:
  With no action flags, this helper only prints help.
  Evidence cleanup removes known generated run directories under target/.
  No shortcut mode deletes Compose volumes; --delete-volumes must be explicit.

Actions:
  --mode scenario          Remove generated scenario evidence for known specs.
  --mode bootstrap         Remove generated bootstrap scenario evidence.
  --mode evidence          Remove scenario and bootstrap evidence.
  --mode stack             Stop and remove the local Docker Compose stack.
  --mode local             Remove evidence and stop the stack, without volumes.
  --scenario-evidence       Remove generated scenario evidence.
  --bootstrap-evidence      Also remove generated bootstrap scenario evidence.
  --compose-stack           Stop and remove the local Docker Compose stack.
  --delete-volumes          With --compose-stack, remove COAT local Compose volumes.

Options:
  --dry-run                 Print actions without removing files or running Compose.
  --scenario-out PATH       Scenario evidence dir. Default: target/coat-scenarios.
  --scenario-specs LIST     Scenario spec paths/globs used to derive run dirs.
  --bootstrap-out PATH      Bootstrap evidence dir. Default: target/coat-scenarios/bootstrap.
  --bootstrap-specs LIST    Bootstrap specs for --bootstrap-out.
  --bootstrap-extra-out PATH
                            Extra bootstrap evidence dir. Default: target/coat-bootstrap-scenarios.
  --bootstrap-extra-specs LIST
                            Specs for --bootstrap-extra-out.
  --env-file PATH           Pass a provider env file to docker compose down.
  --restate-cloud           Include the Restate Cloud compose overlay.
  --restate-cloud-env-file PATH
                            Restate Cloud env file. Default: infra/compose/restate-cloud.env.
  --project-name NAME       Set COMPOSE_PROJECT_NAME for Compose cleanup.
  -h, --help                Show this help.

Examples:
  make scenario-reset-dry-run
  make scenario-reset
  make bootstrap-reset-dry-run
  RESET_BOOTSTRAP=1 make scenario-reset
  sh scripts/coat-local-reset.sh --mode evidence --dry-run
  sh scripts/coat-local-reset.sh --compose-stack
  sh scripts/coat-local-reset.sh --compose-stack --delete-volumes
EOF
}

script_dir=$(CDPATH= cd "$(dirname "$0")" && pwd)
repo_root=$(CDPATH= cd "$script_dir/.." && pwd)
cd "$repo_root"

clear_scenario=0
clear_bootstrap=0
compose_stack=0
delete_volumes=0
dry_run=${COAT_RESET_DRY_RUN:-0}
scenario_out=${COAT_RESET_SCENARIO_OUT:-${COAT_SCENARIO_E2E_OUT:-target/coat-scenarios}}
scenario_specs=${COAT_RESET_SCENARIO_SPECS:-${COAT_SCENARIO_E2E_SPECS:-scenarios/e2e/*.json}}
bootstrap_out=${COAT_RESET_BOOTSTRAP_OUT:-${COAT_BOOTSTRAP_SCENARIO_OUT:-target/coat-scenarios/bootstrap}}
bootstrap_specs=${COAT_RESET_BOOTSTRAP_SPECS:-"scenarios/e2e/goal_lifecycle_basic.json scenarios/e2e/blocked_and_resumed.json scenarios/e2e/fanout_until_done.json"}
bootstrap_extra_out=${COAT_RESET_BOOTSTRAP_EXTRA_OUT:-target/coat-bootstrap-scenarios}
bootstrap_extra_specs=${COAT_RESET_BOOTSTRAP_EXTRA_SPECS:-${COAT_BOOTSTRAP_SCENARIO_SPECS:-"scenarios/e2e/bootstrap_basic.json scenarios/e2e/bootstrap_human_input_thunk_resume.json scenarios/e2e/bootstrap_approval.json scenarios/e2e/bootstrap_fanout.json scenarios/e2e/bootstrap_fork_join.json scenarios/e2e/bootstrap_signal_driven.json scenarios/e2e/blocked_and_resumed.json"}}
compose_env_file=${COAT_RESET_COMPOSE_ENV_FILE:-}
restate_cloud=0
restate_cloud_env_file=${COAT_RESET_RESTATE_CLOUD_ENV_FILE:-infra/compose/restate-cloud.env}
compose_project_name=${COAT_RESET_COMPOSE_PROJECT_NAME:-}

if [ "$#" -eq 0 ]; then
  usage
  exit 0
fi

while [ "$#" -gt 0 ]; do
  case "$1" in
    --mode)
      shift
      [ "$#" -gt 0 ] || fail "--mode requires scenario, bootstrap, evidence, stack, or local"
      case "$1" in
        scenario)
          clear_scenario=1
          ;;
        bootstrap)
          clear_bootstrap=1
          ;;
        evidence)
          clear_scenario=1
          clear_bootstrap=1
          ;;
        stack)
          compose_stack=1
          ;;
        local)
          clear_scenario=1
          clear_bootstrap=1
          compose_stack=1
          ;;
        *)
          fail "unknown reset mode: $1"
          ;;
      esac
      ;;
    --scenario-evidence)
      clear_scenario=1
      ;;
    --bootstrap-evidence)
      clear_bootstrap=1
      ;;
    --compose-stack)
      compose_stack=1
      ;;
    --delete-volumes)
      compose_stack=1
      delete_volumes=1
      ;;
    --dry-run)
      dry_run=1
      ;;
    --scenario-out)
      shift
      [ "$#" -gt 0 ] || fail "--scenario-out requires a path"
      scenario_out=$1
      ;;
    --scenario-specs)
      shift
      [ "$#" -gt 0 ] || fail "--scenario-specs requires a quoted list"
      scenario_specs=$1
      ;;
    --bootstrap-out)
      shift
      [ "$#" -gt 0 ] || fail "--bootstrap-out requires a path"
      bootstrap_out=$1
      ;;
    --bootstrap-specs)
      shift
      [ "$#" -gt 0 ] || fail "--bootstrap-specs requires a quoted list"
      bootstrap_specs=$1
      ;;
    --bootstrap-extra-out)
      shift
      [ "$#" -gt 0 ] || fail "--bootstrap-extra-out requires a path"
      bootstrap_extra_out=$1
      ;;
    --bootstrap-extra-specs)
      shift
      [ "$#" -gt 0 ] || fail "--bootstrap-extra-specs requires a quoted list"
      bootstrap_extra_specs=$1
      ;;
    --env-file)
      shift
      [ "$#" -gt 0 ] || fail "--env-file requires a path"
      compose_env_file=$1
      ;;
    --restate-cloud)
      restate_cloud=1
      ;;
    --restate-cloud-env-file)
      shift
      [ "$#" -gt 0 ] || fail "--restate-cloud-env-file requires a path"
      restate_cloud_env_file=$1
      ;;
    --project-name)
      shift
      [ "$#" -gt 0 ] || fail "--project-name requires a name"
      compose_project_name=$1
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

if [ "$clear_scenario" = "0" ] && [ "$clear_bootstrap" = "0" ] && [ "$compose_stack" = "0" ]; then
  usage
  exit 0
fi

case "$dry_run" in
  0|1) ;;
  *) fail "COAT_RESET_DRY_RUN must be 0 or 1" ;;
esac

print_command() {
  printf '[coat-reset] %s:' "$1"
  shift
  for arg in "$@"; do
    printf ' %s' "$arg"
  done
  printf '\n'
}

run_cmd() {
  if [ "$dry_run" = "1" ]; then
    print_command "dry-run" "$@"
    return 0
  fi
  print_command "running" "$@"
  "$@"
}

trim_trailing_slashes() {
  tts_path=$1
  while [ "$tts_path" != "/" ] && [ "${tts_path%/}" != "$tts_path" ]; do
    tts_path=${tts_path%/}
  done
  printf '%s\n' "$tts_path"
}

ensure_generated_target_path() {
  egtp_label=$1
  egtp_path=$(trim_trailing_slashes "$2")
  case "$egtp_path" in
    ""|"/"|".")
      fail "refusing unsafe $egtp_label path: $egtp_path"
      ;;
    -*)
      fail "refusing $egtp_label path beginning with '-': $egtp_path"
      ;;
    target|target/|./target|./target/)
      fail "refusing to remove broad $egtp_label path: $egtp_path"
      ;;
    *..*)
      fail "refusing $egtp_label path with parent traversal: $egtp_path"
      ;;
    target/*|./target/*)
      ;;
    *)
      fail "$egtp_label path must be under target/: $egtp_path"
      ;;
  esac
  if [ -L "$egtp_path" ]; then
    fail "refusing to remove symlinked $egtp_label path: $egtp_path"
  fi
}

remove_generated_dir() {
  rgd_label=$1
  rgd_path=$(trim_trailing_slashes "$2")
  ensure_generated_target_path "$rgd_label" "$rgd_path"
  if [ -e "$rgd_path" ]; then
    run_cmd rm -rf "$rgd_path"
  else
    log "$rgd_label already absent: $rgd_path"
  fi
}

scenario_id_from_spec() {
  sis_spec=$1
  sis_id=$(awk -F'"' '/^[[:space:]]*"id"[[:space:]]*:/ { print $4; exit }' "$sis_spec")
  case "$sis_id" in
    ""|*[!A-Za-z0-9_.-]*)
      fail "could not derive a safe scenario id from $sis_spec"
      ;;
  esac
  printf '%s\n' "$sis_id"
}

remove_scenario_run_dirs() {
  rsrd_label=$1
  rsrd_out_root=$(trim_trailing_slashes "$2")
  rsrd_specs=$3
  rsrd_matched=0

  ensure_generated_target_path "$rsrd_label root" "$rsrd_out_root"
  remove_generated_dir "$rsrd_label latest run" "$rsrd_out_root/latest"

  for rsrd_spec in $rsrd_specs; do
    [ -f "$rsrd_spec" ] || continue
    rsrd_matched=1
    rsrd_scenario_id=$(scenario_id_from_spec "$rsrd_spec")
    remove_generated_dir "$rsrd_label $rsrd_scenario_id run" "$rsrd_out_root/$rsrd_scenario_id"
  done

  if [ "$rsrd_matched" = "0" ]; then
    log "no $rsrd_label specs matched; no scenario run dirs removed from $rsrd_out_root"
  fi
}

docker_compose_available() {
  command -v docker >/dev/null 2>&1 || return 1
  docker compose version >/dev/null 2>&1 || return 1
  return 0
}

compose_down() {
  [ -f infra/compose/docker-compose.yml ] || fail "missing infra/compose/docker-compose.yml; run from a COAT checkout"

  if [ "$dry_run" != "1" ]; then
    docker_compose_available || fail "docker compose is not available"
    if [ -n "$compose_env_file" ] && [ ! -f "$compose_env_file" ]; then
      fail "compose env file does not exist: $compose_env_file"
    fi
    if [ "$restate_cloud" = "1" ] && [ ! -f "$restate_cloud_env_file" ]; then
      fail "Restate Cloud env file does not exist: $restate_cloud_env_file"
    fi
  fi

  if [ -n "$compose_project_name" ]; then
    export COMPOSE_PROJECT_NAME="$compose_project_name"
  fi

  set -- docker compose
  if [ "$restate_cloud" = "1" ]; then
    set -- "$@" --env-file "$restate_cloud_env_file"
  fi
  if [ -n "$compose_env_file" ]; then
    set -- "$@" --env-file "$compose_env_file"
  fi
  set -- "$@" -f infra/compose/docker-compose.yml --profile db
  if [ "$restate_cloud" = "1" ]; then
    set -- "$@" -f infra/compose/docker-compose.restate-cloud.yml --profile restate-cloud --profile local-restate
  fi
  set -- "$@" down --remove-orphans
  if [ "$delete_volumes" = "1" ]; then
    set -- "$@" --volumes
  fi
  run_cmd "$@"
}

if [ "$compose_stack" = "1" ]; then
  if [ "$delete_volumes" = "1" ]; then
    log "stopping local Compose stack and deleting COAT local stack volumes"
  else
    log "stopping local Compose stack without deleting volumes"
  fi
  compose_down
fi

if [ "$clear_scenario" = "1" ]; then
  remove_scenario_run_dirs "scenario evidence" "$scenario_out" "$scenario_specs"
fi

if [ "$clear_bootstrap" = "1" ]; then
  remove_scenario_run_dirs "bootstrap evidence" "$bootstrap_out" "$bootstrap_specs"
  if [ "$bootstrap_extra_out" != "$bootstrap_out" ]; then
    remove_scenario_run_dirs "bootstrap evidence compatibility" "$bootstrap_extra_out" "$bootstrap_extra_specs"
  fi
fi

log "reset complete"
