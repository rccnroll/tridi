//! `tridi thumb IN OUT SIZE`: what Nautilus runs, inside its sandbox.

use eyre::WrapErr;
use std::{env, path::Path, time::Instant};
use tridi::{Input, Item, Scene, Theme};

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
    let input = match tridi::load(Path::new(inp), theme.mesh).wrap_err_with(|| inp.to_owned())? {
        Item::Cloud(c) => {
            let colors = tridi::colorize(&c, None, theme);
            Input::Points(c.points, colors)
        }
        Item::Mesh(m) => Input::Mesh(m),
    };
    let (ctx, _keep) = tridi::headless()?;
    // no axis triad: at thumbnail size it's only noise
    let scene = Scene::new(&ctx, vec![input], false, tridi::up_for(tridi::is_gltf(Path::new(inp))))?;
    let img = tridi::offscreen(&ctx, &scene, size, theme.bg);
    image::save_buffer(out, &img, size, size, image::ExtendedColorType::Rgba8).wrap_err_with(|| format!("cannot write {out}"))?;
    let elapsed_ms = u64::try_from(t0.elapsed().as_millis()).unwrap_or(u64::MAX);
    info!(elapsed_ms, "thumbnail written");
    Ok(())
}
