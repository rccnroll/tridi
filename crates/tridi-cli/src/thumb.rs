//! `tridi thumb IN OUT SIZE`: what Nautilus runs, inside its sandbox.

// ========================================== Imports ========================================== {{{

use eyre::WrapErr;
use std::{env, path::Path, time::Instant};
use tridi::Theme;

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
    let t = tridi::thumbnail(Path::new(inp), size, theme).wrap_err_with(|| inp.to_owned())?;
    if t.skipped > 0 {
        warn!("{inp}: {} solids skipped", t.skipped);
    }
    // PNG whatever `out` is called: yazi's cache files have no extension
    image::save_buffer_with_format(out, &t.rgba, size, size, image::ExtendedColorType::Rgba8, image::ImageFormat::Png)
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
        let inp = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/data/rgb_ascii.pcd");
        thumb(inp, out.to_str().unwrap(), 64, &tridi::LIGHT).unwrap();
        assert_eq!(&fs::read(&out).unwrap()[..8], b"\x89PNG\r\n\x1a\n");
    }
}

// }}}
