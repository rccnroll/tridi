# tridi

A window for looking at point clouds, meshes and STEP files from Rust, on Linux:
Open3D's `draw_geometries`, as a library. The binary and the file manager
thumbnails are in [tridi-cli](https://github.com/rccnroll/tridi).

    tridi::show(["scan.pcd", "part.step"])?;

`show()` blocks until the window is closed. To show clouds a program computed,
or to set the theme, use `Viewer`:

    let helix: Vec<[f32; 3]> = (0..2000_u16)
        .map(|i| {
            let t = f32::from(i) / 2000.0;
            [(37.7 * t).cos(), (37.7 * t).sin(), 3.0 * t]
        })
        .collect();
    tridi::Viewer::new()
        .file("scan.pcd")
        .cloud("helix", &helix, None)
        .show()?;

A cloud's colors, if given, must have one per point; `show()` returns an error
otherwise, before opening a window. `cargo run -p tridi --example show -- FILE...`
runs a longer version of this.

Files load in parallel and show up as they are read. The window has a legend,
one color per file, and the keys listed in `tridi::KEYS`.

winit allows one event loop per process: windows must be shown from the thread
that showed the first one. `show()` blocks that thread until its window closes.

## STEP

STEP files open through a helper process embedded in the crate: there is
nothing to install and nothing to call. The `step` feature, on by default,
builds the helper and needs OpenCASCADE 7.6 or later, headers and libraries:
`opencascade` on Arch and Fedora, `libocct-data-exchange-dev
libocct-ocaf-dev libocct-modeling-algorithms-dev libocct-modeling-data-dev
libocct-foundation-dev` on Debian/Ubuntu. Elsewhere, point `OCCT_INCLUDE_DIR`
and `OCCT_LIB_DIR` at them. With `default-features = false` there is no STEP
and no OpenCASCADE:

    tridi = { version = "3", default-features = false }

## Reading without a window

[tridi-formats](https://crates.io/crates/tridi-formats) reads the point
cloud formats, PCD, PLY, OFF and XYZ/XYZRGB/PTS, into points (with the faces
of a PLY or OFF mesh), with no GL and no display.
