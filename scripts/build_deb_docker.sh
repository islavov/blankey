#!/bin/sh
# Builds the .deb inside a container of the target release, for local use; CI runs build_deb.sh directly.
#   scripts/build_deb_docker.sh debian:trixie
#   scripts/build_deb_docker.sh ubuntu:26.04
# PLATFORM=linux/amd64 by default (arm64 hosts emulate it, which is slow).
set -eu
IMAGE="${1:?usage: build_deb_docker.sh <image, e.g. debian:trixie or ubuntu:26.04>}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
docker run --rm --platform "${PLATFORM:-linux/amd64}" \
    -v "$ROOT":/src -w /src \
    -v blankey-cargo-$(echo "$IMAGE" | tr ':/' '--'):/root/.cargo \
    -e CARGO_TARGET_DIR=/src/target-linux \
    "$IMAGE" sh -euc '
        apt-get update && apt-get install -y --no-install-recommends curl ca-certificates build-essential
        [ -x /root/.cargo/bin/cargo ] || curl --proto =https -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
        export PATH=/root/.cargo/bin:$PATH
        command -v cargo-tauri >/dev/null || cargo install tauri-cli --version "^2" --locked
        command -v cargo-about >/dev/null || cargo install cargo-about --locked
        scripts/build_deb.sh --install-deps
    '
