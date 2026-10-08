# Roadmap

What's still open, from urgent to low. Work goes top down; a done item
leaves this file for [docs/history.md](docs/history.md), with its date and
why.

## Medium

- [ ] Keys from Open3D's viewer (its `PrintVisualizerHelp`, read 28/09),
  to implement one by one. H help is done (2.5); picked next: P screenshot (the
  thumbnail's offscreen render), color modes (file / X / Y / Z ramp; not on
  0-4, which clash with our 1-9 file toggles: Shift+0-4 or a C cycle),
  `[`/`]` field of view, Alt+Enter fullscreen. If meshes are opened often: W
  wireframe, S flat/smooth, L lighting, B back faces. Harder: N normals
  (scanners often save them), Ctrl+C/V copy the view. Clash: Open3D rolls on
  Shift+left, ours pans. Skipped: line width, depth capture, render
  options, image modes.

- [ ] STEP files load one after the other in the viewer: a file that hangs
  (one of twelve real parts tried) keeps the window empty for the
  whole 15 s. Loading them in parallel, or opening the window first, would
  hide that.

- [ ] Thumbnails in yazi too (05/10): a previewer plugin that runs
  `tridi thumb` for our formats, set up alongside the Nautilus entry.

## Low

- [ ] Open3D's scan GLBs are Z-up, and we follow glTF's Y-up (25/09): their
  thumbnails and views come out on their side next to the .pcd (seen 28/09,
  fine for now). Maybe recognize Open3D's GLBs (its generator string) and
  treat them as Z-up.

- [ ] Which other CAD formats are worth it: IGES first. monstertruck-io
  has IGES code (`src/iges.rs`, a `cadmpeg-codec-iges` feature), not
  tried.

- [ ] A tldr page (07/10). tldr-pages takes projects maintained for at
  least a year, or notable ones: not before 25/09/2027. ~5 examples, from the help.
- [ ] More ways to install (07/10): AUR (`tridi`, or `tridi-bin` from the
  release; `packaging/aur/PKGBUILD` builds the release tag and is ready,
  but the AUR closed new accounts on 08/10: push it when they reopen),
  `cargo install tridi` (the name is free on crates.io, see the
  rename), a Flatpak for the other distros (the app id and the metainfo
  are ready).

- Known, left as they are: thin spikes on some B-spline faces
  (lensmount), from the library. Assemblies through MAPPED_ITEM aren't
  placed (none seen yet); add it when a real file needs it.
