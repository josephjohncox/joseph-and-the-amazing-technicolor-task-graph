#!/usr/bin/env bash
set -euo pipefail

VERSION="${VERSION:-}"
VERSION="${VERSION#v}"
IMAGE_REGISTRY="${IMAGE_REGISTRY:-ghcr.io}"
IMAGE_NAMESPACE="${IMAGE_NAMESPACE:-josephjohncox/joseph-and-the-amazing-technicolor-task-graph}"
PLATFORMS="${PLATFORMS:-linux/amd64,linux/arm64}"
CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}"
BUILDX_PROGRESS="${BUILDX_PROGRESS:-plain}"
BUILDX_CACHE="${BUILDX_CACHE:-auto}"
BUILDX_REGISTRY_CACHE="${BUILDX_REGISTRY_CACHE:-auto}"
PUSH_IMAGES="${PUSH_IMAGES:-true}"
IMAGE_FILTERS=("$@")

if [[ -z "${VERSION}" ]]; then
  echo "VERSION is required" >&2
  exit 2
fi

IMAGE_NAMESPACE="$(printf '%s' "${IMAGE_NAMESPACE}" | tr '[:upper:]' '[:lower:]')"

output_args=()
if [[ "${PUSH_IMAGES}" == "true" ]]; then
  output_args=(--push)
else
  output_args=(--load)
  PLATFORMS="${PLATFORMS%%,*}"
fi

build_image() {
  local image_name="$1"
  local cache_scope="$2"
  shift 2
  local image_ref="${IMAGE_REGISTRY}/${IMAGE_NAMESPACE}/${image_name}"
  local registry_cache_ref="${IMAGE_REGISTRY}/${IMAGE_NAMESPACE}/jattg-build-cache:${cache_scope}"
  local cache_args=()

  if [[ "${BUILDX_CACHE}" == "true" || ( "${BUILDX_CACHE}" == "auto" && "${GITHUB_ACTIONS:-}" == "true" ) ]]; then
    cache_args=(
      --cache-from "type=gha,scope=${cache_scope}"
      --cache-to "type=gha,mode=max,scope=${cache_scope},ignore-error=true"
    )
  fi

  if [[ "${PUSH_IMAGES}" == "true" ]] \
    && [[ "${BUILDX_REGISTRY_CACHE}" == "true" || ( "${BUILDX_REGISTRY_CACHE}" == "auto" && "${GITHUB_ACTIONS:-}" == "true" ) ]]; then
    cache_args+=(
      --cache-from "type=registry,ref=${registry_cache_ref}"
      --cache-to "type=registry,ref=${registry_cache_ref},mode=max,image-manifest=true,oci-mediatypes=true,ignore-error=true"
    )
  fi

  docker buildx build \
    --progress="${BUILDX_PROGRESS}" \
    --platform="${PLATFORMS}" \
    "${output_args[@]}" \
    "${cache_args[@]}" \
    --tag "${image_ref}:v${VERSION}" \
    --tag "${image_ref}:${VERSION}" \
    --tag "${image_ref}:latest" \
    "$@" \
    .
}

should_build_image() {
  local image_name="$1"
  local group_name="$2"

  if [[ "${#IMAGE_FILTERS[@]}" -eq 0 ]]; then
    return 0
  fi

  local filter
  for filter in "${IMAGE_FILTERS[@]}"; do
    case "${filter}" in
      all)
        return 0
        ;;
      rust)
        if [[ "${group_name}" == "rust-service" || "${group_name}" == "agent-toolbox" ]]; then
          return 0
        fi
        ;;
      rust-services)
        if [[ "${group_name}" == "rust-service" ]]; then
          return 0
        fi
        ;;
      agent-toolbox|node-sidecars)
        if [[ "${group_name}" == "${filter}" ]]; then
          return 0
        fi
        ;;
      *)
        if [[ "${image_name}" == "${filter}" ]]; then
          return 0
        fi
        ;;
    esac
  done

  return 1
}

rust_images=(
  jattg-coordinator=coat-coordinator
  jattg-event-gateway=coat-event-gateway
  jattg-goal-store=coat-goal-store
  jattg-memory-gateway=coat-memory-gateway
  jattg-notifier=coat-notifier
  jattg-runner-registry=coat-runner-registry
  jattg-sandbox-runner=coat-sandbox-runner
  jattg-tool-registry=coat-tool-registry
  jattg-validator=coat-validator
)

for spec in "${rust_images[@]}"; do
  image_name="${spec%%=*}"
  bin="${spec#*=}"
  if should_build_image "${image_name}" rust-service; then
    build_image "${image_name}" rust-services \
      -f infra/containers/rust-service.Dockerfile \
      --target service \
      --build-arg "BIN=${bin}" \
      --build-arg "CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS}"
  fi
done

if should_build_image "jattg-agent-toolbox" agent-toolbox; then
  build_image "jattg-agent-toolbox" agent-toolbox \
    -f infra/containers/rust-service.Dockerfile \
    --target agent-toolbox \
    --build-arg "CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS}"
fi

sidecars=(
  jattg-control-web=ui/control-plane-web
  jattg-codex-runner=sidecars/codex-runner-ts
  jattg-claude-code-runner=sidecars/claude-code-runner-ts
  jattg-model-provider-runner=sidecars/model-provider-runner-ts
  jattg-staff-engineer-runner=sidecars/staff-engineer-runner-ts
)

for spec in "${sidecars[@]}"; do
  image_name="${spec%%=*}"
  sidecar_dir="${spec#*=}"
  if should_build_image "${image_name}" node-sidecars; then
    build_image "${image_name}" "${image_name}" \
      -f infra/containers/node-sidecar.Dockerfile \
      --build-arg "SIDECAR_DIR=${sidecar_dir}"
  fi
done
