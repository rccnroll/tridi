//! A thumbnail without a window: what tridi-cli's `tridi thumb` draws.

// ========================================== Imports ========================================== {{{

use std::path::Path;

#[cfg(test)]
use crate::LIGHT;
use crate::{
    Input, Item, LoadError, Purpose, RenderError, Scene, Theme, colorize, is_gltf, load,
    render::{headless, offscreen},
    up_for,
};

// }}}

// ========================================== Errors =========================================== {{{

/// Why no thumbnail.
#[derive(Debug, thiserror::Error)]
pub enum ThumbnailError {
    #[error(transparent)]
    Load(#[from] LoadError),
    #[error(transparent)]
    Render(#[from] RenderError),
}

pub type ThumbnailResult<T> = Result<T, ThumbnailError>;

// }}}

// ========================================= Thumbnail ========================================= {{{

/// `size` x `size` RGBA pixels, top row first, and how many STEP solids
/// didn't make it in time.
#[derive(Debug)]
pub struct Thumbnail {
    pub rgba: Vec<u8>,
    pub skipped: usize,
}

/// Draws `path` the way the file manager shows it: coarse STEP, what came
/// in time, no axis triad.
pub fn thumbnail(path: &Path, size: u32, theme: &Theme) -> ThumbnailResult<Thumbnail> {
    // a STEP file's solids come one by one: the thumbnail is what came in time
    let mut inputs = vec![];
    let skipped = load(path, theme.mesh, Purpose::Thumbnail, &|| false, &mut |item| {
        inputs.push(match item {
            Item::Cloud(c) => {
                let colors = colorize(&c, None, theme);
                Input::Points(c.points, colors)
            }
            Item::Mesh(m) => Input::Mesh(m),
        });
    })?;
    let (ctx, _keep) = headless()?;
    // no axis triad: at thumbnail size it's only noise
    let scene = Scene::new(&ctx, inputs, false, up_for(is_gltf(path)))?;
    Ok(Thumbnail {
        rgba: offscreen(&ctx, &scene, size, theme.bg),
        skipped,
    })
}

// }}}

// =========================================== Tests =========================================== {{{

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::llvmpipe;

    // renders on llvmpipe without a display, as in CI
    #[test]
    fn draws_a_cloud() {
        llvmpipe();
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/data/rgb_ascii.pcd");
        let t = thumbnail(&p, 64, &LIGHT).unwrap();
        assert_eq!((t.rgba.len(), t.skipped), (64 * 64 * 4, 0));
        let bg = LIGHT.bg.map(|c| (c * 255.0).round());
        let (pixels, _) = t.rgba.as_chunks::<4>();
        let not_bg = pixels
            .iter()
            .any(|px| px.iter().zip(bg).any(|(&a, b)| (f32::from(a) - b).abs() > 8.0));
        assert!(not_bg, "something drawn");
    }
}

// }}}
