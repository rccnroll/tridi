#!/bin/sh
# Builds the committed HEAD into a .deb for Ubuntu 24.04 and Debian 12 (both
# OpenCASCADE 7.6, which the package depends on: not Debian 13's 7.8):
#   packaging/deb/build.sh        -> packaging/deb/tridi_*.deb
# In a Debian 12 container, so the binary needs glibc 2.36 at most (24.04 has 2.39).
set -eu
repo=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
out=$repo/packaging/deb
src=$(mktemp -d)
trap 'rm -rf "$src"' EXIT
git -C "$repo" archive HEAD | tar -x -C "$src"
podman run --rm -v "$src:/src" -v "$out:/out" -v tridi-cargo:/usr/local/cargo/registry \
  -w /src docker.io/library/rust:1-bookworm sh -ec '
    # OpenCASCADE, for STEP (build.rs)
    apt-get update -qq
    apt-get install -y -qq --no-install-recommends libocct-data-exchange-dev libocct-ocaf-dev \
      libocct-modeling-algorithms-dev libocct-modeling-data-dev libocct-foundation-dev >/dev/null
    cargo install --locked cargo-deb --root /tmp/cd -q
    cargo build --locked --release
    target/release/tridi generate target/assets
    /tmp/cd/bin/cargo-deb --no-build --output /out/'
