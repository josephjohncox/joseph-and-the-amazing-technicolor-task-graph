#!/usr/bin/env sh
set -eu

cargo run -p jattg-domain --bin generate-schemas -- "${1:-schemas}"
