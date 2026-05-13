#!/bin/sh
set -eu

fail() {
  printf 'scenario e2e failed: %s\n' "$*" >&2
  exit 1
}

log() {
  printf '[scenario-e2e] %s\n' "$*"
}

script_dir=$(CDPATH= cd "$(dirname "$0")" && pwd)
repo_root=$(CDPATH= cd "$script_dir/.." && pwd)
cd "$repo_root"

PATH="/opt/homebrew/bin:/usr/local/bin:$PATH"
export PATH

coat=${COAT:-}
if [ -z "$coat" ] || { [ "$coat" = "coat" ] && ! command -v coat >/dev/null 2>&1; }; then
  if [ -x target/debug/coat ]; then
    coat=target/debug/coat
  else
    coat=coat
  fi
fi

out_root=${COAT_SCENARIO_E2E_OUT:-target/coat-scenarios}
run_dir="$out_root/latest"
stack_dir="$run_dir/stack"
spec_root="$run_dir/specs"
stack_mode=${COAT_SCENARIO_E2E_STACK:-auto}
stack_only=${COAT_SCENARIO_E2E_STACK_ONLY:-0}
keep_stack=${COAT_SCENARIO_E2E_KEEP_STACK:-1}
services=${COAT_SCENARIO_E2E_SERVICES:-}
dry_run=${COAT_SCENARIO_E2E_DRY_RUN:-0}
scenario_args=${COAT_SCENARIO_E2E_ARGS:-}
scenario_gateway_url=${COAT_SCENARIO_E2E_GATEWAY_URL:-}
if [ -z "$scenario_gateway_url" ]; then
  case "$stack_mode" in
    never)
      scenario_gateway_url=http://127.0.0.1:0
      ;;
    *)
      scenario_gateway_url=http://127.0.0.1:9090
      ;;
  esac
fi
profile=${COAT_SCENARIO_E2E_PROFILE:-${COAT_PROFILE:-local}}
stub_env="$stack_dir/stub-local-providers.env"
started_stack=0

case "$stack_mode" in
  auto|always|never|preflight) ;;
  *) fail "COAT_SCENARIO_E2E_STACK must be auto, always, never, or preflight" ;;
esac

case "$run_dir" in
  ""|"/"|".") fail "refusing unsafe evidence directory: $run_dir" ;;
esac
rm -rf "$stack_dir" "$spec_root" "$run_dir/coat-scenario-help"
rm -f "$run_dir/run.env"
mkdir -p "$stack_dir" "$spec_root"

export COAT_PROFILE="$profile"

force_stub_environment() {
  export CODEX_RUNNER_MODE=stub
  export CODEX_REVIEW_RUNNER_MODE=stub
  export CLAUDE_CODE_RUNNER_MODE=stub
  export STAFF_ENGINEER_RUNNER_MODE=stub
  export MODEL_PROVIDER_RUNNER_MODE=stub
  export MODEL_PROVIDER_RESEARCH_RUNNER_MODE=stub
  export MODEL_PROVIDER_LOCAL_RUNNER_MODE=stub
  export CODEX_VERIFY_MCP=0
  export CODEX_VERIFY_APP_SERVER=0
  export CLAUDE_CODE_VERIFY_CLI=0
  export MODEL_PROVIDER_VERIFY_ENDPOINT=0
  export COAT_WEB_SEARCH_ENABLED=false
  export CODEX_NATIVE_WEB_SEARCH=false
  export CLAUDE_CODE_NATIVE_WEB_SEARCH=false
  export MODEL_PROVIDER_WEB_SEARCH_ENABLED=false
  export MODEL_PROVIDER_RESEARCH_WEB_SEARCH_ENABLED=false
  export COAT_CONTROL_CHAT_BACKEND=stub
  export OPENAI_API_KEY=
  export CODEX_API_KEY=
  export ANTHROPIC_API_KEY=
  export ANTHROPIC_AUTH_TOKEN=
  export CLAUDE_CODE_OAUTH_TOKEN=
  export COAT_LLM_GATEWAY_API_KEY=
  export MODEL_PROVIDER_API_KEY=
  export MODEL_PROVIDER_RESEARCH_API_KEY=
  export COAT_CONTROL_CHAT_API_KEY=
  export MEMORY_GATEWAY_GRAPHITI_TOKEN=
  export MEMORY_GATEWAY_QDRANT_TOKEN=
  export MEMORY_GATEWAY_EMBEDDING_TOKEN=
}

write_stub_env() {
  cat >"$stub_env" <<'EOF'
COAT_LOG_LEVEL=debug
COAT_NODE_LOG_LEVEL=debug
COAT_LOG_FORMAT=compact
COAT_LOG_ANSI=false
COAT_RUST_LOG=info,tower_http=info,restate_sdk=info,coat_coordinator=debug,coat_validator=debug,coat_sandbox_runner=debug,coat_tool_registry=debug,coat_runner_registry=debug,coat_notifier=debug,coat_goal_store=debug,coat_event_gateway=debug,coat_memory_gateway=debug
COAT_RESTATE_RUST_LOG=info
CODEX_RUNNER_MODE=stub
CODEX_REVIEW_RUNNER_MODE=stub
CLAUDE_CODE_RUNNER_MODE=stub
STAFF_ENGINEER_RUNNER_MODE=stub
MODEL_PROVIDER_RUNNER_MODE=stub
MODEL_PROVIDER_RESEARCH_RUNNER_MODE=stub
MODEL_PROVIDER_LOCAL_RUNNER_MODE=stub
CODEX_VERIFY_MCP=0
CODEX_VERIFY_APP_SERVER=0
CLAUDE_CODE_VERIFY_CLI=0
MODEL_PROVIDER_VERIFY_ENDPOINT=0
SANDBOX_ENABLE_LOCAL_COMMAND_EXECUTION=false
SANDBOX_REQUIRE_COMMAND_APPROVAL=true
SANDBOX_ALLOWED_LOCAL_BINARIES=git,make,cargo,npm,pnpm,yarn,node,python3,python,pytest,go,buf,docker,helm,kubectl
SANDBOX_COMMAND_TIMEOUT_SECONDS=600
SANDBOX_COMMAND_MAX_OUTPUT_BYTES=65536
CODEX_AUTH_MODE=env_api_key
CODEX_APP_SERVER_URL=
CODEX_AUTH_STATE_PATH=
CODEX_RUNNER_LABELS_JSON={"pool":"default","runtime":"codex","auth.codex.device":"false","auth.codex.api_key":"false","auth.mode":"stub"}
CODEX_REVIEW_RUNNER_LABELS_JSON={"pool":"default","runtime":"codex","lane":"review","auth.codex.device":"false","auth.codex.api_key":"false","auth.mode":"stub"}
CLAUDE_CODE_AUTH_MODE=env_api_key
STAFF_ENGINEER_AUTH_MODE=env_api_key
CLAUDE_CODE_AUTH_STATE_PATH=
CLAUDE_CODE_RUNNER_LABELS_JSON={"pool":"default","runtime":"claude-code","auth.claude.device":"false","auth.claude.api_key":"false","auth.mode":"stub"}
STAFF_ENGINEER_RUNNER_LABELS_JSON={"pool":"default","runtime":"staff-engineer","auth.claude.device":"false","auth.claude.api_key":"false","auth.mode":"stub"}
OPENAI_API_KEY=
CODEX_API_KEY=
ANTHROPIC_API_KEY=
ANTHROPIC_AUTH_TOKEN=
CLAUDE_CODE_OAUTH_TOKEN=
COAT_LLM_GATEWAY_PROVIDER=
COAT_LLM_GATEWAY_URL=
COAT_LLM_GATEWAY_CHAT_COMPLETIONS_URL=
COAT_LLM_GATEWAY_API_KEY=
COAT_LLM_GATEWAY_AUTH_MODE=api_key_or_none
COAT_LLM_GATEWAY_DEFAULT_MODEL=
COAT_LLM_GATEWAY_WORK_MODEL=
COAT_LLM_GATEWAY_RESEARCH_MODEL=
COAT_LLM_GATEWAY_CHAT_MODEL=
COAT_WEB_SEARCH_ENABLED=false
COAT_WEB_SEARCH_ROUTE=coordinator_task
COAT_WEB_SEARCH_PROVIDER=agent_native
COAT_WEB_SEARCH_URL=
COAT_WEB_SEARCH_AUTH_MODE=api_key_or_none
COAT_WEB_SEARCH_API_KEY=
CODEX_NATIVE_WEB_SEARCH=false
CLAUDE_CODE_NATIVE_WEB_SEARCH=false
MODEL_PROVIDER_WEB_SEARCH_ENABLED=false
MODEL_PROVIDER_RESEARCH_WEB_SEARCH_ENABLED=false
MODEL_PROVIDER_KIND=open_ai_compatible
MODEL_PROVIDER_AUTH_MODE=api_key_or_none
MODEL_PROVIDER_MODEL=
MODEL_PROVIDER_ENDPOINT=
MODEL_PROVIDER_API_KEY=
MODEL_PROVIDER_LATENCY_CLASS=
MODEL_PROVIDER_SPEED_TIER=
MODEL_PROVIDER_TEMPERATURE=
MODEL_PROVIDER_TOP_P=
MODEL_PROVIDER_MAX_OUTPUT_TOKENS=
MODEL_PROVIDER_REASONING_EFFORT=
MODEL_PROVIDER_TIMEOUT_SECONDS=
MODEL_PROVIDER_RESEARCH_KIND=open_ai_compatible
MODEL_PROVIDER_RESEARCH_AUTH_MODE=api_key_or_none
MODEL_PROVIDER_RESEARCH_MODEL=
MODEL_PROVIDER_RESEARCH_ENDPOINT=
MODEL_PROVIDER_RESEARCH_API_KEY=
MODEL_PROVIDER_RESEARCH_LATENCY_CLASS=
MODEL_PROVIDER_RESEARCH_SPEED_TIER=
MODEL_PROVIDER_RESEARCH_TEMPERATURE=
MODEL_PROVIDER_RESEARCH_TOP_P=
MODEL_PROVIDER_RESEARCH_MAX_OUTPUT_TOKENS=
MODEL_PROVIDER_RESEARCH_REASONING_EFFORT=
MODEL_PROVIDER_RESEARCH_TIMEOUT_SECONDS=
LOCAL_MODEL_PROVIDER_KIND=ollama
LOCAL_MODEL_PROVIDER_AUTH_MODE=none
LOCAL_MODEL_PROVIDER_MODEL=
LOCAL_MODEL_PROVIDER_ENDPOINT=http://host.docker.internal:11434/v1
LOCAL_MODEL_PROVIDER_LATENCY_CLASS=fast
LOCAL_MODEL_PROVIDER_SPEED_TIER=
LOCAL_MODEL_PROVIDER_TEMPERATURE=0.2
LOCAL_MODEL_PROVIDER_TOP_P=0.9
LOCAL_MODEL_PROVIDER_MAX_OUTPUT_TOKENS=2048
LOCAL_MODEL_PROVIDER_REASONING_EFFORT=low
LOCAL_MODEL_PROVIDER_TIMEOUT_SECONDS=60
COAT_CONTROL_CHAT_BACKEND=stub
COAT_CONTROL_CHAT_PROVIDER=
COAT_CONTROL_CHAT_COMPLETIONS_URL=
COAT_CONTROL_CHAT_MODEL=
COAT_CONTROL_CHAT_API_KEY=
MEMORY_GATEWAY_GRAPHITI_MCP_URL=
MEMORY_GATEWAY_GRAPHITI_GROUP_ID=jattg
MEMORY_GATEWAY_GRAPHITI_TOKEN=
MEMORY_GATEWAY_QDRANT_URL=
MEMORY_GATEWAY_QDRANT_COLLECTION=jattg_memory
MEMORY_GATEWAY_QDRANT_TOKEN=
MEMORY_GATEWAY_EMBEDDING_URL=
MEMORY_GATEWAY_EMBEDDING_MODEL=
MEMORY_GATEWAY_EMBEDDING_DIMENSIONS=
MEMORY_GATEWAY_EMBEDDING_TOKEN=
MEMORY_GATEWAY_EMBEDDING_SEND_DIMENSIONS=false
EOF
}

run_step() {
  step_dir=$1
  shift
  mkdir -p "$step_dir"
  printf '%s\n' "$*" >"$step_dir/command.txt"
  set +e
  "$@" >"$step_dir/stdout.log" 2>"$step_dir/stderr.log"
  status=$?
  set -e
  printf '%s\n' "$status" >"$step_dir/status.txt"
  return "$status"
}

health_targets='runner-registry http://127.0.0.1:9085/healthz goal-store http://127.0.0.1:9088/healthz event-gateway http://127.0.0.1:9089/healthz memory-gateway http://127.0.0.1:9087/healthz notifier http://127.0.0.1:9086/healthz tool-registry http://127.0.0.1:9084/healthz sandbox-runner http://127.0.0.1:9083/healthz validator http://127.0.0.1:9082/healthz control-web http://127.0.0.1:9090/healthz codex-runner http://127.0.0.1:9091/healthz staff-engineer-runner http://127.0.0.1:9092/healthz model-provider-runner http://127.0.0.1:9093/healthz claude-code-runner http://127.0.0.1:9094/healthz'

check_stack_health_once() {
  command -v curl >/dev/null 2>&1 || return 1
  set -- $health_targets
  while [ "$#" -gt 0 ]; do
    name=$1
    url=$2
    shift 2
    if ! curl -fsS "$url" >"$stack_dir/health-$name.out" 2>"$stack_dir/health-$name.err"; then
      return 1
    fi
  done
  return 0
}

wait_stack_health() {
  attempts=${COAT_SCENARIO_E2E_STACK_ATTEMPTS:-120}
  attempt=1
  while [ "$attempt" -le "$attempts" ]; do
    if check_stack_health_once; then
      return 0
    fi
    sleep 2
    attempt=$((attempt + 1))
  done
  return 1
}

docker_possible() {
  command -v docker >/dev/null 2>&1 || return 1
  docker ps >/dev/null 2>&1 || return 1
  docker compose version >/dev/null 2>&1 || return 1
  return 0
}

prepare_stack() {
  write_stub_env

  if [ "$stack_mode" = "never" ]; then
    log "skipping local stack preparation because COAT_SCENARIO_E2E_STACK=never"
    return 0
  fi

  if [ "$stack_mode" = "auto" ] && check_stack_health_once; then
    log "reusing already healthy local stack"
    return 0
  fi

  if ! docker_possible; then
    if [ "$stack_mode" = "auto" ]; then
      log "skipping local stack boot because Docker Compose is not available"
      return 0
    fi
    fail "Docker Compose is required for COAT_SCENARIO_E2E_STACK=$stack_mode"
  fi

  log "running local Compose preflight with stub runners"
  run_step "$stack_dir/preflight" "$coat" deploy local preflight --env-file "$stub_env" --allow-stub-runners \
    || fail "local Compose preflight failed; see $stack_dir/preflight"

  log "recording resolved local Compose config"
  run_step "$stack_dir/config" "$coat" deploy local config --env-file "$stub_env" \
    || fail "local Compose config failed; see $stack_dir/config"

  if [ "$stack_mode" = "preflight" ]; then
    return 0
  fi

  log "starting local Compose stack with stub runners"
  if [ -n "$services" ]; then
    run_step "$stack_dir/up" "$coat" deploy local up --env-file "$stub_env" --allow-stub-runners --skip-preflight --detach $services \
      || fail "local Compose up failed; see $stack_dir/up"
  else
    run_step "$stack_dir/up" "$coat" deploy local up --env-file "$stub_env" --allow-stub-runners --skip-preflight --detach \
      || fail "local Compose up failed; see $stack_dir/up"
  fi
  started_stack=1

  log "waiting for local stack health endpoints"
  wait_stack_health || fail "local stack did not become healthy; see $stack_dir/health-*.err"
}

cleanup() {
  status=$?
  if [ "$started_stack" = "1" ] && [ "$keep_stack" = "0" ]; then
    run_step "$stack_dir/down" "$coat" deploy local down --env-file "$stub_env" >/dev/null 2>&1 || true
  fi
  exit "$status"
}
trap cleanup EXIT INT TERM

force_stub_environment

{
  printf 'repo_root=%s\n' "$repo_root"
  printf 'coat=%s\n' "$coat"
  printf 'profile=%s\n' "$profile"
  printf 'stack_mode=%s\n' "$stack_mode"
  printf 'services=%s\n' "$services"
  printf 'dry_run=%s\n' "$dry_run"
  printf 'scenario_gateway_url=%s\n' "$scenario_gateway_url"
} >"$run_dir/run.env"

prepare_stack

if [ "$stack_only" = "1" ]; then
  log "stack-only mode complete; evidence: $run_dir"
  exit 0
fi

spec_patterns=${COAT_SCENARIO_E2E_SPECS:-"scripts/coat-scenarios/*.json"}
specs=
for spec in $spec_patterns; do
  if [ -f "$spec" ]; then
    specs="${specs}${specs:+ }$spec"
  fi
done

if [ -z "$specs" ]; then
  fail "no scenario specs matched: $spec_patterns"
fi

if [ "$dry_run" != "1" ]; then
  run_step "$run_dir/coat-scenario-help" "$coat" scenario --help \
    || fail "coat scenario subcommand is unavailable; see $run_dir/coat-scenario-help"
fi

for spec in $specs; do
  spec_name=$(printf '%s' "$(basename "$spec" .json)" | tr -c 'A-Za-z0-9_.-' '_')
  spec_dir="$spec_root/$spec_name"
  mkdir -p "$spec_dir"
  cp "$spec" "$spec_dir/spec.json"
  if [ "$dry_run" = "1" ]; then
    printf 'dry-run: would run %s scenario run --file %s %s\n' "$coat" "$spec" "$scenario_args" >"$spec_dir/stdout.log"
    printf '0\n' >"$spec_dir/status.txt"
    continue
  fi
  log "running scenario spec $spec"
  run_step "$spec_dir/run" "$coat" scenario run --file "$spec" --gateway-url "$scenario_gateway_url" --output-dir "$out_root" $scenario_args \
    || fail "scenario spec failed: $spec; see $spec_dir/run"
done

log "complete; evidence: $run_dir"
