#!/usr/bin/env bash
set -euo pipefail

CHART_DIR="${CHART_DIR:-infra/helm/coat}"
DIST_DIR="${DIST_DIR:-dist/helm}"
CHART_VERSION="${CHART_VERSION:-0.1.0}"
RELEASE_URL="${RELEASE_URL:-}"

if [[ -z "${APP_VERSION:-}" ]]; then
  APP_VERSION="$(
    awk -F': *' '$1 == "appVersion" { gsub(/^"|"$/, "", $2); print $2; exit }' \
      "${CHART_DIR}/Chart.yaml"
  )"
fi
APP_VERSION="${APP_VERSION:-${CHART_VERSION}}"

if ! command -v helm >/dev/null 2>&1; then
  echo "helm is required to package the chart" >&2
  exit 127
fi

mkdir -p "${DIST_DIR}"

helm lint "${CHART_DIR}"
helm package "${CHART_DIR}" \
  --version "${CHART_VERSION}" \
  --app-version "${APP_VERSION}" \
  --destination "${DIST_DIR}"

if [[ -n "${RELEASE_URL}" ]]; then
  helm repo index "${DIST_DIR}" --url "${RELEASE_URL}"
else
  helm repo index "${DIST_DIR}"
fi

for artifact in "${DIST_DIR}"/coat-"${CHART_VERSION}".tgz "${DIST_DIR}/index.yaml"; do
  shasum -a 256 "${artifact}" > "${artifact}.sha256"
done

echo "${DIST_DIR}/coat-${CHART_VERSION}.tgz"
