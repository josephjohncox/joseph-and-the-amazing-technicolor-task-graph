#!/usr/bin/env sh
set -eu

output="infra/compose/local-providers.env"
write_env=false
check=false
print_commands=false

while [ "$#" -gt 0 ]; do
  case "$1" in
    --output)
      shift
      output="${1:?missing value for --output}"
      ;;
    --write-env)
      write_env=true
      ;;
    --check)
      check=true
      ;;
    --print-commands)
      print_commands=true
      ;;
    --help|-h)
      printf 'usage: %s [--write-env] [--check] [--print-commands] [--output PATH]\n' "$0"
      printf 'delegates to the interactive `coat setup local-auth` wizard when the installed CLI is available.\n'
      printf 'without `coat` on PATH, no-flag fallback prints checks and suggested commands only.\n'
      exit 0
      ;;
    *)
      printf 'unknown argument: %s\n' "$1" >&2
      exit 2
      ;;
  esac
  shift
done

if command -v coat >/dev/null 2>&1; then
  set -- setup local-auth --output "$output"
  if [ "$write_env" = true ]; then set -- "$@" --write-env; fi
  if [ "$check" = true ]; then set -- "$@" --check; fi
  if [ "$print_commands" = true ]; then set -- "$@" --print-commands; fi
  exec coat "$@"
fi

if [ "$write_env" = false ] && [ "$check" = false ] && [ "$print_commands" = false ]; then
  check=true
  print_commands=true
fi

if [ "$check" = true ]; then
  printf 'local provider auth check\n'
  for tool in coat docker node npm codex claude aws ollama vllm hf; do
    if command -v "$tool" >/dev/null 2>&1; then
      printf '  %-8s ok\n' "$tool"
    else
      printf '  %-8s missing\n' "$tool"
    fi
  done
  for name in OPENAI_API_KEY ANTHROPIC_API_KEY ANTHROPIC_AUTH_TOKEN CLAUDE_CODE_OAUTH_TOKEN AWS_PROFILE AWS_REGION AWS_DEFAULT_REGION MODEL_PROVIDER_ENDPOINT LOCAL_MODEL_PROVIDER_ENDPOINT COAT_CONTROL_CHAT_MODEL MEMORY_GATEWAY_EMBEDDING_TOKEN; do
    eval "value=\${$name:-}"
    if [ -n "$value" ]; then
      printf '  %-34s set\n' "$name"
    else
      printf '  %-34s unset\n' "$name"
    fi
  done
  printf 'secret values are intentionally not printed\n'
fi

if [ "$print_commands" = true ]; then
  printf 'suggested local auth/setup commands:\n'
  printf '  codex login\n'
  printf '  claude login\n'
  printf '  aws sso login --profile <profile>\n'
  printf '  ollama pull llama3.1\n'
  printf '  vllm serve <model> --host 0.0.0.0 --port 8000\n'
  printf '  hf auth login\n'
  printf 'run `coat setup local-auth` interactively to flip selected runner lanes live\n'
  printf 'then preflight Compose with:\n'
  printf '  coat deploy local preflight --env-file %s\n' "$output"
fi

if [ "$write_env" = true ]; then
  mkdir -p "$(dirname "$output")"
  cp infra/compose/local-providers.env.example "$output"
  printf 'wrote %s\n' "$output"
fi
