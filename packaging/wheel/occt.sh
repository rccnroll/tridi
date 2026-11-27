#!/bin/sh
# Builds OpenCASCADE into PREFIX for the wheel's STEP helper, inside
# quay.io/pypa/manylinux_2_28_x86_64 (which has no OpenCASCADE):
#   packaging/wheel/occt.sh PREFIX
# Shared libraries only (LGPL: no static linking). Does nothing if PREFIX
# already holds a build: that's what the CI cache restores.
set -eu

VERSION=7_9_3
SHA256=5ecf094ec6b12d5413dfb851d8c3590c354058aee556e32e408bdfbf8c357d57
PREFIX=$1

if [ -e "$PREFIX/lib/libTKDESTEP.so" ]; then
    echo "OpenCASCADE already in $PREFIX"
    exit 0
fi

src=$(mktemp -d)
trap 'rm -rf "$src"' EXIT
curl -fsSL -o "$src/occt.tar.gz" "https://github.com/Open-Cascade-SAS/OCCT/archive/refs/tags/V$VERSION.tar.gz"
echo "$SHA256  $src/occt.tar.gz" | sha256sum -c -
tar xzf "$src/occt.tar.gz" -C "$src"

# Visualization stays on: TKXCAF links TKV3d and TKService. What they'd
# draw with (X11, FreeType, OpenGL) is off, and --as-needed drops the GL
# and EGL they'd link anyway without using: nothing beyond libc and
# libstdc++ ends up in the wheel.
cmake -S "$src/OCCT-$VERSION" -B "$src/build" \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_INSTALL_PREFIX="$PREFIX" \
    -DINSTALL_DIR_LIB=lib \
    -DINSTALL_DIR_INCLUDE=include/opencascade \
    -DBUILD_LIBRARY_TYPE=Shared \
    -DCMAKE_SHARED_LINKER_FLAGS=-Wl,--as-needed \
    -DBUILD_MODULE_Draw=OFF \
    -DBUILD_MODULE_DETools=OFF \
    -DBUILD_DOC_Overview=OFF \
    -DUSE_XLIB=OFF \
    -DUSE_FREETYPE=OFF \
    -DUSE_OPENGL=OFF \
    -DUSE_GLES2=OFF \
    -DUSE_TK=OFF \
    -DUSE_FREEIMAGE=OFF \
    -DUSE_VTK=OFF
cmake --build "$src/build" -j "$(nproc)"
cmake --install "$src/build"
# the license the wheel ships next to the libraries, and the source the
# release carries next to the wheel (LGPL-2.1: from the same place)
mkdir -p "$PREFIX/share/licenses" "$PREFIX/share/src"
cp "$src/occt.tar.gz" "$PREFIX/share/src/occt-$VERSION.tar.gz"
cp "$src/OCCT-$VERSION/LICENSE_LGPL_21.txt" "$src/OCCT-$VERSION/OCCT_LGPL_EXCEPTION.txt" "$PREFIX/share/licenses/"
