#!/usr/bin/env bash
set -euo pipefail

VERSION="${VERSION:-dev}"
VERSION="${VERSION#v}"
TARGET="${TARGET:-}"
DIST_DIR="${DIST_DIR:-dist}"
ASSET_SUFFIX="${ASSET_SUFFIX:-${TARGET:-native}}"
ARCHIVE_NAME="jattg-binaries-${VERSION}-${ASSET_SUFFIX}"

BINARIES=(
  coat
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

if [[ -n "${TARGET}" ]]; then
  TARGET_RELEASE_DIR="target/${TARGET}/release"
else
  TARGET_RELEASE_DIR="target/release"
fi

STAGING_DIR="${DIST_DIR}/${ARCHIVE_NAME}"
rm -rf "${STAGING_DIR}"
mkdir -p "${STAGING_DIR}/bin"

for binary in "${BINARIES[@]}"; do
  source_path="${TARGET_RELEASE_DIR}/${binary}"
  if [[ ! -x "${source_path}" ]]; then
    echo "missing built binary: ${source_path}" >&2
    exit 1
  fi
  cp "${source_path}" "${STAGING_DIR}/bin/${binary}"
done

cp README.md "${STAGING_DIR}/README.md"

cat > "${STAGING_DIR}/manifest.json" <<JSON
{
  "name": "jattg-binaries",
  "version": "${VERSION}",
  "target": "${TARGET:-native}",
  "binaries": [
    "coat",
    "coat-coordinator",
    "coat-event-gateway",
    "coat-goal-store",
    "coat-memory-gateway",
    "coat-notifier",
    "coat-runner-registry",
    "coat-sandbox-runner",
    "coat-tool-registry",
    "coat-validator"
  ]
}
JSON

mkdir -p "${DIST_DIR}"
tar -C "${DIST_DIR}" -czf "${DIST_DIR}/${ARCHIVE_NAME}.tar.gz" "${ARCHIVE_NAME}"
shasum -a 256 "${DIST_DIR}/${ARCHIVE_NAME}.tar.gz" > "${DIST_DIR}/${ARCHIVE_NAME}.tar.gz.sha256"

echo "${DIST_DIR}/${ARCHIVE_NAME}.tar.gz"
