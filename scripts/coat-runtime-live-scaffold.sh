#!/bin/sh
set -eu

fail() {
  printf 'coat runtime live scaffold failed: %s\n' "$*" >&2
  exit 1
}

log() {
  printf '[coat-runtime-live-scaffold] %s\n' "$*"
}

usage() {
  cat <<'EOF'
Usage:
  sh scripts/coat-runtime-live-scaffold.sh [options]

Options:
  --output-dir PATH  Summary/evidence root. Default: target/coat-runtime-live-scaffold
  --summary PATH     Summary JSON path. Default: <output-dir>/runtime-live-scaffold.json
  -h, --help         Show this help.

This is a readiness scaffold plus gated proof runner. With the default
environment it records skipped proof families and exits successfully. When an
explicit live gate is enabled, unsafe or incomplete configuration is reported as
failed. Proof families only start live infrastructure when their dedicated gate
is enabled; currently the Restate restart/resume proof can run Docker/Restate,
and the Codex App Server proof can run /verify plus a typed /run-task smoke
against an already-started App Server.
EOF
}

script_dir=$(CDPATH= cd "$(dirname "$0")" && pwd)
repo_root=$(CDPATH= cd "$script_dir/.." && pwd)
cd "$repo_root"

out_root=${COAT_RUNTIME_LIVE_SCAFFOLD_OUT:-target/coat-runtime-live-scaffold}
summary_path=

while [ "$#" -gt 0 ]; do
  case "$1" in
    --output-dir)
      shift
      [ "$#" -gt 0 ] || fail "--output-dir requires a path"
      out_root=$1
      ;;
    --summary)
      shift
      [ "$#" -gt 0 ] || fail "--summary requires a path"
      summary_path=$1
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

case "$out_root" in
  ""|"/"|".") fail "refusing unsafe output directory: $out_root" ;;
esac

[ -n "$summary_path" ] || summary_path="$out_root/runtime-live-scaffold.json"
run_dir="$out_root/proofs"
results_file="$out_root/proofs.tsv"
overall_status=passed
any_live_proof=0

mkdir -p "$run_dir"
: >"$results_file"

json_escape() {
  printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g; s/	/\\t/g'
}

env_enabled() {
  value=$1
  case "$(printf '%s' "$value" | tr '[:upper:]' '[:lower:]')" in
    1|true|yes|on) return 0 ;;
    *) return 1 ;;
  esac
}

env_value() {
  name=$1
  eval "printf '%s' \"\${$name:-}\""
}

command_exists() {
  command -v "$1" >/dev/null 2>&1
}

record_proof() {
  proof_name=$1
  proof_status=$2
  gate_name=$3
  gate_enabled_value=$4
  reason=$5
  next_step=$6
  live_proof_executed=${7:-false}
  proof_dir="$run_dir/$proof_name"
  mkdir -p "$proof_dir"
  {
    printf 'name=%s\n' "$proof_name"
    printf 'status=%s\n' "$proof_status"
    printf 'gate=%s\n' "$gate_name"
    printf 'gate_enabled=%s\n' "$gate_enabled_value"
    printf 'reason=%s\n' "$reason"
    printf 'next_step=%s\n' "$next_step"
    printf 'live_proof_executed=%s\n' "$live_proof_executed"
  } >"$proof_dir/status.txt"
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$proof_name" "$proof_status" "$gate_name" "$gate_enabled_value" "$reason" "$next_step" "$live_proof_executed" >>"$results_file"
  log "$proof_name: $proof_status - $reason"
  if [ "$live_proof_executed" = "true" ]; then
    any_live_proof=1
  fi
  if [ "$proof_status" = "failed" ]; then
    overall_status=failed
  fi
}

write_summary() {
  generated_at=$(date -u '+%Y-%m-%dT%H:%M:%SZ')
  {
    printf '{\n'
    printf '  "schema_version": 1,\n'
    printf '  "status": "%s",\n' "$(json_escape "$overall_status")"
    printf '  "generated_at": "%s",\n' "$(json_escape "$generated_at")"
    printf '  "repo_root": "%s",\n' "$(json_escape "$repo_root")"
    if [ "$any_live_proof" = "1" ]; then
      printf '  "live_proof_executed": true,\n'
      printf '  "note": "At least one explicitly gated live proof executed; remaining proof families may still be skipped or readiness-only.",\n'
    else
      printf '  "live_proof_executed": false,\n'
      printf '  "note": "Readiness scaffold only; no live proof gate was enabled.",\n'
    fi
    printf '  "proofs": [\n'
    first=1
    while IFS='	' read -r proof_name proof_status gate_name gate_enabled_value reason next_step live_proof_executed; do
      [ -n "$proof_name" ] || continue
      if [ "$first" = "1" ]; then
        first=0
      else
        printf ',\n'
      fi
      printf '    {\n'
      printf '      "name": "%s",\n' "$(json_escape "$proof_name")"
      printf '      "status": "%s",\n' "$(json_escape "$proof_status")"
      printf '      "gate": "%s",\n' "$(json_escape "$gate_name")"
      printf '      "gate_enabled": %s,\n' "$gate_enabled_value"
      printf '      "live_proof_executed": %s,\n' "$live_proof_executed"
      printf '      "reason": "%s",\n' "$(json_escape "$reason")"
      printf '      "next_step": "%s"\n' "$(json_escape "$next_step")"
      printf '    }'
    done <"$results_file"
    printf '\n'
    printf '  ]\n'
    printf '}\n'
  } >"$summary_path"
}

check_restate_restart_resume() {
  gate=COAT_RESTATE_RESTART_RESUME_TEST
  gate_raw=$(env_value "$gate")
  if ! env_enabled "$gate_raw"; then
    record_proof restate_restart_resume skipped "$gate" false \
      "explicit Restate live proof gate is not enabled" \
      "set COAT_RESTATE_RESTART_RESUME_TEST=1 only on a machine with Docker and the coordinator binary available"
    return
  fi

  image=${COAT_RESTATE_TESTCONTAINERS_IMAGE:-docker.restate.dev/restatedev/restate:1.5}
  coordinator_bin=$(env | sed -n 's/^CARGO_BIN_EXE_coat-coordinator=//p' | sed -n '1p')
  [ -n "$coordinator_bin" ] || coordinator_bin=target/debug/coat-coordinator

  case "$image" in
    *:latest)
      record_proof restate_restart_resume failed "$gate" true \
        "COAT_RESTATE_TESTCONTAINERS_IMAGE must be pinned, got $image" \
        "use a version-pinned Restate image before running the ignored coordinator test"
      return
      ;;
  esac

  if ! command_exists docker; then
    record_proof restate_restart_resume skipped "$gate" true \
      "docker CLI is unavailable; Docker-backed Restate proof cannot run here" \
      "install Docker or run this proof on a Docker-enabled host"
    return
  fi

  if [ ! -x "$coordinator_bin" ] && [ ! -f "$coordinator_bin" ]; then
    record_proof restate_restart_resume failed "$gate" true \
      "coordinator binary is missing at $coordinator_bin" \
      "run cargo test or cargo build -p coat-coordinator so CARGO_BIN_EXE_coat-coordinator is available"
    return
  fi

  proof_dir="$run_dir/restate_restart_resume"
  mkdir -p "$proof_dir"
  log "restate_restart_resume: running live proof with image $image"
  if COAT_RESTATE_RESTART_RESUME_TEST=1 \
    COAT_RESTATE_TESTCONTAINERS_IMAGE="$image" \
    cargo test -p coat-coordinator restate_restart_resume_proof_entrypoint -- --ignored --exact --nocapture \
    >"$proof_dir/live-proof.log" 2>&1; then
    record_proof restate_restart_resume passed "$gate" true \
      "live Restate restart/resume proof passed" \
      "keep this proof in release/runtime verification when Docker is available" \
      true
  else
    record_proof restate_restart_resume failed "$gate" true \
      "live Restate restart/resume proof failed; see $proof_dir/live-proof.log" \
      "fix the RuntimeVerifier harness or local Docker/Restate/coordinator configuration" \
      true
  fi
}

check_codex_app_server() {
  gate=COAT_CODEX_APP_SERVER_LIVE_PROOF
  gate_raw=$(env_value "$gate")
  if ! env_enabled "$gate_raw"; then
    record_proof codex_app_server skipped "$gate" false \
      "explicit Codex App Server live proof gate is not enabled" \
      "set COAT_CODEX_APP_SERVER_LIVE_PROOF=1 with CODEX_RUNNER_MODE=live, app-server auth, URL, and isolated workspace"
    return
  fi

  mode=${CODEX_RUNNER_MODE:-stub}
  auth_mode=${CODEX_AUTH_MODE:-env_api_key}
  app_server_url=${CODEX_APP_SERVER_URL:-}
  workspace=${CODEX_APP_SERVER_CWD:-${CODEX_WORKSPACE_DIR:-}}

  if [ "$mode" != "live" ]; then
    record_proof codex_app_server failed "$gate" true \
      "CODEX_RUNNER_MODE must be live, got $mode" \
      "set CODEX_RUNNER_MODE=live only for an isolated live worker smoke"
    return
  fi

  if [ "$auth_mode" != "app_server" ]; then
    record_proof codex_app_server failed "$gate" true \
      "CODEX_AUTH_MODE must be app_server, got $auth_mode" \
      "use runner-local App Server auth; do not put raw user tokens in task state"
    return
  fi

  case "$app_server_url" in
    ws://*|wss://*|http://*|https://*) ;;
    "")
      record_proof codex_app_server failed "$gate" true \
        "CODEX_APP_SERVER_URL is missing" \
        "point CODEX_APP_SERVER_URL at a reachable Codex App Server endpoint"
      return
      ;;
    *)
      record_proof codex_app_server failed "$gate" true \
        "CODEX_APP_SERVER_URL must use ws, wss, http, or https" \
        "use a supported App Server endpoint URL"
      return
      ;;
  esac

  if [ -z "$workspace" ] || [ ! -d "$workspace" ]; then
    record_proof codex_app_server failed "$gate" true \
      "CODEX_APP_SERVER_CWD or CODEX_WORKSPACE_DIR must name an existing isolated workspace" \
      "create an isolated task workspace before running live Codex execution"
    return
  fi

  if ! command_exists npm; then
    record_proof codex_app_server failed "$gate" true \
      "npm is unavailable; the TypeScript Codex runner proof cannot build" \
      "install Node/npm before running the live Codex App Server proof"
    return
  fi

  if ! command_exists node; then
    record_proof codex_app_server failed "$gate" true \
      "node is unavailable; the TypeScript Codex runner proof cannot run" \
      "install Node before running the live Codex App Server proof"
    return
  fi

  proof_dir="$run_dir/codex_app_server"
  mkdir -p "$proof_dir"
  request_path=${COAT_CODEX_APP_SERVER_PROOF_REQUEST:-examples/agent-run-smoke.json}
  log "codex_app_server: running live proof against $app_server_url"

  if ! npm run --prefix sidecars/codex-runner-ts build >"$proof_dir/build.log" 2>&1; then
    record_proof codex_app_server failed "$gate" true \
      "codex-runner-ts build failed; see $proof_dir/build.log" \
      "fix the TypeScript runner build before running live Codex execution" \
      true
    return
  fi

  if CODEX_VERIFY_APP_SERVER=1 \
    COAT_CODEX_APP_SERVER_PROOF_DIR="$proof_dir" \
    COAT_CODEX_APP_SERVER_PROOF_REQUEST="$request_path" \
    node sidecars/codex-runner-ts/scripts/codex-app-server-live-proof.mjs \
    >"$proof_dir/live-proof.log" 2>&1; then
    record_proof codex_app_server passed "$gate" true \
      "live Codex App Server /verify and /run-task proof passed" \
      "capture or update sanitized replay fixtures when the protocol shape changes" \
      true
  else
    record_proof codex_app_server failed "$gate" true \
      "live Codex App Server proof failed; see $proof_dir/live-proof.log" \
      "inspect verify.json, run-task-result.json, App Server auth, and isolated workspace configuration" \
      true
  fi
}

check_kubernetes_executor() {
  gate=COAT_KUBERNETES_EXECUTOR_LIVE_PROOF
  gate_raw=$(env_value "$gate")
  if ! env_enabled "$gate_raw"; then
    record_proof kubernetes_executor_kind_k3d skipped "$gate" false \
      "explicit kind/k3d executor live proof gate is not enabled" \
      "set COAT_KUBERNETES_EXECUTOR_LIVE_PROOF=1 only with kubectl, kind or k3d, and coordinator evidence refs"
    return
  fi

  mode=${COAT_KUBERNETES_EXECUTOR_PROOF_MODE:-server_dry_run}
  case "$mode" in
    server_dry_run|apply) ;;
    *)
      record_proof kubernetes_executor_kind_k3d failed "$gate" true \
        "COAT_KUBERNETES_EXECUTOR_PROOF_MODE must be server_dry_run or apply, got $mode" \
        "use server_dry_run for the first kind/k3d proof stage"
      return
      ;;
  esac

  if [ "${SANDBOX_ENABLE_KUBERNETES_PROVISIONER:-false}" != "true" ]; then
    record_proof kubernetes_executor_kind_k3d failed "$gate" true \
      "SANDBOX_ENABLE_KUBERNETES_PROVISIONER must be true before live cluster contact" \
      "enable the sandbox-runner Kubernetes provisioner only for the proof environment"
    return
  fi

  if ! command_exists kubectl; then
    record_proof kubernetes_executor_kind_k3d failed "$gate" true \
      "kubectl is unavailable" \
      "install kubectl and select the intended kind or k3d context"
    return
  fi

  if ! command_exists kind && ! command_exists k3d; then
    record_proof kubernetes_executor_kind_k3d failed "$gate" true \
      "neither kind nor k3d is available" \
      "install kind or k3d before running the first Kubernetes executor proof"
    return
  fi

  capacity_ref=${COAT_KUBERNETES_CAPACITY_DECISION_REF:-}
  template_ref=${COAT_KUBERNETES_TEMPLATE_REF:-}
  ingestion_ref=${COAT_KUBERNETES_RESULT_INGESTION_REF:-}

  if [ -z "$capacity_ref" ] || [ -z "$template_ref" ] || [ -z "$ingestion_ref" ]; then
    record_proof kubernetes_executor_kind_k3d failed "$gate" true \
      "coordinator evidence refs are incomplete for live executor provisioning" \
      "set COAT_KUBERNETES_CAPACITY_DECISION_REF, COAT_KUBERNETES_TEMPLATE_REF, and COAT_KUBERNETES_RESULT_INGESTION_REF"
    return
  fi

  record_proof kubernetes_executor_kind_k3d ready "$gate" true \
    "static gates are ready for $mode; this scaffold did not contact the Kubernetes API" \
    "send a coordinator-approved sandbox-runner provision request, then watch Job/Pod results and ingest attestation evidence"
}

log "summary=$summary_path"
check_restate_restart_resume
check_codex_app_server
check_kubernetes_executor
write_summary
log "complete; summary: $summary_path"

if [ "$overall_status" = "failed" ]; then
  exit 1
fi
