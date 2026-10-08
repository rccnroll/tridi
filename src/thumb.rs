//! `tridi thumb IN OUT SIZE`: what Nautilus runs, inside its sandbox.

use std::path::Path;
use std::time::Instant;
use tridi::{Input, Item, Scene, Theme};

pub fn thumb(inp: &str, out: &str, size: u32, theme: &Theme) -> Result<(), String> {
    let t0 = Instant::now();
    // Nautilus's sandbox clears the environment: without this glvnd would
    // also load NVIDIA's EGL, even for a user who limited it to Mesa
    let mesa = "/usr/share/glvnd/egl_vendor.d/50_mesa.json";
    if std::env::var_os("__EGL_VENDOR_LIBRARY_FILENAMES").is_none() && Path::new(mesa).exists() {
        // SAFETY: single-threaded, and before EGL is loaded
        unsafe { std::env::set_var("__EGL_VENDOR_LIBRARY_FILENAMES", mesa) };
    }
    let input = match tridi::load(Path::new(inp), theme.mesh).map_err(|e| format!("{inp}: {e}"))? {
        Item::Cloud(c) => {
            let colors = tridi::colorize(&c, None, theme);
            Input::Points(c.points, colors)
        }
        Item::Mesh(m) => Input::Mesh(m),
    };
    let (ctx, _keep) = tridi::headless()?;
    // no axis triad: at thumbnail size it's only noise
    let scene = Scene::new(&ctx, vec![input], false, tridi::up_for(tridi::is_gltf(Path::new(inp))))?;
    let img = tridi::offscreen(&ctx, &scene, size, theme.bg)?;
    image::save_buffer(out, &img, size, size, image::ExtendedColorType::Rgba8).map_err(|e| e.to_string())?;
    if std::env::var_os("TRIDI_TIMING").is_some() {
        eprintln!("thumb {:?}", t0.elapsed());
    }
    Ok(())
}
