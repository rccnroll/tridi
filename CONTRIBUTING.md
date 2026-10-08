# Contributing

Bugs and ideas go in the issues; the templates ask for what's needed to
reproduce. A pull request is welcome for anything already in
[ROADMAP.md](ROADMAP.md), or after an issue for anything else.

## Build and test

    cargo build --release
    cargo run --release -- FILE
    cargo test

One test renders a thumbnail offscreen: with no GPU it runs on Mesa's
llvmpipe (`libegl-mesa0` and `libgl1-mesa-dri` on Debian/Ubuntu, `mesa` on
Arch), as in CI.

Before pushing, what CI checks on every push and PR:

    cargo fmt --check
    cargo clippy --all-targets --locked -- -D warnings
    cargo test --locked

plus `cargo check` on the oldest Rust supported, 1.92 (`rust-version` in
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

The crate is a library (`src/lib.rs`: readers, scene, rendering) and a
binary on top of it (`src/main.rs`: the window, the thumbnailer command,
the desktop integration). Dependencies go one way, down this list:

- `src/formats/` — the point cloud readers, one file per format (pcd, ply,
  off, xyz/xyzrgb/pts). Every byte is untrusted, and nothing here uses the
  rest of tridi, so the fuzz targets include this folder alone.
- `src/step/` — STEP: the child process with its deadline (`mod.rs`),
  monstertruck's tessellation and our assemblies (`tessellate.rs`), the
  colors read from the file (`styles.rs`).
- `src/model.rs` — what a file is: a cloud, or a mesh (three-d-asset,
  ply/off faces, STEP), and which way is up.
- `src/color.rs`, `src/theme.rs` — the coloring rules and the two themes.
- `src/render/` — the scene with eye-dome lighting, the headless EGL
  context on the right GPU, the offscreen thumbnail.
- `src/view.rs`, `src/thumb.rs`, `src/desktop.rs` — the binary's parts:
  the window (navigation, keys, legend), `tridi thumb`, and `tridi theme`
  with the thumbnail cache.
- `share/` — what the packages install: thumbnailer entry, MIME types,
  `.desktop`, AppStream metainfo, icons. Under `Papirus/` only symlinks to
  our mimetype icon: GTK looks for every name of a file's icon, generic
  ones included, in the current theme before hicolor, so without them
  Papirus users get its generic file icon.
- `packaging/` — the PKGBUILD and the `.deb` build (cargo-deb, metadata in
  `Cargo.toml`) with their install scripts, and the licenses of the fonts
  egui builds into the binary.
- `tests/data/` — tiny fixtures: PCD and GLB written by Open3D, STEP by
  monstertruck.
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
`fuzz/artifacts/`; it becomes a test in `src/formats/`, next to the reader.

## Commits

One line, [Conventional Commits](https://www.conventionalcommits.org),
no body: `feat(viewer): ...`, `fix(cloud): ...`, `docs(readme): ...`. The
changelog and the release notes are generated from them, so the line is
what a user reads. The why goes in docs/history.md (decided) or
ROADMAP.md (open), not in the commit.

## Releases

1. Set the version in `Cargo.toml` and the PKGBUILD's `pkgver`, then
   `cargo check` (for `Cargo.lock`).
2. Add this release under the changelog's header (`--prepend` would
   repeat the header, and regenerating the whole file would empty 2.0.0,
   where the history starts):

       { sed -n 1,5p CHANGELOG.md; git cliff --unreleased --tag vX.Y.Z --strip header; sed -n '6,$p' CHANGELOG.md; } > CHANGELOG.new
       mv CHANGELOG.new CHANGELOG.md
3. Commit `chore(release): X.Y.Z`, then
   `git tag vX.Y.Z && git push origin main vX.Y.Z`.
4. The Release workflow builds both packages and opens a draft release
   with them and this version's changelog. Edit the notes, publish.
