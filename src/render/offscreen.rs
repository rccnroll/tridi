//! The thumbnail: the scene drawn into textures and read back.

use three_d::*;

use crate::render::Scene;

/// Renders `scene` to a size×size RGBA image, top row first. Drawn at twice
/// the size and averaged down 2×2: anti-aliasing for points too, which MSAA
/// doesn't smooth.
pub fn offscreen(ctx: &Context, scene: &Scene, size: u32, bg: [f32; 3]) -> Result<Vec<u8>, String> {
    let big = size * 2;
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
    target.clear(ClearState::color_and_depth(bg[0], bg[1], bg[2], 1.0, 1.0));
    // points 2 px wide in the final image: at 1 px sparse scans show the gaps
    // between points, and eye-dome lighting outlines each one
    scene.render(&target, &cam, (big as f32 / 128.0).max(2.0));
    // three-d's read_color already returns the top row first: no flip
    let px = target.read_color::<[u8; 4]>();
    let (b, s) = (big as usize, size as usize);
    let mut out = Vec::with_capacity(s * s * 4);
    for y in 0..s {
        for x in 0..s {
            let at = |dx: usize, dy: usize| px[(2 * y + dy) * b + 2 * x + dx];
            for c in 0..4 {
                let sum: u32 = [at(0, 0), at(1, 0), at(0, 1), at(1, 1)].iter().map(|p| p[c] as u32).sum();
                out.push(((sum + 2) / 4) as u8);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{Input, headless, quiet_stderr};

    // a thumbnail on llvmpipe (no GPU needed): right size, not all
    // background, and not upside down (a heavy blob at the top, Z up)
    #[test]
    fn thumbnail_llvmpipe() {
        // SAFETY: tests in this binary don't read this variable concurrently
        unsafe { std::env::set_var("TRIDI_EGL_DEVICE", software_device().to_string()) };
        let (ctx, _keep) = headless().unwrap();
        let mut pts: Vec<Vec3> = (0..2000)
            .map(|i| vec3((i % 40) as f32 / 40.0 - 0.5, (i / 40) as f32 / 50.0 - 0.5, 1.0))
            .collect();
        pts.extend((0..20).map(|i| vec3(0.0, 0.0, i as f32 / 20.0)));
        let cols = vec![vec3(0.0, 0.0, 0.0); pts.len()];
        let scene = Scene::new(&ctx, vec![Input::Points(pts, cols)], false, Vec3::unit_z()).unwrap();
        let img = offscreen(&ctx, &scene, 64, [1.0, 1.0, 1.0]).unwrap();
        assert_eq!(img.len(), 64 * 64 * 4);
        let dark = |rows: &[u8]| rows.chunks(4).filter(|p| p[0] < 128).count();
        let (top, bottom) = img.split_at(img.len() / 2);
        assert!(dark(top) > 50, "only {} dark pixels on top", dark(top));
        assert!(dark(top) > dark(bottom), "upside down: top {} bottom {}", dark(top), dark(bottom));
    }

    fn software_device() -> usize {
        quiet_stderr(glutin::api::egl::device::Device::query_devices)
            .unwrap()
            .position(|d| d.extensions().contains("EGL_MESA_device_software"))
            .expect("no llvmpipe EGL device")
    }
}
