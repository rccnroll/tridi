# History

Why tridi works the way it does, release by release, newest first. What is
still open is in [ROADMAP.md](../ROADMAP.md); the full list of changes is in
[CHANGELOG.md](../CHANGELOG.md). Up to 2.2.0 the project was called pcdview,
and the sections for those releases use that name.

## 2.8.0 (2026-10-09)

### STEP through OpenCASCADE

- OpenCASCADE replaces monstertruck for reading and meshing STEP. On an
  84 MB Creo assembly (97 solids, 24k faces) monstertruck left fins on the
  robot arm, triangles standing off the part. It sometimes gets the surface
  parameters of a boundary point wrong, or meshes outside a face's trim.
  Mesh filters (normals, points off the surface) removed most fins but left
  holes and one fin whose points all lay on the untrimmed surface. The 2.2
  prototype had already seen thin spikes on lensmount. OpenCASCADE, the
  kernel behind FreeCAD and f3d, meshes the same file clean.
- OpenCASCADE is a system library and a package dependency, linked through a
  small C++ shim (`src/step/occt.cpp`, built by `build.rs` with `cc`).
- XCAF reads the assembly structure and the colors, so the STYLED_ITEM
  scanner is gone. OpenCASCADE returns linear colors; they are converted back
  to the file's sRGB.
- Healing took 27 of the 33 s of reading, on one core. It is off in the
  reader (after `ReadFile`, which turns it back on) and runs per part instead,
  one part per core. Before OCCT 7.8 (Debian 12, Ubuntu 24.04) there is no
  per-part healing API, so the reader heals on one core there.
- On the Creo assembly: 6 s to read, all 98 parts in 22 s (was 97 s),
  1.0M triangles, 1.2 GB peak (was 5.5 GB). The thumbnail gets 92 of 97 solids
  within its 15 s.
- The `.deb` is built on Debian 12 and depends on OCCT 7.6, so it no longer
  installs on Debian 13.

### STEP solid by solid

- The same assembly used to open empty and its thumbnail failed. One solid
  never finished, and the child process sent all triangles at the end, so the
  15 s deadline threw away the 51 solids done before it and the 41 queued
  after it.
- The child now spreads the solids over the cores and sends each one, placed,
  as soon as it is done. The viewer adds it to the file's layer and the
  legend shows `loading…` until the last one.
- The child is killed when no solid has arrived for 60 s. That file has a 34 s
  gap between two real solids, so a shorter timeout cannot tell slow from
  stuck. Solids never sent are reported as `N solids skipped`.
- Thumbnails keep a 15 s budget and draw whatever arrived.

### No solid left hanging (monstertruck, before the switch)

- monstertruck parallelizes inside rayon's global pool, and the faces of the
  big solids queued every small solid behind them: 52 of 97 in 13 s, nothing
  for 86 s, then the other 45 in 3 s. Each worker got a one-thread pool of its
  own.
- On a 429-face part used twice, two bicubic B-spline faces produced 3.7M
  triangles each and `put_together_same_attrs` never finished on them. That
  step only merged vertices the unindexed triangles don't share, so it was
  replaced by a zero-area check, and any face over 100k triangles was dropped.
- A solid not done within 60 s on the fine pass (robust triangulation, 0.1%)
  got a coarse pass (plain triangulation, 1%). Thumbnails used the coarse pass
  from the start.

## 2.7.0 (2026-10-08)

- Files load in parallel, one thread per core, and the window opens with the
  first file instead of the last. A STEP file that hangs (one in twelve of the
  real parts tried) used to keep the window empty for the full 15 s; now it
  only holds back itself.
- The legend lists every file from the start, marked `loading…` or `failed`.
  Keys 1–9 follow the command line order, so a failed file doesn't shift the
  others.
- The view reframes as files arrive, until the user moves the camera.

## 2.6.0 (2026-10-08)

- `src/` split into a library (readers, scene, rendering) and a binary on
  top, with typed errors in the library and eyre in the binary.
- Logging through `tracing` (`TRIDI_DEBUG`, `TRIDI_LOG`); `TRIDI_TIMING` is
  gone. Lints deny anything that can panic or cast silently.
- App id `io.github.rccnroll.tridi` with AppStream metainfo, which Flatpak and
  software centers need.
- Papirus icons: GTK looks up every name of a file's icon, including the
  generic `application-x-generic`, in the current theme before hicolor. Under
  Papirus a `.pcd` therefore got Papirus's generic icon. The package ships
  symlinks to the tridi mimetype icon in Papirus's sizes. The app icon has a
  single name and falls through to hicolor, so its copies were dropped.
- `SECURITY.md` with private reporting, since the thumbnailer parses
  untrusted files.
- The repository went public. History before 2.0 (Python, the prototypes,
  chezmoi) was left out, so it starts at the 2.0.0 commit. Rulesets: `main`
  can't be deleted or force-pushed and needs `check` and `msrv` green to
  merge; `v*` tags can't be moved or deleted; workflows from outside
  contributors wait for approval; the default `GITHUB_TOKEN` is read-only.
  Contributions come as pull requests, with no outside write access.

## 2.5.0 and 2.5.1 (2026-10-07)

- In-window help: H or ? shows the keys top right. Printing them to the
  terminal was useless when launched from Nautilus.
- Legend top left: each file's key, tint (an empty square for files with
  their own colors and for meshes), name and size, dimmed when turned off.
  I toggles it. L is left free for lighting, as in Open3D.
- Both panels are drawn by egui through three-d's `egui-gui` feature. The
  binary grew from 7.2 to 11 MB and the MSRV went from 1.88 to 1.92.
- CHANGELOG generated by git-cliff with the same config as the release notes;
  `CONTRIBUTING.md`, issue and PR templates; weekly grouped Dependabot updates.

## 2.4.0 and 2.4.1 (2026-10-07)

- Command line on clap derive. `--theme` works before or after the
  subcommand; help goes to stdout with exit 0, a bad flag exits 2, a file that
  fails to load exits 1. A test pins the `Exec` lines of the thumbnailer
  entries, which must keep parsing.
- Man pages and bash, zsh and fish completions come from the same clap
  definition, through a hidden `tridi generate DIR` that the packages run.
  A `build.rs` would have pulled clap into the build dependencies. File
  arguments complete any path, because clap's static completions can't filter
  by extension.
- Fuzzing in `fuzz/`, four targets on `cloud::parse`, because Nautilus runs
  `tridi thumb` on any matching file it lists. It found two bugs within
  seconds: `COUNT 0` in a PCD panicked, and PCD, PLY and OFF headers with a
  huge count preallocated exabytes. Allocation is now capped by the file
  size; ~22M inputs ran clean after the fix. Meshes go through three-d-asset
  and STEP through its watchdog, so neither is fuzzed.
- License MIT OR Apache-2.0. `rustfmt.toml` at 140 columns, since no narrower
  width matched the existing code. CI runs fmt, clippy, tests on llvmpipe and
  an MSRV job. One release workflow on `v*` tags builds both packages and
  drafts the release.

## 2.3.0 and 2.3.1 (2026-10-06)

- Renamed to tridi (2026-10-05): the name was free on the AUR, in the Arch
  repos and on crates.io. Binary, package, `.desktop`, thumbnailer entry,
  config dir (`~/.config/tridi/`) and env vars (`TRIDI_DEBUG`, …) follow, and
  the package `replaces` pcdview. Tags before 2.3 keep the old name.
- A `.deb` for Ubuntu 24.04 and Debian 12+, built in a Debian 12 container so
  the binary needs glibc 2.36 at most.
- The window is now created with winit 0.30 and glutin 0.32 instead of
  three-d's pinned winit 0.28. GNOME at scale 2 rejected the old title bar
  (`Buffer size (820x45) must be an integer multiple of the buffer_scale (2)`,
  seen on Ubuntu 24.04).

## 2.2.0 (2026-09-28)

STEP (.step/.stp) support, in Rust and without third-party programs.

### Prototype

Two libraries tried on twelve files: four local ones (two cubes, the 01465622
assembly, BBVK004 with 476 faces of planes, cylinders, cones and tori) and
eight B-spline parts from GitHub (lensmount, Pinion, camsense, …,
80–220 kB). Tolerance 0.1% of the bbox diagonal.

| | `truck-stepio` 0.3 (ricosjp) | `monstertruck-io` 0.4.1 (virtualritz's fork) |
|---|---|---|
| opens | 11/12; camsense hangs (> 120 s, eats memory) | 11/12; Pinion hangs (> 120 s) |
| quality | 1976-D twisted, spikes on lensmount, 5 Pinion faces missing | 1976-D and 1362-A clean and `Closed`, thin spikes on lensmount |
| time | 10–350 ms, BBVK004 1.3 s | 20–340 ms, BBVK004 0.5 s |
| assemblies | ignored, 01465622's parts all at the origin | placement code present, but finds no shapes in these files |
| project | slow releases | pushed 2026-09-19, 30 stars, one maintainer, also reads IGES |

monstertruck won on mesh quality and activity. The risk was depending on one
person's fork. Either library pulled 114–122 crates.

### What shipped

- STEP opens as a mesh layer in the viewer and the thumbnailer. Meshing runs
  in a child process (`pcdview step-mesh IN`, with `PR_SET_PDEATHSIG`) rather
  than a thread, because Pinion's memory grows ~20 MB/s while it hangs. The
  child is killed at 15 s and the file counts as an error. 11 of 12 files in
  120–740 ms, in bwrap too (BBVK004 1.1 s on llvmpipe). The library's normals
  are kept. The binary went from 4.1 to 6.2 MB.
- Assembly placement: a product's SHAPE_REPRESENTATION holds only a placement,
  with the brep behind a SHAPE_REPRESENTATION_RELATIONSHIP, and monstertruck
  didn't follow that link. With it, 01465622 has its 9 instances of 4 parts,
  bbox −140..50 × ±1030 × ±125, matching the reference STL. Each solid is
  tessellated once however many times it is used. MAPPED_ITEM assemblies are
  not handled.
- Colors: monstertruck drops style entities, so a small scanner of the DATA
  section follows STYLED_ITEM to COLOUR_RGB or DRAUGHTING_PRE_DEFINED_COLOUR.
  It checks the face, then the solid, then the representation. Faces are
  matched by position, and only when the library dropped none.
- Light CAD colors on the light theme looked flat because the key light came
  straight from the eye at 2.2. It now comes from above-left of the camera at
  1.6, for every mesh. The file's colors are kept as they are.
- `model/step` is claimed again in the `.desktop`, the thumbnailer entry and
  the MIME package (shared-mime-info already knows it). Rewriting the user's
  thumbnailer copy now also clears cached thumbnails: before the first viewer
  start the copy lacked `model/step`, so the package's light entry drew STEP
  thumbnails on the dark theme and they stayed cached.

## 2.1.0 (2026-09-28)

- `pcdview theme` runs `xdg-mime default` for the desktop entry's types, so
  STL, OBJ, PLY and OFF stop opening in f3d. A package can't set a user's
  default apps. xdg-utils became a dependency.
- The viewer compares the user's thumbnailer copy with the package's and
  rewrites it when it differs, so a changed MIME list doesn't go stale.
- A missing app icon came from 1.0's
  `~/.local/share/icons/Papirus/icon-theme.cache`, which outlived the icons it
  listed; GTK trusts it over /usr/share. The fix is removing that directory.
- Viewer fixes: one color for every point is treated as no color and kept
  alone, tinted only with several files; a non-mesh .obj is an error, not a
  panic; orbit 20% slower; normal window instead of maximized; `*.pcd` claimed
  over Photo CD, since Zivid headers don't start with `# .PCD`.

## 2.0.0 (2026-09-25)

A rewrite in Rust, viewer and thumbnailer in one binary with offscreen
rendering. 1.0 was Python and Open3D through `uv`, installed by chezmoi:
~1.1 s for `import open3d` with a warm cache and 1.3 GB of dependencies
(open3d + OCP) on first launch. STEP was left out because Rust had no mature
tessellator short of compiling OpenCASCADE; it came back in 2.2. 1.0's test
cases (except STEP) were ported as the acceptance suite.

### three-d prototype

A throwaway prototype read ascii and binary PCD (xyz only), opened a window
with an orbit camera, and rendered `thumb IN OUT SIZE` without a window.
Measured on an Intel Arc iGPU (Mesa) with the NVIDIA dGPU in D3cold the whole
time.

| | 1.0 | prototype |
|---|---|---|
| startup, first frame (61k) | > 1.1 s for `import open3d` alone | ~120 ms |
| startup, first frame (3M) | | ~260 ms |
| 256 px thumbnail (61k) | 1.1–1.9 s (f3d) | ~80 ms |
| 256 px thumbnail (3M) | 4.5 s (f3d) | ~370 ms |
| thumbnail, llvmpipe | | 250 ms / 640 ms |
| size | 1.3 GB of dependencies | 2.6 MB binary (strip + LTO), links only libc |

three-d 0.19.0 (2026-04-17) has one maintainer and slow releases. Notes from
the prototype:

- 0.19 removed `HeadlessContext`. The thumbnail context is built by hand with
  glutin, an EGL device and a surfaceless context, then
  `Context::from_gl_context`: about 25 lines, works with and without a
  display, and lets tridi pick the GPU so thumbnails don't wake the NVIDIA.
- `Program` only draws triangles. Points go through `draw_with` with a glow
  `draw_arrays(POINTS)` and a custom shader.
- three-d doesn't set the Wayland app_id, so the winit window is built with
  `with_name("pcdview", …)` and passed to `Window::from_winit_window`.
- It pins winit 0.28 and glutin 0.30 (replaced in 2.3.1).

### Point clouds

- PCD with every `TYPE`/`SIZE` pair (F4/F8/U1–8/I1–8), `COUNT > 1`,
  `binary_compressed` (LZF), `rgb`/`rgba` packed as float or uint; NaNs from
  organized clouds dropped. PLY without faces, `.xyz`/`.xyzrgb`/`.pts`.
- Colors as in 1.0: the file's own if present, else a Z ramp with one file or
  one palette color per file with several.
- A file that fails prints an error and the others still open; exit 1 only if
  none opens.
- 10M points (120 MB binary PCD) render a 256 px thumbnail in 590 ms with the
  NVIDIA still in D3cold. Counts and extents match Open3D on five real files.
- Eye-dome lighting in the viewer and the thumbnails: points render to their
  own color and depth textures, a full-screen pass darkens each pixel by how
  much closer its neighbors are, and depth is written back so points and
  meshes still occlude each other. Thumbnails draw points 2 px wide, since at
  1 px sparse scans showed their gaps as stripes.

### Meshes

- glb/gltf with materials and textures, obj and stl through `three-d-asset`;
  off/coff and ply with faces parsed directly, polygons fan-triangulated.
  Open3D's GLBs with no BIN chunk load as they are, so 1.0's workaround went.
  Triangle counts match Open3D.
- Normals are computed only when the file has none. 1.0 always recomputed
  them because assimp read GLB normals wrong. Back faces are drawn and lit.
  A headlight follows the camera, plus ambient light.
- glTF opens Y-up as its spec says, everything else Z-up.
- Clouds and meshes mix in one window, one layer per file, one bbox and
  camera for all.

### Viewer

- Left drag orbits, right or middle drag (or shift + left) pans, wheel zooms,
  `R` resets, `+`/`-` change point size, `1`–`9` toggle files, `Q`/`Esc`
  quits. Pan is custom code, since three-d's `OrbitControl` has none.
- 4× MSAA; point size scales with the device pixel ratio. Always renders on
  the iGPU, whatever state the NVIDIA is in.

### Thumbnailer

- Two Nord themes, light (nord6, default) and dark (nord0). `pcdview theme
  dark` saves the theme in `~/.config/pcdview/theme` and writes a copy of the
  `.thumbnailer` entry with `--theme dark` in `~/.local/share/thumbnailers/`.
  The sandbox sees neither `$HOME` nor the session bus and clears the
  environment, so the `Exec=` line is the only way to pass it. The command
  also clears cached thumbnails of the supported formats.
- The user copy is written for the light theme too: f3d's entries also claim
  glb, stl and obj, and the user directory takes precedence.
- The camera moves as close as the eight bbox corners allow, so thumbnails
  fill the frame.
- EGL device chosen by vendor, never NVIDIA, llvmpipe as fallback. With a
  cleared environment `__EGL_VENDOR_LIBRARY_FILENAMES` is pinned to Mesa so
  glvnd doesn't load NVIDIA's EGL (226 ms instead of 367 ms in the sandbox).
  Mesa's `pci id for fd …` noise can't be silenced by any env var, so stderr
  is closed while EGL starts.
- Rendered at 2× and averaged down 2×2 for anti-aliasing.
- In Nautilus's bwrap sandbox at 256 px: 61k points 226 ms, 3M points 843 ms,
  tub_mould.stl 443 ms, a textured GLB 2.1 s. f3d took 1.1–4.5 s. Nothing is
  capped by tridi; Nautilus's `thumbnail-limit` (100 MB) stops bigger files.

### Packaging

- PKGBUILD in `packaging/arch/` (at the root, makepkg's `src/` would collide
  with Rust's). It builds the committed HEAD through `git+file://`, runs
  `cargo test` and installs `/usr/bin/pcdview`. pacman's hooks refresh the
  MIME, desktop and icon caches. Build paths are remapped out of the binary.
  Package size 1.6 MB.
- The install script clears cached thumbnails of the supported formats in
  every `/home/*/.cache/thumbnails` and prints the two one-time user commands:
  `pcdview theme light|dark` and the Nautilus `thumbnail-limit` setting.
- Installed and checked: Nautilus drew 15 thumbnails in its sandbox (12 PCD,
  2 STL, a GLB), all 512 px on nord0, none failed.
- 1.0 (Python, chezmoi hook, `pcd-thumbnailer`) was removed from the dotfiles
  and from this tree. The public repository starts at this release.
