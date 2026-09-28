# pcdview

Viewer for point clouds and meshes, with thumbnails in Nautilus. One Rust
binary does both: a window for looking at the files, and `pcdview thumb`,
which Nautilus calls to draw their previews without a window or a GPU.

- Point clouds: `.pcd` (ascii, binary, binary_compressed; rgb from PCL or
  Open3D), `.ply` without faces, `.xyz`, `.xyzrgb`, `.pts`.
- Meshes: `.glb`/`.gltf` (materials and textures), `.obj`, `.stl`, `.off`,
  `.ply` with faces.

1.0 (tag `v1.0.0`) was Python + Open3D launched by `uv`, with an f3d-drawn
thumbnailer; 2.0 replaces it and drops STEP. Why and how is in
[ROADMAP.md](ROADMAP.md).

## Install (Arch)

From a release: download `pcdview-*.pkg.tar.zst` and `sudo pacman -U` it.
From the repo (builds the committed HEAD):

    cd packaging/arch && makepkg -si

Then, once per user:

    pcdview theme dark         # or: pcdview theme light (both Nord)
    gsettings set org.gnome.nautilus.preferences thumbnail-limit 100

`pcdview theme` picks the look of both the viewer and the thumbnails, and
clears the cached thumbnails of these formats so Nautilus redraws them. It
also has to run once for the thumbnails to be ours: f3d, if installed, claims
glb, stl and obj too, and `pcdview theme` writes our entry where it wins
(`~/.local/share/thumbnailers/`). It also makes pcdview the default app for
these formats (`xdg-mime default`), which a package can't do for its users;
if an upgrade adds a format, the next time the viewer opens it brings both
up to date. The second line lets Nautilus thumbnail
files up to 100 MB (the default stops at 50). The package clears every
user's cached thumbnails of these formats on install and upgrade.

## Use

    pcdview FILE [FILE ...]              # open files together
    pcdview thumb IN OUT SIZE            # what Nautilus runs
    pcdview theme [light|dark]           # show or switch the theme
    pcdview clear-thumbnails [DIR ...]   # drop cached thumbnails of these formats

`--theme light|dark` overrides the saved theme for one run.

In the window: left drag orbits, right or middle drag (or shift + left drag)
pans, the wheel zooms; `R` resets the view, `+`/`-` change the point size,
`1`–`9` turn the N-th file off and on, `Q` or `Esc` quits.

One file is colored by height (Z); several files get one color each, so
they tell apart. A file's own colors always win, and meshes keep their
materials. glTF opens Y-up, as its standard says; everything else Z-up.
Point clouds get eye-dome lighting, which gives them depth.

The window and the thumbnails always render on the integrated GPU (never
the NVIDIA one: waking it costs seconds), and thumbnails fall back to
llvmpipe inside Nautilus's sandbox, which has no GPU. `PCDVIEW_DEBUG=1`
prints which device is used.

## What's here

- `src/main.rs` — command line, window and navigation.
- `src/cloud.rs` — point cloud readers (pcd, ply, xyz/pts, off) and the
  coloring rules.
- `src/mesh.rs` — meshes (through `three-d-asset`, plus ply/off faces), and
  which files are clouds and which meshes.
- `src/render.rs` — the scene, eye-dome lighting, the headless EGL context
  and the offscreen thumbnail.
- `src/theme.rs` — the two themes, `pcdview theme` and the thumbnail cache.
- `share/` — thumbnailer entry, MIME types, `.desktop`, icons (one SVG, the
  rest symlinks to it).
- `packaging/arch/` — PKGBUILD and install script.
- `tests/data/` — small fixtures written by Open3D.

## Develop

    cargo test                   # 21 tests; one renders on llvmpipe
    cargo run --release -- FILE
