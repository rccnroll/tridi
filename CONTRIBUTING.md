# Contributing

Bugs and ideas go in the issues; the templates ask for what's needed to
reproduce. A pull request is welcome for anything already in
[ROADMAP.md](ROADMAP.md), or after an issue for anything else.

## Build and test

STEP needs OpenCASCADE's headers and libraries, 7.6 or later: `opencascade`
on Arch and Fedora, `libocct-data-exchange-dev libocct-ocaf-dev
libocct-modeling-algorithms-dev libocct-modeling-data-dev
libocct-foundation-dev` on Debian/Ubuntu. Elsewhere, point
`OCCT_INCLUDE_DIR` and `OCCT_LIB_DIR` at them.

    cargo build --release
    cargo run --release -p tridi-cli -- FILE
    cargo test

One test renders a thumbnail offscreen: with no GPU it runs on Mesa's
llvmpipe (`libegl-mesa0` and `libgl1-mesa-dri` on Debian/Ubuntu, `mesa` on
Arch), as in CI.

Before pushing, what CI checks on every push and PR:

    cargo fmt --check
    cargo clippy --workspace --all-targets --locked -- -D warnings
    cargo test --workspace --locked
    cargo publish --workspace --dry-run --locked

plus `cargo check --workspace` on the oldest Rust supported, 1.92 (`rust-version` in
`Cargo.toml`).

The lints in `Cargo.toml` deny what can panic or lose data silently:
indexing, `unwrap`, `as` casts, `unsafe` without a `// SAFETY:` comment.
The only way around one is a scoped `#[expect(lint, reason = "...")]`
that says why; tests may unwrap and index.

The files in `tests/data/` are tiny and made for the tests: no real scans
or CAD from anyone's work. The README's screenshots and GIFs come from
synthetic files too (`python3 scripts/samples.py`, recorded with
`scripts/record.sh`).

## Layout

The repository is a Cargo workspace of four crates. Dependencies go one
way, down this list, and nothing depends on `tridi-cli`:

- `crates/tridi-cli/` — the `tridi` command: `main.rs` parses the command
  line and dispatches, `thumb.rs` is `tridi thumb` (pins Mesa's EGL, writes
  the PNG), `desktop.rs` is `tridi theme` with the thumbnail cache. clap,
  eyre, tracing-subscriber and image live here only. The `.deb` metadata is
  in its `Cargo.toml`.
- `crates/tridi/` — the viewer as a library: `show` and `Viewer`. The
  window in `src/viewer/` (`mod.rs` the API, `app.rs` the event loop,
  `window.rs` the window and its GL, `load.rs` the loader threads,
  `navigate.rs` the mouse, `overlay.rs` the legend and keys); what a file is
  (a cloud or a mesh, which way is up) in `model.rs`; the coloring rules and
  the two themes in `color.rs` and `theme.rs`; the scene with eye-dome
  lighting in `src/render/`. Behind the `thumbnail` feature, for
  `tridi-cli` only and outside the semver promise: `thumbnail.rs`, with the
  headless EGL context on the right GPU and the offscreen render. The
  `step` feature (default) brings in `tridi-step`.
- `crates/tridi-step/` — STEP through OpenCASCADE, in a helper process
  embedded in the crate: `helper/occt.cpp` reads, heals and meshes the
  parts, `helper/main.cpp` writes them to the parent, both built by
  `build.rs`; `src/parent.rs` runs the helper with its deadline and reads
  what it sends, `src/helper.rs` holds the embedded executable, `src/lib.rs`
  the API.
- `crates/tridi-formats/` — the point cloud readers, one file per format
  (pcd, ply, off, xyz/xyzrgb/pts). Every byte is untrusted, and the crate
  depends on nothing of tridi's, so the fuzz targets build it alone.
- `share/` — what the packages install: thumbnailer entry, MIME types,
  `.desktop`, AppStream metainfo, icons. Under `Papirus/` only symlinks to
  our mimetype icon: GTK looks for every name of a file's icon, generic
  ones included, in the current theme before hicolor, so without them
  Papirus users get its generic file icon.
- `packaging/` — the PKGBUILD and the `.deb` build (cargo-deb, metadata in
  `crates/tridi-cli/Cargo.toml`) with their install scripts, and the
  licenses of the fonts egui builds into the binary.
- `tests/data/` — tiny fixtures: PCD and GLB written by Open3D, STEP by
  hand.
- `fuzz/` — cargo-fuzz targets for the readers, and their seeds.
- `docs/` — `history.md` (how tridi got here, and why) and `media/`, the
  README's screenshots and GIFs.
- `scripts/` — `samples.py` writes the synthetic files behind the
  screenshots, `record.sh` records the GIFs on niri.

Every source file has the same sections in the same order (imports,
constants, errors, the main type, helpers, tests), each fenced by a
`// ==== Title ==== {{{` banner that folds in vim.

## Fuzzing

The point cloud parsers read whatever file Nautilus lists, so they are
fuzzed: `cargo install cargo-fuzz`, then from `fuzz/`,
`mkdir -p corpus/pcd && cargo +nightly fuzz run -O pcd corpus/pcd seeds/pcd`
(or `ply`, `off`, `text`). New inputs go to the first folder, `corpus/`
(ignored), so the seeds stay as committed. A crash lands in
`fuzz/artifacts/`; it becomes a test in `crates/tridi-formats/src/`, next to the reader.

## Commits

One line, [Conventional Commits](https://www.conventionalcommits.org),
no body: `feat(viewer): ...`, `fix(cloud): ...`, `docs(readme): ...`. The
changelog and the release notes are generated from them, so the line is
what a user reads. The why goes in docs/history.md (decided) or
ROADMAP.md (open), not in the commit.

## Releases

1. Set the version in `Cargo.toml`, in `[workspace.package]` and in the
   `version = "..."` of the internal crates in `[workspace.dependencies]`
   (one sed changes both), and the PKGBUILD's `pkgver`:

       sed -i 's/version = "2.8.0"/version = "3.0.0"/' Cargo.toml

   then `cargo check` (for `Cargo.lock`), and add
   `<release version="X.Y.Z" date="YYYY-MM-DD"/>` on top of `<releases>` in
   `share/metainfo/io.github.rccnroll.tridi.metainfo.xml`.
2. Add this release under the changelog's header (`--prepend` would
   repeat the header, and regenerating the whole file would empty 2.0.0,
   where the history starts):

       { sed -n 1,5p CHANGELOG.md; git cliff --unreleased --tag vX.Y.Z --strip header; sed -n '6,$p' CHANGELOG.md; } > CHANGELOG.new
       mv CHANGELOG.new CHANGELOG.md
3. Commit `chore(release): X.Y.Z`, then
   `git tag vX.Y.Z && git push origin main vX.Y.Z`.
4. The Release workflow builds both packages and opens a draft release
   with them and this version's changelog. Edit the notes, publish.
5. `cargo publish --workspace --locked` publishes `tridi-formats`,
   `tridi-step`, `tridi` and `tridi-cli`, dependencies first. CI's dry run has already packaged and built
   them.
6. In `packaging/aur/PKGBUILD`, set `pkgver` and run `updpkgsums` (the
   tag's tarball now exists), commit `build(aur): X.Y.Z`.
