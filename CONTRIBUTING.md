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

plus `cargo check` on the oldest Rust supported, 1.88 (`rust-version` in
`Cargo.toml`).

The files in `tests/data/` are tiny and made for the tests: no real scans
or CAD from anyone's work. The README's screenshots and GIFs come from
synthetic files too (`python3 docs/samples.py`, recorded with
`docs/record.sh`).

## Fuzzing

The point cloud parsers read whatever file Nautilus lists, so they are
fuzzed: `cargo install cargo-fuzz`, then from `fuzz/`,
`cargo +nightly fuzz run -O pcd` (or `ply`, `off`, `text`), seeded from
`fuzz/seeds/`. A crash lands in `fuzz/artifacts/`; it becomes a test in
`src/cloud.rs`.

## Commits

One line, [Conventional Commits](https://www.conventionalcommits.org),
no body: `feat(viewer): ...`, `fix(cloud): ...`, `docs(readme): ...`. The
changelog and the release notes are generated from them, so the line is
what a user reads. The why goes in ROADMAP.md, not in the commit.

## Releases

1. Set the version in `Cargo.toml` and the PKGBUILD's `pkgver`, then
   `cargo check` (for `Cargo.lock`).
2. Regenerate the changelog:
   `git cliff v1.0.0.. --tag vX.Y.Z -o CHANGELOG.md`.
3. Commit `chore(release): X.Y.Z`, then
   `git tag vX.Y.Z && git push origin main vX.Y.Z`.
4. The Release workflow builds both packages and opens a draft release
   with them and this version's changelog. Edit the notes, publish.
