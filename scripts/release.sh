#!/bin/bash
set -euo pipefail

VERSION=${1:-}
SUFFIX=${2:-}

if [ -z "$VERSION" ]; then
    echo "Usage: $0 <version> [suffix]"
    exit 1
fi

ARGS=(release cut --version "$VERSION" --chart-version "$VERSION" --allow-dirty --push)

if [ -n "$SUFFIX" ]; then
    ARGS+=(--tag-suffix "$SUFFIX")
    SUFFIX_STRING=".$SUFFIX"
else
    SUFFIX_STRING=""
fi

echo "Releasing version $VERSION$SUFFIX_STRING"

coat "${ARGS[@]}"
