# tridi-cli

A fast viewer for point clouds, meshes and STEP files on Linux, and the
thumbnails for them in the file manager (Nautilus, Nemo, Caja, Thunar).
One Rust binary does both: a window for looking at the files, and
`tridi thumb`, which the file manager calls to draw their previews without
a window or a GPU.

    cargo install tridi-cli

installs the `tridi` command. It builds the STEP side against OpenCASCADE,
so it needs its development packages (Linux only). The Arch and Debian
packages, which also set up the file manager thumbnails, are in the
[repository's README](https://github.com/rccnroll/tridi#readme).
