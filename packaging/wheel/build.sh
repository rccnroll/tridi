#!/bin/sh
# The Python wheel: the tridi binary, the OpenCASCADE its STEP helper
# needs, and the Python package around them. Inside
# quay.io/pypa/manylinux_2_28_x86_64, from the repository root:
#   packaging/wheel/build.sh OUT
# OpenCASCADE is built into /opt/occt first (packaging/wheel/occt.sh),
# unless it's already there. The repository is only read.
set -eu

OUT=$(realpath "$1")
OCCT=/opt/occt
PY=/opt/python/cp312-cp312/bin/python
here=$(dirname "$0")

"$here/occt.sh" "$OCCT"

if ! command -v cargo >/dev/null; then
    curl -fsSL https://sh.rustup.rs | sh -s -- -y -q --profile minimal --default-toolchain 1.92
    . "$HOME/.cargo/env"
fi
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/tmp/target}
OCCT_INCLUDE_DIR=$OCCT/include/opencascade OCCT_LIB_DIR=$OCCT/lib \
    cargo build --release --locked -p tridi-cli

# the package, with _bin/ filled, in a copy: the checkout stays as it is
pkg=$(mktemp -d)
cp -r python/. "$pkg"
rm -rf "$pkg/tridi/_bin" "$pkg/.venv"
bin=$pkg/tridi/_bin
mkdir -p "$bin/occt" "$bin/licenses"
cp "$CARGO_TARGET_DIR/release/tridi" "$bin/"

# the libraries the helper loads, from OpenCASCADE's, each under its
# soname: what the loader looks for
# the newest: a kept target dir may hold one built against another OpenCASCADE
helper=$(ls -t "$CARGO_TARGET_DIR"/release/build/tridi-step-*/out/tridi-step-helper | head -n 1)
LD_LIBRARY_PATH=$OCCT/lib ldd "$helper" | awk -v d="$OCCT/lib/" 'index($3, d) == 1 { print $1, $3 }' |
    while read -r soname path; do cp -L "$path" "$bin/occt/$soname"; done
cp "$OCCT"/share/licenses/* "$bin/licenses/"
cp LICENSE-MIT LICENSE-APACHE "$bin/licenses/"

# nothing else from OpenCASCADE is missing for the helper
missing=$(LD_LIBRARY_PATH=$bin/occt ldd "$helper" | grep 'not found' || true)
[ -z "$missing" ] || { echo "$missing"; exit 1; }

"$PY" -m pip install -q build wheel
mkdir -p "$OUT"
"$PY" -m build --wheel --outdir "$pkg/dist" "$pkg"
"$PY" -m wheel tags --remove --python-tag py3 --abi-tag none --platform-tag manylinux_2_28_x86_64 "$pkg"/dist/*.whl
mv "$pkg"/dist/*manylinux_2_28_x86_64.whl "$OUT/"
rm -rf "$pkg"
auditwheel show "$OUT"/*.whl
du -h "$OUT"/*.whl
