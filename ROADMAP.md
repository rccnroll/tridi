# Roadmap

What's still open, from urgent to low. Work goes top down; a done item
leaves this file for [docs/history.md](docs/history.md), under the release
that shipped it.

## Medium

- [ ] The viewer as a crate (3.0): `tridi::show(["scan.pcd", "part.step"])`
  from any Rust program, Open3D's `draw_geometries` style, with clouds
  from memory too. The repo becomes a workspace: `tridi-formats` (the
  readers, no dependencies, also usable next to rerun), `tridi-step`
  (OpenCASCADE and its child process, started through `tridi::init()`
  rather than `current_exe step-mesh`), `tridi` (the viewer) and
  `tridi-cli` (the `tridi` binary, thumbnails, packaging). Thumbnails stay
  out of the public API.

- [ ] Keys from Open3D's viewer (its `PrintVisualizerHelp`), one by one. H help is done (2.5); picked next: P screenshot (the
  thumbnail's offscreen render), color modes (file / X / Y / Z ramp; not on
  0-4, which clash with our 1-9 file toggles: Shift+0-4 or a C cycle),
  `[`/`]` field of view, Alt+Enter fullscreen. If meshes are opened often: W
  wireframe, S flat/smooth, L lighting, B back faces. Harder: N normals
  (scanners often save them), Ctrl+C/V copy the view. Clash: Open3D rolls on
  Shift+left, ours pans. Skipped: line width, depth capture, render
  options, image modes.

- [ ] Thumbnails in yazi too: a previewer plugin that runs
  `tridi thumb` for our formats, set up alongside the Nautilus entry.

## Low

- [ ] Open3D's scan GLBs are Z-up, and tridi follows glTF's Y-up: their
  thumbnails and views come out on their side next to the .pcd. Maybe recognize Open3D's GLBs (its generator string) and
  treat them as Z-up.

- [ ] Which other CAD formats are worth it: IGES first. OpenCASCADE
  reads it (`IGESCAFControl_Reader`, colors too), the rest of the STEP
  path would serve as is.

- [ ] Dolphin (KDE) thumbnails: KDE ignores `.thumbnailer` files
  (bugs.kde.org 402721) and wants a KIO ThumbnailCreator plugin, C++
  against KF6, that would run `tridi thumb`.

- [ ] A tldr page. tldr-pages takes projects maintained for at
  least a year, or notable ones: not before 2027-09-25. ~5 examples, from the help.
- [ ] More ways to install: AUR (`tridi`, or `tridi-bin` from the
  release; `packaging/aur/PKGBUILD` builds the release tag and is ready,
  but the AUR closed new accounts on 2026-10-08: push it when they reopen),
  `cargo install tridi` (the name is free on crates.io, see the
  rename), a Flatpak for the other distros (the app id and the metainfo
  are ready).
