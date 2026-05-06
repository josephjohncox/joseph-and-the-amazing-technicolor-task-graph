#!/usr/bin/env sh
set -eu

cargo run -p coat-domain --bin generate-schemas -- "${1:-schemas}"
