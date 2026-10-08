# History

How tridi got here and why; what is still open is in [ROADMAP.md](../ROADMAP.md),
what changed in each release in [CHANGELOG.md](../CHANGELOG.md). Up to 2.2.0
tridi was called pcdview: the sections before the rename use that name.

## 2.0 (released 25/09/2026, tag `v2.0.0`)

1.0 was this: Python + Open3D via `uv`, installed by chezmoi (from the author's dotfiles),
local only. It costs ~1.1 s of `import open3d` with a warm cache and 1.3 GB of
dependencies (open3d + OCP) on first launch.

Decided on 25 September 2026:
- **Rewrite in Rust**, viewer and thumbnailer in the same binary (offscreen
  rendering): no more uv, f3d and the stdlib-only Python thumbnailer.
- **Render with `three-d`**, 3D view only, no panel — confirmed by the
  prototype, see below.
- **STEP out of 2.0**: Rust has no mature tessellator short of compiling
  OpenCascade. Not reopened for 2.0; it came back in 2.2 (below).
- **The repo stays private** for now, no license (MIT OR Apache-2.0 since
  2.4.0). Docs, code and commits are in English (switched from Italian on
  25 September 2026).
- Acceptance: the cases in `tests/test-pcdview.py`, 1.0's test suite
  (except STEP), ported to Rust.
- Distribution: a PKGBUILD (step 7), and the package attached to the GitHub
  release.

### three-d verdict: yes (prototype of 25 September 2026)

Throwaway prototype (folder `proto/`, not kept): reads
ascii/binary PCD (xyz only), window with orbit camera, Z ramp, white
background, and `thumb IN OUT SIZE` without a window. Measured on two real
files: 61k points (1.5 MB) and 3M points (59 MB), on the Intel Arc GPU (Mesa),
with the NVIDIA dGPU in **D3cold from start to finish**, for both the window
and the thumbnail.

| | 1.0 | prototype |
|---|---|---|
| startup, first frame (61k) | > 1.1 s for `import open3d` alone | ~120 ms |
| startup, first frame (3M) | | ~260 ms |
| 256 px thumbnail (61k) | 1.1–1.9 s (f3d) | ~80 ms |
| 256 px thumbnail (3M) | 4.5 s (f3d) | ~370 ms |
| thumbnail, llvmpipe | | 250 ms / 640 ms |
| size | 1.3 GB of dependencies | 2.6 MB binary (strip + LTO), links only libc |

three-d is maintained: 0.19.0 released on 17/04/2026, last push on
24/06/2026. It has a single maintainer and releases are slow. What the
prototype showed:
- **0.19 removed `HeadlessContext`**: the thumbnail is set up by hand with
  glutin, an EGL device and a surfaceless context, then
  `Context::from_gl_context`. It's ~25 lines and works fine: it runs with and
  without a display (`env -u WAYLAND_DISPLAY -u DISPLAY`) and lets us pick the
  GPU, so the thumbnail doesn't wake the NVIDIA.
- **`Program` only draws triangles**: points go through `draw_with` with a
  glow `draw_arrays(POINTS)` and our own shader. So point clouds are handled
  by our code, and three-d provides the context, camera, `OrbitControl` and
  buffers. For meshes (glb/obj/stl) there are `three-d-asset`'s loaders, not
  yet tried.
- **It doesn't set the Wayland app_id**: we build the winit window ourselves
  with `with_name("pcdview", …)` and hand it to `Window::from_winit_window`.
  Checked with `niri msg windows`.
- It drags along old dependencies, winit 0.28 and glutin 0.30: if three-d
  stalls, we're stuck there.
- Mesa prints `pci id for fd N: 10de:…, driver (null)` when it enumerates EGL
  devices: it touches the NVIDIA node but doesn't wake it. It's just noise, to
  silence in 2.0.

What the prototype leaves open is in the steps below.

## Road to 2.0

From the prototype to a `pcdview` that replaces 1.0 on this machine, in order.
Every step ends with something that runs; 1.0 stays installed from dotfiles
until step 8.

### 1. Skeleton
- [x] Cargo project at the repo root (`Cargo.toml`, `src/`), binary named
  `pcdview`, subcommand `pcdview thumb IN OUT SIZE`. Start from
  `proto/src/main.rs`, split only where a file gets too long (reader, render,
  window, thumb).
- [x] Release profile from the prototype (strip + LTO), `cargo clippy` clean.
- [x] The Python 1.0 files (`bin/`, `tests/test-pcdview.py`) stay in the
  tree until step 8, so 1.0 is still testable.

### 2. Point clouds: same as 1.0
- [x] PCD: every `TYPE`/`SIZE` pair (F4/F8/U1–8/I1–8), `COUNT > 1`,
  `binary_compressed` (LZF, ~30 lines, or the `pcd-rs` crate that
  `three-d-asset` already uses), `rgb`/`rgba` packed as float or uint. PCL
  organized clouds: drop NaNs (already in the prototype).
- [x] PLY without faces (ascii and binary little-endian, with or without
  colors), `.xyz`/`.xyzrgb`/`.pts` (text, one point per line).
- [x] Colors like 1.0: the file's own colors if present; else with one file
  the Z ramp (RAMP_LO → RAMP_HI); with several files one flat color per file
  from `PALETTE`, the ramp still skipped when the file has colors.
- [x] Axis triad at the min corner of the merged bbox, size 20% of the
  largest extent.
- [x] One file that fails prints `[pcdview] name: error` and the others still
  open; if none opens, exit 1. Summary line on stdout per file
  (`N points, extent …`).
- [x] Big clouds: measure 10M+ points, check GPU memory on the Intel.
  10M points (120 MB binary PCD): 590 ms for a 256 px thumbnail, NVIDIA
  still in D3cold. Counts and extents match Open3D on five real files
  (3M and 3.5M point scans, PCD with rgb, PLY).

### 3. Meshes
- [x] glb/gltf with materials and textures via `three-d-asset` (`gltf` +
  `data-url` features). Open3D's GLBs (JSON chunk only, no BIN) load as
  they are: 1.0's `read_glb_open3d` workaround isn't needed, and a 13 MB
  scan GLB that Open3D itself reads back as 0 triangles opens fine.
- [x] obj and stl via `three-d-asset` (uppercase `.STL` included); off/coff
  and ply with faces by hand, polygons fan-triangulated. Triangle counts
  match Open3D on the real files (RAV4 GLB 399k, STL moulds, PLY).
- [x] Normals computed only when the file has none (1.0 recomputed them
  always, but only because assimp read GLB normals wrong; three-d-asset
  doesn't). Back faces drawn and lit (no culling, three-d's shader flips
  the normal). A headlight that follows the camera plus ambient. Meshes
  without a material get the matte blue-grey (CAD_GREY), or white when
  they carry vertex colors.
- [x] Mixed clouds and meshes in one window: one layer per file, one bbox
  and camera for all.
- [x] Camera (decided 25/09): glTF opens Y-up, as its standard says, the
  rest Z-up, both in three-quarter view. Open3D writes scan GLBs Z-up, so
  those stand up: that's Open3D's problem, not ours.
- [x] Shading: dark STLs (no material) were almost black; the mesh color
  now comes from the theme (dark grey on light, nord4 on nord).

### 4. The viewer window
- [x] Controls (decided 25/09): left drag orbits, right or middle drag (or
  shift + left) pans, the wheel zooms, `R` resets the view, `+`/`-` change
  the point size, `1`–`9` turn the N-th file off and on (echoed on stdout,
  the key map is printed at start when there are several files), `Q`/`Esc`
  quits. Pan is our own code: three-d's `OrbitControl` has none.
- [x] Title `pcdview — a.pcd, b.glb` with the app_id fixed at `pcdview`
  (checked with `niri msg windows`). The title is set once: three-d owns
  the winit window afterwards and has no `set_title`, so the on/off state
  goes to stdout only.
- [x] Anti-aliasing: three-d's default surface has 4× MSAA. Point size
  scales with the device pixel ratio.
- [x] Always on the iGPU: the window renders on `Mesa Intel(R) Graphics
  (ARL)` (`PCDVIEW_DEBUG=1` prints it) with the NVIDIA in D3cold, and still
  on the Intel with the NVIDIA awake (D0).
- [x] Tried by hand (25/09): works. Rocco has notes for later.

### 5. Thumbnailer
- [x] `pcdview thumb` for every format the viewer opens, one look for all.
- [x] Themes (decided 25/09): two, both Nord, `light` (nord6 background,
  the default) and `dark` (nord0, 1.0's thumbnails). `pcdview theme dark`
  switches viewer and thumbnails together: it saves the theme in
  `~/.config/pcdview/theme` for the viewer, writes a copy of the package's
  `.thumbnailer` with `--theme dark` in `~/.local/share/thumbnailers/`
  (the sandbox sees neither `$HOME` nor the session bus and clears the
  environment; the `Exec=` line is the only way in), and clears the cached
  thumbnails of our formats so Nautilus redraws them. `pcdview theme light`
  writes the copy without the flag (see step 7 for why it's always
  written); `pcdview theme` prints the current one.
- [x] Framing: the camera comes as close as the eight bbox corners allow,
  so thumbnails fill the frame like f3d's did (the viewer's first view too).
- [x] EGL device by vendor, never NVIDIA, llvmpipe as the fallback
  (`PCDVIEW_DEBUG=1` lists the devices and the one used). In the thumbnail
  path `__EGL_VENDOR_LIBRARY_FILENAMES` is pinned to Mesa when the
  environment has been cleared, so glvnd doesn't load NVIDIA's EGL
  (226 ms instead of 367 ms in the sandbox). Mesa's `pci id for fd …`
  lines are gone: no env var silences them, so stderr is closed while EGL
  starts (errors still come back as results).
- [x] Anti-aliasing: rendered at 2× and averaged down 2×2.
- [x] Nautilus's sandbox, simulated with the same bwrap arguments
  (`--dev /dev` has no `/dev/dri`, `--clear-env`, `--unshare-all`): works on
  llvmpipe with the NVIDIA in D3cold. 256 px: 61k points 226 ms, 3M
  points 843 ms, tub_mould.stl 443 ms, RAV4 GLB with textures 2.1 s; f3d
  took 1.1–4.5 s.
- [x] Big files: no cap of our own. 10M points take 590 ms on the Intel;
  Nautilus's `thumbnail-limit` (100 MB, set in step 7) stops anything much
  bigger before it reaches us.
- [x] Real Nautilus run: done with the package (step 7).
- [x] Shading of point clouds (asked 25/09): eye-dome lighting, in the
  viewer and the thumbnails. Points go to their own color + depth
  textures, then a full-screen pass darkens each pixel by how much its
  neighbors are closer, and writes the depth back so points and meshes
  still hide each other. Thumbnails draw points 2 px wide: at 1 px sparse
  scans showed their gaps as stripes.
- [x] No third-party programs (decided 25/09): everything in Rust, so f3d
  is gone, STEP thumbnails included.

### 6. Tests
- [x] Port cases 1–9 of `tests/test-pcdview.py` to `cargo test`: ramp
  bottom to top, ramp and palette dark enough for white, per-file tints,
  own colors untouched, flat cloud with no division by zero, unreadable
  file is an error and not an empty window, fixed app_id, Open3D GLB with
  no BIN chunk, ply without faces is a cloud. Case 10 (STEP) is dropped.
- [x] One thumbnail test on llvmpipe (runs without a GPU or display): PNG
  of the right size, not all background.
- [x] Sample files in `tests/data/` (small ones generated by the tests,
  none of the 59 MB ones in git).

### 7. Packaging
- [x] PKGBUILD in `packaging/arch/` (in the root, makepkg's `src/` would
  collide with Rust's). It builds the committed HEAD through
  `git+file://`, runs `cargo test`, and installs `/usr/bin/pcdview`, which
  also satisfies bwrap's "under /usr". pacman's own hooks refresh the MIME
  and desktop databases and the icon cache: the chezmoi hook's steps are
  gone. Build paths are remapped out of the binary. Package: 1.6 MB.
- [x] Contents: the binary, `pcdview.thumbnailer`
  (`Exec=/usr/bin/pcdview thumb %i %o %s`), the MIME package, the
  `.desktop` (no STEP, canonical MIME types), the icons (one SVG, the rest
  relative symlinks).
- [x] `pcdview.install`: on install and upgrade, `pcdview clear-thumbnails`
  on every `/home/*/.cache/thumbnails` (STEP included: 1.0 drew them and
  nothing will redraw them); after install it prints the two commands a
  user runs once, `pcdview theme light|dark` (pacman can't ask) and the
  Nautilus `thumbnail-limit` at 100 MB (a per-user gsettings key). Both are
  in the README.
- [x] The user's thumbnailer entry is always written, for light too: f3d is
  installed and its entries in `/usr/share/thumbnailers` also claim glb,
  stl and obj, and between two entries there directory order decides; the
  user's directory comes first (1.0 relied on this too).
- [x] Installed and checked for real (25/09): `pcdview 2.0.0dev.r28`,
  then `pcdview theme dark`. Nautilus drew 15 thumbnails in its sandbox
  (12 PCD, 2 STL, the RAV4 GLB), all 512 px on nord0, none failed: our
  entry in `~/.local/share` wins over f3d's.

### 8. Switching over
- [x] Remove pcdview from `rccnroll/dotfiles` (Rocco, 25/09, `74cbc5c`): the files, the
  chezmoi hook, and what they installed: `/usr/local/bin/pcd-thumbnailer`,
  `~/.local/bin/pcdview`, `~/.local/share/thumbnailers/pcd.thumbnailer`,
  `~/.local/share/applications/pcdview.desktop` (it would shadow the
  package's), `~/.local/share/mime/packages/pointcloud.xml`. Before
  installing the package.
- [x] `model/step` out of the `.desktop` MimeType and the thumbnailer entry
  (the 2.0 ones in `share/`).
- [x] Remove `bin/`, the Python test, `chezmoi/` and 1.0's
  `pcd.thumbnailer` from the tree; README rewritten for 2.0.
- [x] Tag `v2.0.0`, GitHub release with the package. The repository's
  public history starts here: what came before (1.0, the prototypes) is
  only told in this file.

## After 2.0

2.1 released 28/09/2026 (tag `v2.1.0`, private GitHub release with the
package): the ticked items below.

- [x] STL, OBJ, PLY and OFF still open with f3d on double click: the
  package can't set a user's default apps. Done 28/09: `pcdview theme` runs
  `xdg-mime default pcdview.desktop` for the desktop entry's types
  (xdg-utils is now a dependency).
- [x] pcdview's icon doesn't show after the package install (seen 25/09):
  find out where it's missing (hicolor, Papirus, the icon caches). Found
  28/09, not the package: 1.0's `~/.local/share/icons/Papirus/icon-theme.cache`
  outlived the icons it lists, and GTK trusts it over /usr/share. Fix:
  remove 1.0's `~/.local/share/icons/Papirus`.
- [x] Rocco's notes on the viewer, from trying it (25/09). Done 28/09 from a
  hand checklist: one color for every point kept alone and tinted with
  several files; a non-mesh .obj is an error, not a panic; orbit 20%
  slower; a normal window, not maximized; `*.pcd` claimed over Photo CD
  (Zivid headers don't start with `# .PCD`).
- [x] If the package's MIME list changes, users' copies of the thumbnailer
  entry go stale until they run `pcdview theme` again. Done 28/09: the
  viewer compares the copy with the package's and, if it differs, rewrites
  it and sets the default apps again.

## 2.2: STEP (released 28/09/2026, tag `v2.2.0`)

Private GitHub release with the package. Installed and checked by Rocco
on 29/09: STEP opens, and the thumbnails and viewer follow the dark theme.
The prototype was thrown away once STEP was in.

Decided 28/09 as the step after 2.1: opening and thumbnails for CAD files,
STEP (.step/.stp) first, in Rust and with no third-party programs (settled
on 25/09).

- [x] Throwaway prototype, like the three-d one: read and tessellate a real
  STEP, e.g. with `truck`'s STEP reader, and see what fails and how fast.
  Done 28/09 on a throwaway branch, see below.

### STEP prototype (28/09): usable, with a watchdog and our own assemblies

Two variants on twelve files: Rocco's four (two cubes, the 01465622
assembly, BBVK004 with 476 faces of planes, cylinders, cones and tori) and
eight with B-spline surfaces from GitHub (lensmount, Pinion, camsense, …,
80–220 kB). Tolerance 0.1% of the bbox diagonal, output as STL and
looked at through `pcdview thumb`.

| | `truck-stepio` 0.3 (ricosjp) | `monstertruck-io` 0.4.1 (virtualritz's fork) |
|---|---|---|
| opens | 11/12; camsense hangs (> 120 s, eats memory) | 11/12; Pinion hangs (> 120 s) |
| quality | 1976-D twisted, spikes on lensmount, 5 Pinion faces missing | 1976-D and 1362-A clean and `Closed`, lensmount still has thin spikes |
| time | 10–350 ms, BBVK004 1.3 s | 20–340 ms, BBVK004 0.5 s |
| assemblies | ignored: 01465622's parts all at the origin | has the placement code, but finds no shapes in any of these files; unplaced solids as a fallback |
| project | slow releases | pushed 19/09/2026, 30 stars, one maintainer, also reads IGES |

What it means for pcdview:
- **Speed is fine** for the viewer and thumbnails: half a second for the
  biggest part.
- **A watchdog is required**: each library hangs on one real file out of
  twelve. Tessellation runs under a deadline, and a file that misses it is
  an error, like an unreadable one (a thumbnail isn't drawn). A thread was
  the plan; it became a process, see below.
- **Assemblies are ours to place**: neither library places 01465622's
  parts. That means following `NEXT_ASSEMBLY_USAGE_OCCURRENCE` →
  `CONTEXT_DEPENDENT_SHAPE_REPRESENTATION` → `ITEM_DEFINED_TRANSFORMATION`.
  Check against 01465622.stl (bbox X −140..50, ours −75..75).
- Colors (`STYLED_ITEM` → `COLOUR_RGB`) not tried (done later, below).
- **Library: monstertruck** (decided 28/09, Rocco looked at both contact
  sheets): the better meshes, and it is alive. The risk is that it is one
  person's fork. Either one is heavy: 114–122 crates in
  the tree, and a 3.4–4.8 MB prototype binary (not stripped, no LTO)
  against our 1.6 MB package.
- [x] STEP in pcdview with `monstertruck-io`: `.step`/`.stp` (any case)
  as a mesh layer, in the viewer and the thumbnailer. Done 28/09: a child
  process (`pcdview step-mesh IN`, with `PR_SET_PDEATHSIG`) rather than a
  thread, because the Pinion's memory grows ~20 MB/s while it hangs; it is
  killed at 15 s and the file is an error. 11 of the 12 files in
  120–740 ms, in bwrap too (BBVK004 1.1 s on llvmpipe). The library's
  normals are kept, so the shading is clean. The binary goes from 4.1 to
  6.2 MB. Files load one after the other, so a hanging STEP holds the
  window for 15 s.
- [x] Assemblies placed by our own code (see the prototype), checked
  against 01465622.stl. Done 28/09: monstertruck's `step_assy` already
  reads the graph and the matrices; what it missed is that a product's
  SHAPE_REPRESENTATION holds only a placement, with the brep behind a
  SHAPE_REPRESENTATION_RELATIONSHIP, and we follow that link. 01465622 now
  has its 9 instances of 4 parts, bbox −140..50 × ±1030 × ±125 as in the
  STL. Each solid is tessellated once, however many times it is used.
  Test: `tests/data/assembly.step`, the cube twice, one copy moved and
  turned. Not handled: assemblies through MAPPED_ITEM (none seen yet).
- [x] Colors from `STYLED_ITEM`, else CAD_GREY like the other meshes.
  Done 28/09: monstertruck drops the style entities, so a small scanner
  of the DATA section follows STYLED_ITEM → … → COLOUR_RGB or
  DRAUGHTING_PRE_DEFINED_COLOUR. On a face (FreeCAD: lensmount's green
  face), a solid (most exporters), or a representation (1362-A), in that
  order; faces are matched to the file's by position, and only when the
  library dropped none. Test: `tests/data/colors.step`.
- [x] Light CAD colors on the light theme: BBVK004 and 1362-A
  (0.79, 0.82, 0.93) and FreeCAD's 0.8 grey are barely darker than nord6
  (seen 28/09). Decided 28/09: they stay as the file says (Rocco uses
  dark). What made them flat was the light: the key light came straight
  from the eye at 2.2, so every face turned to us was the same white. It
  now comes from above-left of the camera at 1.6, for every mesh.
- [x] Back in the package: `model/step` in the `.desktop`, the thumbnailer
  entry and the MIME package; tests on the cubes and on a B-spline part.
  Done 28/09: shared-mime-info already knows `model/step` (`.STP` too), so
  the MIME package is unchanged; the viewer rewrites the user's
  thumbnailer copy and the default apps on its next start (f3d's OCCT
  entry claims `model/step` too). Tests: cube, assembly, colors; the
  B-spline parts are checked by hand, their files aren't ours to commit.
  Seen installing it (28/09): the STEP thumbnails came out light on the
  dark theme. Until the viewer's first start the user's copy lacked
  `model/step`, so the package's entry (light) drew them, and the rewrite
  left them cached. The rewrite now clears the cached thumbnails too.

## Renamed to tridi (05/10/2026)

pcdview became tridi after 2.2.0: the name was free on the AUR, in the Arch
repos and on crates.io. Binary, package, `.desktop`, thumbnailer entry,
config dir (`~/.config/tridi/`) and env vars (`TRIDI_DEBUG`, …) follow; the
package `replaces` pcdview. The repo is `rccnroll/tridi`. Tags and the
history above keep the old name.

## 2.3 to 2.6: a tool others can use (06–08/10/2026)

- **Our own window** (2.3.1): winit 0.30 + glutin 0.32 instead of three-d's
  pinned winit 0.28, whose title bar GNOME at scale 2 rejected (`Buffer
  size (820x45) must be an integer multiple of the buffer_scale (2)`, seen
  on Ubuntu 24.04).
- **A `.deb`** for Ubuntu 24.04 and Debian 12+ (2.3.0), built in a Debian 12
  container so the binary needs glibc 2.36 at most.
- **A command line that behaves** (2.4.0): clap derive; `--theme` goes
  before or after the subcommand; help on stdout with exit 0, a wrong flag
  exits 2, a file that fails to load 1. A test pins the thumbnailer
  entries' `Exec` lines, which must keep parsing.
- **Man pages and completions** (2.4.0) from the same clap definition,
  through a hidden `tridi generate DIR` the packages run (not a `build.rs`
  that would pull clap into the build deps). Files complete any path:
  clap's static completions can't filter by extension.
- **Fuzzing** (2.4.1): `fuzz/`, four targets on `cloud::parse`, because
  Nautilus runs `tridi thumb` on any matching file it lists. Found in
  seconds: `COUNT 0` in a PCD panicked, and PCD/PLY/OFF headers with a
  huge count preallocated it (exabytes). Fixed: no preallocation past the
  file's size; then ~22M inputs clean. Meshes go through three-d-asset and
  STEP through its watchdog: not fuzzed.
- **Keys and legend in the window** (2.5.0, 2.5.1): H or ? shows the keys
  top right (printing them to the terminal was useless from Nautilus); a
  legend top left gives each file's key, tint (an empty square for own
  colors and meshes), name and size, a file turned off dimmed; I toggles
  it (not L: L stays free for Open3D's lighting). Drawn by egui through
  three-d's `egui-gui` feature: the binary goes from 7.2 to 11 MB, and the
  MSRV from 1.88 to 1.92.
- **The repo, shareable** (07–08/10): MIT OR Apache-2.0; `rustfmt.toml` at
  140 columns (no narrower width matched the code), one formatting commit;
  CI (fmt, clippy, tests on llvmpipe, an MSRV job); one release workflow on
  `v*` tags that builds both packages and drafts the release with
  git-cliff's notes, the same config as `CHANGELOG.md`; Dependabot;
  `CONTRIBUTING.md`; issue and PR templates; `SECURITY.md`, because the
  thumbnailer parses untrusted files.
- **Papirus icon copies** (08/10): GTK looks for every name of a file's
  icon, the generic `application-x-generic` included, in the current theme
  before hicolor, so under Papirus a `.pcd` got Papirus's generic icon:
  the package keeps symlinks to our mimetype icon in Papirus's sizes. The
  app icon has a single name and falls through to hicolor: its copies
  went.
- **A library under the binary** (2.6.0): `src/` split into a library
  (readers, scene, rendering) and the binary on top, with typed errors,
  `tracing` instead of prints (`TRIDI_DEBUG`, `TRIDI_LOG`; `TRIDI_TIMING`
  went) and lints that deny what can panic or cast silently. The app id
  became `io.github.rccnroll.tridi`, with AppStream metainfo, which a
  Flatpak or a software center needs.
- **Public** (08/10): the history before 2.0 (Python, the prototypes,
  chezmoi) stays out, so the repository starts at the 2.0.0 commit. Rules
  set the same day: `main` can't be deleted or force-pushed and needs CI
  (`check`, `msrv`) green to merge, the `v*` tags can't be moved or
  deleted, an outside contributor's workflows wait for approval, the
  default `GITHUB_TOKEN` is read-only, and vulnerabilities are reported
  privately. Help comes as pull requests: no collaborators with write
  access.

## 2.7 (released 08/10/2026)

- **Files load in parallel** (08/10): one thread per core, and the window
  opens with the first file read instead of the last. A STEP file that
  hangs (one of twelve real parts tried) used to keep the window empty for
  the whole 15 s; now it holds back only itself. The legend lists every
  file from the start, `loading…` or `failed` until it's in, and the keys
  1–9 follow the command line, so a file that fails doesn't shift the
  others. The view reframes as files come in, until the user moves it.

## After 2.7

- **STEP solid by solid** (08/10): an 84 MB Creo assembly (97 solids,
  24k faces) opened to nothing and its thumbnail broke: one solid the
  library never finishes, and the child sent its triangles all at the end,
  so the 15 s deadline threw away the 51 done before it and the 41 queued
  behind it. Now the child spreads the solids over the cores and sends
  each, placed, as soon as it's done; the viewer adds it to the file's
  layer and the legend counts `loading…` until the last. The child is
  killed once no solid has come for 60 s (that file goes 34 s between two
  real ones: slow and stuck look the same), and what it never sent is
  `N solids skipped`. Thumbnails keep a 15 s budget and draw what came:
  71 of 97 there. Measured on that file: 7 s to parse, 1.9M triangles
  in 56 s, 5 solids skipped, 6.8 GB at the peak (the solids in flight at
  once).
- **No solid left hanging** (08/10): the 5 solids skipped weren't stuck.
  monstertruck parallelizes inside rayon's global pool, and the big
  solids' faces queued every small one behind them: 52 of 97 in 13 s,
  then nothing for 86 s, then the other 45 in 3 s. Each worker now has a
  one-thread pool of its own, so solids run side by side and a slow one
  holds back only itself. Two more causes, on one 429-face part used
  twice: two bicubic B-spline faces came out at 3.7M triangles each, and
  `put_together_same_attrs` never finished on them. That step only
  merged vertices our unindexed triangles don't share: it went, a
  zero-area check took its place, and a face over 100k triangles is left
  out as the library's blow-up. A solid the fine pass (robust
  triangulation, 0.1%) doesn't hand back within 60 s gets a coarse pass
  (plain triangulation, 1%), which finished all 6 slowest alone;
  thumbnails go coarse from the start. On that file: all 97 solids in
  97 s with the fine pass alone, 5.5 GB at the peak; the thumbnail has
  94 of 97 in 15 s, the arm the gripper hung from included.
- **STEP through OpenCASCADE** (09/10): on the same assembly the robot
  arm grew fins, triangles standing off the part. monstertruck sometimes
  gets a boundary point's surface parameters wrong, or meshes outside a
  face's trim, and its triangles then reach across the face; filters on
  the mesh (normals, points off the surface) took most away but left
  holes and still one fin, every point of it on the untrimmed surface.
  The prototype (28/09) had seen them already ("lensmount still has thin
  spikes"). OpenCASCADE, the kernel FreeCAD and f3d use, meshed the file
  clean, so it replaces monstertruck: the system's library, linked through
  a small C++ side (`src/step/occt.cpp`, built by `build.rs` with `cc`),
  a package dependency rather than something we ship. XCAF reads the
  assembly and the colors (our STYLED_ITEM scanner went); the parts are
  healed and meshed a part per core and handed to the same child, wire and
  passes as before. The healing was 27 of the 33 s of reading, one core:
  it's turned off in the reader (after `ReadFile`, which sets it back) and
  run per part instead. On that file: 6 s to read, all 98 parts in 22 s
  (was 97 s), 1.0M triangles, 1.2 GB at the peak (was 5.5 GB); the
  thumbnail has 92 of 97 in 15 s. OpenCASCADE's colors are linear, turned
  back into the file's sRGB. Before 7.8 (Debian 12, Ubuntu 24.04) there's
  no per-part healing API, so the reader heals on one core there, as
  before; the `.deb` built on Debian 12 depends on 7.6 and so no longer
  installs on Debian 13.
