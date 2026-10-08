//! The thumbnail: the scene drawn into textures and read back.

// ========================================== Imports ========================================== {{{

use three_d::{ClearState, Context, DepthTexture2D, Interpolation, RenderTarget, Texture2D, Viewport, Wrapping};

use crate::render::{Scene, px_f32};

// }}}

// ========================================= Offscreen ========================================= {{{

/// Renders `scene` to a size×size RGBA image, top row first. Drawn at twice
/// the size and averaged down 2×2: anti-aliasing for points too, which MSAA
/// doesn't smooth.
pub fn offscreen(ctx: &Context, scene: &Scene, size: u32, bg: [f32; 3]) -> Vec<u8> {
    let big = size.saturating_mul(2);
    let color = Texture2D::new_empty::<[u8; 4]>(
        ctx,
        big,
        big,
        Interpolation::Nearest,
        Interpolation::Nearest,
        None,
        Wrapping::ClampToEdge,
        Wrapping::ClampToEdge,
    );
    let depth = DepthTexture2D::new::<f32>(ctx, big, big, Wrapping::ClampToEdge, Wrapping::ClampToEdge);
    let cam = scene.camera(Viewport::new_at_origo(big, big));
    let target = RenderTarget::new(color.as_color_target(None), depth.as_depth_target());
    let [r, g, b] = bg;
    target.clear(ClearState::color_and_depth(r, g, b, 1.0, 1.0));
    // points 2 px wide in the final image: at 1 px sparse scans show the gaps
    // between points, and eye-dome lighting outlines each one
    scene.render(&target, &cam, (px_f32(big) / 128.0).max(2.0));
    // three-d's read_color already returns the top row first: no flip
    let pixels = target.read_color::<[u8; 4]>();
    let row = usize::try_from(big).unwrap_or(usize::MAX);
    let mut out = Vec::with_capacity(pixels.len());
    for rows in pixels.chunks_exact(row.saturating_mul(2)) {
        let (top, bottom) = rows.split_at(row);
        for (&[p0, p1], &[p2, p3]) in top.as_chunks::<2>().0.iter().zip(bottom.as_chunks::<2>().0) {
            for (((c0, c1), c2), c3) in p0.into_iter().zip(p1).zip(p2).zip(p3) {
                // the mean of four bytes, rounded: it fits a byte
                let sum = u16::from(c0) + u16::from(c1) + u16::from(c2) + u16::from(c3);
                out.push(u8::try_from((sum + 2) >> 2).unwrap_or(u8::MAX));
            }
        }
    }
    out
}

// }}}

// =========================================== Tests =========================================== {{{

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{Input, headless, quiet_stderr};
    use glutin::api::egl::device::Device;
    use std::env;
    use three_d::{Vec3, vec3};

    // a thumbnail on llvmpipe (no GPU needed): right size, not all
    // background, and not upside down (a heavy blob at the top, Z up)
    #[test]
    fn thumbnail_llvmpipe() {
        // SAFETY: tests in this binary don't read this variable concurrently
        unsafe { env::set_var("TRIDI_EGL_DEVICE", software_device().to_string()) };
        let (ctx, _keep) = headless().unwrap();
        let mut pts: Vec<Vec3> = (0..50_u16)
            .flat_map(|row| (0..40_u16).map(move |col| vec3(f32::from(col) / 40.0 - 0.5, f32::from(row) / 50.0 - 0.5, 1.0)))
            .collect();
        pts.extend((0..20_u16).map(|i| vec3(0.0, 0.0, f32::from(i) / 20.0)));
        let cols = vec![vec3(0.0, 0.0, 0.0); pts.len()];
        let scene = Scene::new(&ctx, vec![Input::Points(pts, cols)], false, Vec3::unit_z()).unwrap();
        let img = offscreen(&ctx, &scene, 64, [1.0, 1.0, 1.0]);
        assert_eq!(img.len(), 64 * 64 * 4);
        let dark = |rows: &[u8]| rows.chunks(4).filter(|p| p[0] < 128).count();
        let (top, bottom) = img.split_at(64 * 32 * 4);
        assert!(dark(top) > 50, "only {} dark pixels on top", dark(top));
        assert!(dark(top) > dark(bottom), "upside down: top {} bottom {}", dark(top), dark(bottom));
    }

    fn software_device() -> usize {
        quiet_stderr(Device::query_devices)
            .unwrap()
            .position(|d| d.extensions().contains("EGL_MESA_device_software"))
            .expect("no llvmpipe EGL device")
    }
}

// }}}
