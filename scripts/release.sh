#!/bin/bash
VERSION=$1
SUFFIX=$2

if [ -z "$VERSION" ]; then
    echo "Usage: $0 <version> <suffix>"
    exit 1
fi

if ! [ -z "$SUFFIX" ]; then
    SUFFIX_ARG="--tag-suffix $SUFFIX"
    SUFFIX_STRING=".$SUFFIX"
else
    SUFFIX_ARG=""
    SUFFIX_STRING=""
fi

echo "Releasing version $VERSION$SUFFIX_STRING"

cargo run -p coat-cli -- release cut --version $VERSION --chart-version $VERSION --allow-dirty --push $SUFFIX_ARG
