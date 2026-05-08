#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source_logo="${repo_root}/assets/coat-logo.png"
source_mark="${repo_root}/assets/coat-mark.svg"
root_icon_dir="${repo_root}/assets/icons"
web_brand_dir="${repo_root}/ui/control-plane-web/public/brand"

if [[ ! -f "${source_logo}" ]]; then
  echo "missing source logo: ${source_logo}" >&2
  exit 1
fi

if [[ ! -f "${source_mark}" ]]; then
  echo "missing source mark: ${source_mark}" >&2
  exit 1
fi

if ! command -v sips >/dev/null 2>&1; then
  echo "sips is required to generate PNG icon sizes on macOS" >&2
  exit 1
fi

mkdir -p "${root_icon_dir}" "${web_brand_dir}"
cp "${source_logo}" "${web_brand_dir}/coat-logo.png"
cp "${source_mark}" "${web_brand_dir}/coat-mark.svg"

for size in 16 32 48 64 128 180 192 512; do
  sips -s format png -z "${size}" "${size}" "${source_logo}" \
    --out "${root_icon_dir}/coat-icon-${size}.png" >/dev/null
  cp "${root_icon_dir}/coat-icon-${size}.png" "${web_brand_dir}/coat-icon-${size}.png"
done

echo "generated COAT brand assets in assets/icons and ui/control-plane-web/public/brand"
