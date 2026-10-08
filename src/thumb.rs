//! `tridi thumb IN OUT SIZE`: what Nautilus runs, inside its sandbox.

// ========================================== Imports ========================================== {{{

use eyre::WrapErr;
use std::{
    env,
    path::Path,
    time::{Duration, Instant},
};
use tridi::{Input, Item, Scene, Theme};

// }}}

// ========================================= Constants ========================================= {{{

/// The wait for a STEP file's solids: the file manager waits on us.
const BUDGET: Duration = Duration::from_secs(15);

// }}}

// ========================================= Thumbnail ========================================= {{{

/// Draws `inp` into the PNG `out`, `size` pixels square.
#[instrument(skip(theme), fields(theme = theme.name))]
pub fn thumb(inp: &str, out: &str, size: u32, theme: &Theme) -> eyre::Result<()> {
    let t0 = Instant::now();
    // Nautilus's sandbox clears the environment: without this glvnd would
    // also load NVIDIA's EGL, even for a user who limited it to Mesa
    let mesa = "/usr/share/glvnd/egl_vendor.d/50_mesa.json";
    if env::var_os("__EGL_VENDOR_LIBRARY_FILENAMES").is_none() && Path::new(mesa).exists() {
        // SAFETY: single-threaded, and before EGL is loaded
        unsafe { env::set_var("__EGL_VENDOR_LIBRARY_FILENAMES", mesa) };
    }
    // a STEP file's solids come one by one: the thumbnail is what came in BUDGET
    let mut inputs = vec![];
    let skipped = tridi::load(Path::new(inp), theme.mesh, Some(BUDGET), &mut |item| {
        inputs.push(match item {
            Item::Cloud(c) => {
                let colors = tridi::colorize(&c, None, theme);
                Input::Points(c.points, colors)
            }
            Item::Mesh(m) => Input::Mesh(m),
        });
    })
    .wrap_err_with(|| inp.to_owned())?;
    if skipped > 0 {
        warn!("{inp}: {skipped} solids skipped");
    }
    let (ctx, _keep) = tridi::headless()?;
    // no axis triad: at thumbnail size it's only noise
    let scene = Scene::new(&ctx, inputs, false, tridi::up_for(tridi::is_gltf(Path::new(inp))))?;
    let img = tridi::offscreen(&ctx, &scene, size, theme.bg);
    // PNG whatever `out` is called: yazi's cache files have no extension
    image::save_buffer_with_format(out, &img, size, size, image::ExtendedColorType::Rgba8, image::ImageFormat::Png)
        .wrap_err_with(|| format!("cannot write {out}"))?;
    let elapsed_ms = u64::try_from(t0.elapsed().as_millis()).unwrap_or(u64::MAX);
    info!(elapsed_ms, "thumbnail written");
    Ok(())
}

// }}}

// =========================================== Tests =========================================== {{{

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, process};

    // yazi's cache paths have no extension
    #[test]
    fn writes_png_without_extension() {
        let dir = env::temp_dir().join(format!("tridi-thumb-{}", process::id()));
        fs::create_dir_all(&dir).unwrap();
        let out = dir.join("cache-entry");
        // not a STEP file: those run `current_exe() step-mesh`, the test binary under cargo test
        let inp = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/rgb_ascii.pcd");
        thumb(inp, out.to_str().unwrap(), 64, &tridi::LIGHT).unwrap();
        assert_eq!(&fs::read(&out).unwrap()[..8], b"\x89PNG\r\n\x1a\n");
    }
}

// }}}
