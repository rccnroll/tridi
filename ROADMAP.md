# Roadmap

What's still open, in two lists, each from urgent to low: what makes tridi
a better tool, and what makes the repo shareable. Work goes top down; a
done item leaves this file for [docs/history.md](docs/history.md), with
its date and why.

## Product

### Medium

- [ ] Keys from Open3D's viewer (its `PrintVisualizerHelp`, read 28/09),
  to implement one by one. H help is done (2.5); picked next: P screenshot (the
  thumbnail's offscreen render), color modes (file / X / Y / Z ramp; not on
  0-4, which clash with our 1-9 file toggles: Shift+0-4 or a C cycle),
  `[`/`]` field of view, Alt+Enter fullscreen. If meshes are opened often: W
  wireframe, S flat/smooth, L lighting, B back faces. Harder: N normals
  (our scans have them), Ctrl+C/V copy the view. Clash: Open3D rolls on
  Shift+left, ours pans. Skipped: line width, depth capture, render
  options, image modes.

- [ ] STEP files load one after the other in the viewer: a file that hangs
  (the Voron pinion, of the twelve tried) keeps the window empty for the
  whole 15 s. Loading them in parallel, or opening the window first, would
  hide that.

- [ ] Thumbnails in yazi too (05/10): a previewer plugin that runs
  `tridi thumb` for our formats, set up alongside the Nautilus entry.

### Low

- [ ] Open3D's scan GLBs are Z-up, and we follow glTF's Y-up (25/09): their
  thumbnails and views come out on their side next to the .pcd (seen 28/09,
  fine for now). Maybe recognize Open3D's GLBs (its generator string) and
  treat them as Z-up.

- [ ] Which other CAD formats are worth it: IGES first. monstertruck-io
  has IGES code (`src/iges.rs`, a `cadmpeg-codec-iges` feature), not
  tried.

- [ ] A tldr page (07/10). tldr-pages takes projects maintained for at
  least a year, or notable ones, and only public ones: not before tridi is
  public and has that year (from 25/09/2026). ~5 examples, from the help.
- [ ] More ways to install (07/10): AUR (`tridi`, or `tridi-bin` from the
  release), `cargo install tridi` (the name is free on crates.io, see the
  rename), a Flatpak for the other distros. Only once the repo is public.

- Known, left as they are: thin spikes on some B-spline faces
  (lensmount), from the library. Assemblies through MAPPED_ITEM aren't
  placed (none seen yet); add it when a real file needs it.

## Repo and development

### Low

- [ ] Repo topics and social preview image on GitHub (07/10): `point-cloud`,
  `pcd`, `mesh-viewer`, `step`, `nautilus`, `thumbnailer`, `rust`,
  `wayland`.
- [ ] Decide whether the repo goes public (07/10). The AUR, crates.io,
  tldr and the badges' reach depend on it. Skipped until then: a code of
  conduct, `FUNDING.yml`, discussions.
- [ ] The rules for going public (07/10), set the day it goes public (a
  private repo on the free plan can't: rulesets and branch protection
  answer 403). Already true without rules: anyone can open a pull request
  from a fork, only who has write access can merge it, push to `main` or
  push a `v*` tag (a release); today that is only Rocco.
  - A ruleset on `main`: no force push, no deletion, CI (`check`, `msrv`)
    green before a pull request merges. Rocco bypasses it, so his own
    commits and releases still go straight to `main`.
  - A ruleset on the `v*` tags: no deletion or move, so a published
    release can't be swapped.
  - Actions: approval before the workflows of any outside contributor run
    (a fork's PR could otherwise run its own code on our runners), the
    default `GITHUB_TOKEN` read-only (CI already asks for nothing more;
    only the release job writes). Never `pull_request_target`.
  - Never add collaborators with write access: help comes as pull
    requests.
  - Private vulnerability reporting on (`SECURITY.md` points to it).
