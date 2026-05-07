#!/usr/bin/env bash
set -euo pipefail

kind="${COAT_EPHEMERAL_KIND:-command}"
injection_dir="${COAT_INJECTION_DIR:-/opt/coat/injections}"

if [[ -d "${injection_dir}/env" ]]; then
  while IFS= read -r env_file; do
    set -a
    # shellcheck disable=SC1090
    . "${env_file}"
    set +a
  done < <(find "${injection_dir}/env" -maxdepth 1 -type f -name '*.env' | sort)
fi

if [[ -d "${injection_dir}/bin" ]]; then
  export PATH="${injection_dir}/bin:${PATH}"
fi

if [[ "${COAT_ENABLE_INJECTION_SCRIPTS:-false}" == "true" && -d "${injection_dir}/init.d" ]]; then
  while IFS= read -r script; do
    bash "${script}"
  done < <(find "${injection_dir}/init.d" -maxdepth 1 -type f -name '*.sh' | sort)
fi

case "${kind}" in
  codex-runner)
    cd /opt/coat/sidecars/codex-runner-ts
    exec npm start
    ;;
  claude-code-runner)
    cd /opt/coat/sidecars/claude-code-runner-ts
    exec npm start
    ;;
  model-provider-runner)
    cd /opt/coat/sidecars/model-provider-runner-ts
    exec npm start
    ;;
  staff-engineer-runner)
    cd /opt/coat/sidecars/staff-engineer-runner-ts
    exec npm start
    ;;
  coordinator)
    exec coat-coordinator "$@"
    ;;
  event-gateway)
    exec coat-event-gateway "$@"
    ;;
  goal-store)
    exec coat-goal-store "$@"
    ;;
  memory-gateway)
    exec coat-memory-gateway "$@"
    ;;
  notifier)
    exec coat-notifier "$@"
    ;;
  runner-registry)
    exec coat-runner-registry "$@"
    ;;
  sandbox-runner)
    exec coat-sandbox-runner "$@"
    ;;
  tool-registry)
    exec coat-tool-registry "$@"
    ;;
  validator)
    exec coat-validator "$@"
    ;;
  command)
    exec "$@"
    ;;
  *)
    echo "unknown COAT_EPHEMERAL_KIND=${kind}" >&2
    exit 2
    ;;
esac
