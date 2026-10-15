#!/bin/sh
# Builds the Linux package inside a container of the target release, for local use; CI runs build_deb.sh and
# build_rpm.sh directly.
#   scripts/build_linux_docker.sh debian:trixie     .deb (also ubuntu:26.04)
#   scripts/build_linux_docker.sh fedora:latest     .rpm (also almalinux:10)
# PLATFORM=linux/amd64 by default (arm64 hosts emulate it, which is slow).
set -eu
IMAGE="${1:?usage: build_linux_docker.sh <image, e.g. debian:trixie, ubuntu:26.04, fedora:latest, almalinux:10>}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TAG="$(echo "$IMAGE" | tr ':/' '--')"
docker run --rm --platform "${PLATFORM:-linux/amd64}" \
    -v "$ROOT":/src -w /src \
    -v "blankey-cargo-$TAG":/root/.cargo -v "blankey-rustup-$TAG":/root/.rustup \
    -e CARGO_TARGET_DIR="/src/target-$TAG" \
    "$IMAGE" sh -euc '
        if command -v apt-get >/dev/null; then
            apt-get update && apt-get install -y --no-install-recommends curl ca-certificates build-essential
            build=build_deb.sh
        else
            dnf install -y curl ca-certificates gcc
            build=build_rpm.sh
        fi
        export PATH=/root/.cargo/bin:$PATH
        if ! cargo --version >/dev/null 2>&1; then
            if command -v rustup >/dev/null; then rustup default stable
            else curl --proto =https -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal; fi
        fi
        command -v cargo-tauri >/dev/null || cargo install tauri-cli --version "^2" --locked
        command -v cargo-about >/dev/null || cargo install cargo-about --locked --features cli
        "scripts/$build" --install-deps
    '
