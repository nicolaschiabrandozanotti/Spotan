#!/bin/sh
# Build Spoti for Linux (Wayland window + terminal) and Windows (terminal) into ../dist.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(dirname "$here")
docker build -q -t spoti-builder "$here" >/dev/null
docker run --rm -v "$root:/src" -v spoti-cargo:/usr/local/cargo/registry -v spoti-target:/target \
  -e CARGO_TARGET_DIR=/target -e HOST_UID="$(id -u)" -e HOST_GID="$(id -g)" \
  -w /src/ncspot spoti-builder sh /src/packaging/inside.sh
