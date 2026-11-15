#!/bin/sh
set -eu
base=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
output=${1:-"$base/dist/spoti-athena-source.tar.gz"}
mkdir -p -- "$(dirname -- "$output")"
tar -czf "$output" -C "$base" \
    athena/Cargo.toml athena/Cargo.lock athena/README.md athena/src \
    athena/pack-source.sh athena/systemd \
    control/Cargo.toml control/Cargo.lock control/README.md control/src
printf '%s\n' "$output"
