#!/bin/sh
# The Python wheel: the tridi binary, whose STEP helper has OpenCASCADE
# linked in statically, and the Python package around it. Inside
# quay.io/pypa/manylinux_2_28_x86_64, from the repository root:
#   packaging/wheel/build.sh OUT
# OUT gets the wheel and the OpenCASCADE source it's built from (LGPL-2.1:
# the release carries it). The repository is only read.
set -eu

OUT=$(realpath "$1")
PY=/opt/python/cp312-cp312/bin/python

if ! command -v cargo >/dev/null; then
    curl -fsSL https://sh.rustup.rs | sh -s -- -y -q --profile minimal --default-toolchain 1.92
    . "$HOME/.cargo/env"
fi
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/tmp/target}
# the OpenCASCADE built from occt-sys, never the system's: the helper
# then needs no library beyond libc and libstdc++
TRIDI_OCCT=bundled cargo build --release --locked -p tridi-cli

# the package, with _bin/ filled, in a copy: the checkout stays as it is
pkg=$(mktemp -d)
cp -r python/. "$pkg"
rm -rf "$pkg/tridi/_bin" "$pkg/.venv"
bin=$pkg/tridi/_bin
mkdir -p "$bin/licenses"
cp "$CARGO_TARGET_DIR/release/tridi" "$bin/"
occt=$(cargo metadata --format-version 1 --locked |
    "$PY" -c 'import json, sys; print(next(p["manifest_path"] for p in json.load(sys.stdin)["packages"] if p["name"] == "occt-sys").rsplit("/", 1)[0])')
cp "$occt/OCCT/LICENSE_LGPL_21.txt" "$occt/OCCT/OCCT_LGPL_EXCEPTION.txt" LICENSE-MIT LICENSE-APACHE "$bin/licenses/"

"$PY" -m pip install -q build wheel
mkdir -p "$OUT"
"$PY" -m build --wheel --outdir "$pkg/dist" "$pkg"
"$PY" -m wheel tags --remove --python-tag py3 --abi-tag none --platform-tag manylinux_2_28_x86_64 "$pkg"/dist/*.whl
mv "$pkg"/dist/*manylinux_2_28_x86_64.whl "$OUT/"
rm -rf "$pkg"
# occt-sys's crate is OpenCASCADE's source with the patch it's built with
version=${occt##*/occt-sys-}
cp "$occt"/../../../cache/*/"occt-sys-$version.crate" "$OUT/opencascade-$version-source.tar.gz"
auditwheel show "$OUT"/*.whl
du -h "$OUT"/*
