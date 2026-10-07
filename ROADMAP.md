# Roadmap

## 2.0 (released 25/09/2026, tag `v2.0.0`)

1.0 (tag `v1.0.0`) was this: Python + Open3D via `uv`, installed by chezmoi,
local only. It costs ~1.1 s of `import open3d` with a warm cache and 1.3 GB of
dependencies (open3d + OCP) on first launch.

Decided on 25 September 2026:
- **Rewrite in Rust**, viewer and thumbnailer in the same binary (offscreen
  rendering): no more uv, f3d and the stdlib-only Python thumbnailer.
- **Render with `three-d`**, 3D view only, no panel — confirmed by the
  prototype, see below.
- **STEP out of 2.0**: Rust has no mature tessellator short of compiling
  OpenCascade. Not reopened for 2.0; it is the next step after 2.1 (see Next).
- **The repo stays private**: no license. Docs, code and commits are in
  English (switched from Italian on 25 September 2026).
- Acceptance: the cases in `tests/test-pcdview.py` (except STEP), ported to
  Rust.
- Distribution: a PKGBUILD (step 7), and the package attached to the GitHub
  release.

### three-d verdict: yes (prototype of 25 September 2026)

Throwaway prototype, kept as the tag `proto-three-d` (folder `proto/`): reads
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
  `pcd.thumbnailer` from the tree (they stay in the history and in
  `v1.0.0`); README rewritten for 2.0.
- [x] Tag `v2.0.0`, GitHub release with the package; the prototype branch
  becomes the tag `proto-three-d`.

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
  outlived the icons it lists, and GTK trusts it over /usr/share. Fix on
  Rocco's machine: `rm -r ~/.local/share/icons/Papirus`.
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
The prototype branch is now the tag `proto-step`.

Decided 28/09 as the step after 2.1: opening and thumbnails for CAD files,
STEP (.step/.stp) first, in Rust and with no third-party programs (settled
on 25/09).

- [x] Throwaway prototype, like the three-d one: read and tessellate a real
  STEP, e.g. with `truck`'s STEP reader, and see what fails and how fast.
  Done 28/09 on the branch `proto-step` (now a tag), see below.

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

2.3 released 06/10/2026 (tag `v2.3.0`, private GitHub release with the
Arch package and the `.deb`): the rename and the `.deb`, nothing else.

2.3.1 released 06/10/2026 (tag `v2.3.1`, both packages): the window is
ours (winit 0.30 + glutin 0.32), three-d's pinned winit 0.28, whose title
bar GNOME at scale 2 rejected (`Buffer size (820x45) must be an integer
multiple of the buffer_scale (2)`, seen on Ubuntu 24.04).

2.4.0 released 07/10/2026 (tag `v2.4.0`, both packages), the first one
built by the Release workflow: `--help` and `--version` through clap, man
pages and bash/zsh/fish completions, the MIT OR Apache-2.0 license.

2.4.1 released 07/10/2026 (tag `v2.4.1`): the parser fixes the fuzzing
found. 2.5.0 released 07/10/2026 (tag `v2.5.0`): the legend of the open
files and H for the keys; MSRV 1.92 (egui).

## Open

Two lists, each from urgent to low: what makes tridi a better tool, and
what makes the repo shareable (from the checklist of 07/10/2026, against
clig.dev, GitHub's community health files and tldr-pages' rules). Work
goes top down; an item is ticked with its date when done.

### Product

#### Urgent

- [x] `--help`/`-h` and `--version`/`-V` (07/10). Done 07/10: clap
  derive; `--theme` goes before or after the subcommand, a test pins the
  thumbnailer's Exec lines. Today both are read as
  file names (`--help: No such file or directory`). Move the argument
  parsing to clap (derive): subcommands `thumb`, `theme`,
  `clear-thumbnails`, the global `--theme`, and the bare `FILE...` for the
  viewer. Help to stdout with exit 0; a wrong or unknown flag gets clap's
  error and the usage on stderr with exit 2; a file that fails to load
  stays exit 1. The help leads with 2-3 examples, lists the formats, the
  window keys and `TRIDI_DEBUG`, and ends with the repo URL for bugs.
  The thumbnailer entries' `Exec` lines must still parse unchanged.

#### High

- [x] Man page `tridi.1` (07/10). Done 07/10: hidden `tridi generate DIR`,
  run by the PKGBUILD and `build.sh`; one page per subcommand, Files and
  Environment in the help text, so `--help` has them too. Was: generated by `clap_mangen` from the same
  definition (an `xtask` or a hidden `tridi man` subcommand writing it at
  package time, not a `build.rs` that pulls clap into the build deps for
  nothing). Installed to `/usr/share/man/man1/` by the PKGBUILD and the
  `.deb` assets. Adds a FILES section (`~/.config/tridi/`, the
  thumbnailer entries) and an ENVIRONMENT one (`TRIDI_DEBUG`).
- [x] Shell completions for bash, zsh and fish (07/10). Done 07/10, same
  `tridi generate`; files complete any path (clap's static completions
  can't filter by extension). Was: from
  `clap_complete`, generated the same way as the man page and installed to
  `/usr/share/bash-completion/completions/tridi`,
  `/usr/share/zsh/site-functions/_tridi`,
  `/usr/share/fish/vendor_completions.d/tridi.fish`. File arguments
  complete only our extensions.
- [x] Fuzz the parsers (07/10). Done 07/10: `fuzz/`, four targets on
  `cloud::parse`. Found in seconds: `COUNT 0` in a PCD panicked (index out
  of the row), and PCD/PLY/OFF headers with a huge count preallocated it
  (exabytes, the process killed). Fixed (no preallocation past the file's
  size), with a test; then ~22M inputs clean. Meshes go through
  three-d-asset and STEP through its watchdog: not fuzzed. Was: `cargo-fuzz` targets for pcd (all three
  encodings), ply, off and xyz/pts. Nautilus runs `tridi thumb` on any
  matching file it lists, so a panic, a huge allocation from a lying
  header or an endless loop there is the most likely real bug. Each crash
  found becomes a fixture in `tests/data/` and a test.

#### Medium

- [x] H in the window prints the keys (07/10), first of the Open3D keys
  below; the same text the help and the man page show. Done 07/10: it
  printed the help's Window section to the terminal, useless when opened
  from Nautilus; since 2.5.1, H or ? (everyone else's help key) shows it
  in the window, top right, and a faint "H  keys" bottom left says so.
- [x] A legend in the window (07/10, asked by Rocco). Done 07/10: with
  several files, top left, each one's key, tint (an empty square for own
  colors and meshes), name and size; a file turned off is dimmed. Drawn by
  egui through three-d's `egui-gui` feature: the binary goes from 7.2 to
  11 MB (egui and its fonts). Not clickable, the keys toggle. Since 2.5.1
  I turns it off and on, on from the start with several files, off with
  one (I, not L: L stays free for Open3D's lighting).
- [ ] Keys from Open3D's viewer (its `PrintVisualizerHelp`, read 28/09),
  to implement one by one. Picked first: H help, P screenshot (the
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

#### Low

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

### Repo and development

#### Urgent

- [x] A license (07/10). Done 07/10: MIT OR Apache-2.0, `LICENSE-MIT` and
  `LICENSE-APACHE`, in `Cargo.toml`, the PKGBUILD (installed to
  `/usr/share/licenses/tridi/`) and the `.deb` (`/usr/share/doc/tridi/`).
  The icons are ours (the `Papirus/` ones are symlinks), nothing third
  party to declare.
- [x] `cargo fmt --check` passes (07/10). Done 07/10: `rustfmt.toml` at
  140, one formatting commit. Today it fails (e.g.
  `src/cloud.rs`): lines run up to 235 columns and no single width
  matches the code as it is (at 140 rustfmt still rewrites 62 spots in
  `main.rs`). Pick a width in a `rustfmt.toml` (140 keeps the wide style),
  run `cargo fmt` once in its own commit, so later diffs stay clean.

#### High

- [x] CI on GitHub Actions (07/10). Done 07/10: `.github/workflows/ci.yml`,
  green in ~4 min, the llvmpipe test runs there too. Was: on every push and PR, `cargo fmt
  --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`
  (the render test needs Mesa's llvmpipe: `libegl1 libgl1-mesa-dri` on the
  runner). Cache with `Swatinem/rust-cache`. Badge in the README.
- [x] One release pipeline (05/10). Done 07/10: `release.yml`, on a `v*`
  tag, checks the version, builds the Arch package (container, as
  `makepkg`) and the `.deb` (`build.sh`), and opens a draft release with
  git-cliff's notes. Left for when the repo is public: the AUR and a Nix
  flake. Was: GitHub Actions on a `v*` tag builds and
  publishes the AUR package, the `.deb` (Ubuntu 24.04, today
  `packaging/deb/build.sh`) and a Nix flake, with the changelog generated
  from the Conventional Commits. Today a release is by hand: `makepkg`,
  `build.sh`, `gh release`.
- [x] Package metadata in `Cargo.toml` (07/10). Done 07/10; MSRV 1.88
  (let chains; 1.92 since the legend's egui), checked by a second CI job. Was: `description`,
  `repository`, `homepage`, `readme`, `keywords`, `categories`
  (`graphics`, `command-line-utilities`, `visualization`),
  `rust-version` (the MSRV CI checks), `authors`.
- [x] README for someone who has never seen tridi (07/10). Done 07/10:
  screenshots of synthetic files (`docs/samples.py`), license badge, the
  history at the end. Was: a screenshot of
  the viewer and one of Nautilus with thumbnails at the top
  (`docs/` or `share/screenshots/`), badges (CI, release, license), a
  one-line pitch, "Install" before the history. The history paragraph
  moves down or into the ROADMAP. Drop the test count ("21 tests" is
  already wrong: there are 26).

#### Medium

- [x] `CHANGELOG.md` (07/10). Done 07/10: `cliff.toml` (skips the
  release and roadmap commits), the same config the release notes use;
  regenerated before each tag. Was: Keep a Changelog format, generated by
  `git-cliff` from the Conventional Commits (the same one the release
  pipeline uses); backfilled from `v2.0.0`. The ROADMAP stays the why, the
  changelog the what.
- [x] Dependabot (07/10). Done 07/10: `.github/dependabot.yml`. Was: for the Cargo dependencies and the Actions,
  weekly, grouped, so the deps don't age silently.
- [x] `CONTRIBUTING.md` (07/10). Done 07/10; the README's Develop points
  to it. Was: short: build and test commands, the
  llvmpipe test, the commit style (one-line Conventional Commits), how a
  release is cut, where the fixtures come from.
- [x] Issue and PR templates in `.github/` (07/10). Done 07/10: a bug
  form, an idea form, a PR checklist. Was: the bug report asks for
  the distro, the desktop/compositor, the scale, the package version and
  the output of `TRIDI_DEBUG=1 tridi FILE`, plus the file if it can be
  shared.

#### Low

- [ ] `SECURITY.md` (07/10): where to report privately. Matters because
  the thumbnailer parses untrusted files; GitHub's private vulnerability
  reporting covers the how.
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
  - Private vulnerability reporting on, with `SECURITY.md` above.
