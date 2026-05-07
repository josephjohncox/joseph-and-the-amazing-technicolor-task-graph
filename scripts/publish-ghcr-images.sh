#!/usr/bin/env bash
set -euo pipefail

VERSION="${VERSION:-}"
VERSION="${VERSION#v}"
IMAGE_REGISTRY="${IMAGE_REGISTRY:-ghcr.io}"
IMAGE_NAMESPACE="${IMAGE_NAMESPACE:-}"
PLATFORMS="${PLATFORMS:-linux/amd64,linux/arm64}"
CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}"
BUILDX_PROGRESS="${BUILDX_PROGRESS:-plain}"
PUSH_IMAGES="${PUSH_IMAGES:-true}"

if [[ -z "${VERSION}" ]]; then
  echo "VERSION is required" >&2
  exit 2
fi

if [[ -z "${IMAGE_NAMESPACE}" ]]; then
  if [[ -z "${GITHUB_REPOSITORY:-}" ]]; then
    echo "IMAGE_NAMESPACE or GITHUB_REPOSITORY is required" >&2
    exit 2
  fi
  IMAGE_NAMESPACE="${GITHUB_REPOSITORY}"
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
  shift
  local image_ref="${IMAGE_REGISTRY}/${IMAGE_NAMESPACE}/${image_name}"

  docker buildx build \
    --progress="${BUILDX_PROGRESS}" \
    --platform="${PLATFORMS}" \
    "${output_args[@]}" \
    --tag "${image_ref}:v${VERSION}" \
    --tag "${image_ref}:${VERSION}" \
    --tag "${image_ref}:latest" \
    "$@" \
    .
}

rust_bins=(
  coat-coordinator
  coat-event-gateway
  coat-goal-store
  coat-memory-gateway
  coat-notifier
  coat-runner-registry
  coat-sandbox-runner
  coat-tool-registry
  coat-validator
)

for bin in "${rust_bins[@]}"; do
  build_image "${bin}" \
    -f infra/containers/rust-service.Dockerfile \
    --build-arg "BIN=${bin}" \
    --build-arg "CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS}"
done

sidecars=(
  coat-control-web=ui/control-plane-web
  coat-codex-runner=sidecars/codex-runner-ts
  coat-staff-engineer-runner=sidecars/staff-engineer-runner-ts
)

for spec in "${sidecars[@]}"; do
  image_name="${spec%%=*}"
  sidecar_dir="${spec#*=}"
  build_image "${image_name}" \
    -f infra/containers/node-sidecar.Dockerfile \
    --build-arg "SIDECAR_DIR=${sidecar_dir}"
done
