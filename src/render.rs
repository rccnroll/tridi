//! Drawing: the scene (points + axis triad) and the headless GL context the
//! thumbnailer uses.

use std::sync::Arc;
use three_d::*;

const VS: &str = "
uniform mat4 mvp;
uniform float psize;
in vec3 pos;
in vec3 color;
out vec3 col;
void main() {
    gl_Position = mvp * vec4(pos, 1.0);
    gl_PointSize = psize;
    col = color;
}";
const FS: &str = "
in vec3 col;
layout (location = 0) out vec4 outColor;
void main() { outColor = vec4(col, 1.0); }";

// Eye-dome lighting (Boucheny 2009): points are drawn to their own color +
// depth textures, then composited with each pixel darkened by how much its
// neighbors are closer to the eye. Gives flat clouds their depth and
// outlines. The depth is written back, so meshes and points still hide each
// other correctly.
const EDL_VS: &str = "
in vec2 corner;
out vec2 uv;
void main() {
    uv = corner;
    gl_Position = vec4(corner * 2.0 - 1.0, 0.0, 1.0);
}";
const EDL_FS: &str = "
uniform sampler2D colorTex;
uniform sampler2D depthTex;
uniform float near;
uniform float far;
uniform vec2 px;
uniform float strength;
in vec2 uv;
layout (location = 0) out vec4 outColor;
float logz(float d) {
    float z = d * 2.0 - 1.0;
    return log2(2.0 * near * far / (far + near - z * (far - near)));
}
void main() {
    float d = texture(depthTex, uv).r;
    if (d >= 1.0) discard;
    float c = logz(d);
    float s = 0.0;
    for (int i = 0; i < 8; i++) {
        float a = float(i) * 0.785398;
        float n = texture(depthTex, uv + vec2(cos(a), sin(a)) * px).r;
        s += max(0.0, c - (n >= 1.0 ? log2(far) : logz(n)));
    }
    outColor = vec4(texture(colorTex, uv).rgb * exp(-strength * s), 1.0);
    gl_FragDepth = d;
}";

enum Layer {
    Points {
        pos: VertexBuffer<Vec3>,
        col: VertexBuffer<Vec3>,
        n: u32,
    },
    Mesh(Model<PhysicalMaterial>),
}

/// vertical field of view, degrees
pub const FOV: f32 = 45.0;

/// What goes on screen: one layer per opened file, plus the axis triad.
pub struct Scene {
    ctx: Context,
    program: Program,
    edl: Program,
    /// one triangle covering the screen (a real buffer: three-d unbinds the
    /// VAO after every draw, and only binding an attribute brings it back)
    edl_tri: VertexBuffer<Vec2>,
    /// the points' own color and depth textures, kept while the size holds
    edl_targets: std::cell::RefCell<Option<(u32, u32, Texture2D, DepthTexture2D)>>,
    layers: Vec<Layer>,
    visible: Vec<bool>,
    axes: Option<(VertexBuffer<Vec3>, VertexBuffer<Vec3>)>,
    lo: Vec3,
    hi: Vec3,
    pub center: Vec3,
    pub radius: f32,
    /// +Z for scans and CAD, +Y when every file is glTF (its convention)
    pub up: Vec3,
}

/// One opened file, ready for the GPU: a colored cloud or a mesh.
pub enum Input {
    Points(Vec<Vec3>, Vec<Vec3>),
    Mesh(CpuModel),
}

impl Scene {
    /// With `axes`, the triad sits at the min corner of the bbox (size 20% of
    /// the largest extent, like 1.0).
    pub fn new(ctx: &Context, inputs: Vec<Input>, axes: bool, up: Vec3) -> Result<Scene, String> {
        let mut bb = AxisAlignedBoundingBox::EMPTY;
        let mut layers = vec![];
        for i in inputs {
            layers.push(match i {
                Input::Points(p, c) => {
                    bb.expand(&p);
                    Layer::Points {
                        pos: VertexBuffer::new_with_data(ctx, &p),
                        col: VertexBuffer::new_with_data(ctx, &c),
                        n: p.len() as u32,
                    }
                }
                Input::Mesh(m) => {
                    let model = Model::<PhysicalMaterial>::new(ctx, &m).map_err(|e| e.to_string())?;
                    for part in model.iter() {
                        bb.expand_with_aabb(part.aabb());
                    }
                    Layer::Mesh(model)
                }
            });
        }
        let (lo, hi) = (bb.min(), bb.max());
        let ext = hi - lo;
        let axes = axes.then(|| {
            let size = match ext.x.max(ext.y).max(ext.z) * 0.2 {
                s if s > 0.0 => s,
                _ => 0.1,
            };
            let (mut p, mut c) = (vec![], vec![]);
            for axis in [Vec3::unit_x(), Vec3::unit_y(), Vec3::unit_z()] {
                p.extend([lo, lo + axis * size]);
                c.extend([axis, axis]);
            }
            (VertexBuffer::new_with_data(ctx, &p), VertexBuffer::new_with_data(ctx, &c))
        });
        unsafe { ctx.enable(context::PROGRAM_POINT_SIZE) };
        Ok(Scene {
            ctx: ctx.clone(),
            program: Program::from_source(ctx, VS, FS).map_err(|e| e.to_string())?,
            edl: Program::from_source(ctx, EDL_VS, EDL_FS).map_err(|e| e.to_string())?,
            edl_targets: Default::default(),
            edl_tri: VertexBuffer::new_with_data(ctx, &[vec2(0.0, 0.0), vec2(2.0, 0.0), vec2(0.0, 2.0)]),
            visible: vec![true; layers.len()],
            layers,
            axes,
            lo,
            hi,
            center: (lo + hi) * 0.5,
            radius: (ext.magnitude() * 0.5).max(1e-3),
            up,
        })
    }

    /// Turns layer `i` (the i-th file read) off or on; returns whether it's
    /// now visible, None if there's no such layer.
    pub fn toggle(&mut self, i: usize) -> Option<bool> {
        let v = self.visible.get_mut(i)?;
        *v = !*v;
        Some(*v)
    }

    /// Which layers are on, in file order.
    pub fn visible(&self) -> &[bool] {
        &self.visible
    }

    /// Three-quarter view from the front-right, above, as close as it can be
    /// with all eight bbox corners on screen (and a small margin).
    pub fn camera(&self, vp: Viewport) -> Camera {
        let dir = if self.up.y > 0.5 {
            vec3(1.0, 0.8, 1.0)
        } else {
            vec3(1.0, -1.0, 0.8)
        }
        .normalize();
        let right = self.up.cross(dir).normalize();
        let up = dir.cross(right);
        let ty = (FOV.to_radians() / 2.0).tan() / 1.08;
        let tx = ty * vp.width as f32 / vp.height.max(1) as f32;
        let mut d = self.radius * 0.1;
        for i in 0..8 {
            let pick = |bit: usize, lo: f32, hi: f32| if i & bit == 0 { lo } else { hi };
            let v = vec3(
                pick(1, self.lo.x, self.hi.x),
                pick(2, self.lo.y, self.hi.y),
                pick(4, self.lo.z, self.hi.z),
            ) - self.center;
            // the corner is at depth d - v·dir from the camera
            let z = v.dot(dir);
            d = d.max(z + v.dot(right).abs() / tx).max(z + v.dot(up).abs() / ty);
        }
        Camera::new_perspective(
            vp,
            self.center + dir * d,
            self.center,
            self.up,
            degrees(FOV),
            self.radius * 0.01,
            d + self.radius * 20.0,
        )
    }

    pub fn render(&self, target: &RenderTarget, cam: &Camera, psize: f32) {
        // headlight from above-left of the camera: faces turned different ways
        // get different shades (white parts would read flat with the light
        // straight from the eye), and nothing facing us is ever in the dark
        let ctx = &self.ctx;
        let ambient = AmbientLight::new(ctx, 0.35, Srgba::WHITE);
        let (view, up) = (cam.view_direction(), cam.up());
        let key = DirectionalLight::new(ctx, 1.6, Srgba::WHITE, view - up * 0.5 + view.cross(up) * 0.35);
        let shown = || self.layers.iter().zip(&self.visible).filter(|(_, v)| **v).map(|(l, _)| l);
        let meshes = shown().filter_map(|l| match l {
            Layer::Mesh(m) => Some(m),
            _ => None,
        });
        target.render(cam, meshes.flat_map(|m| m.into_iter()), &[&ambient, &key]);
        let p = &self.program;
        p.use_uniform("mvp", cam.projection() * cam.view());
        p.use_uniform("psize", psize);
        // three-d's Program only draws triangles: points and lines go straight to GL
        let draw = |pos: &VertexBuffer<Vec3>, col: &VertexBuffer<Vec3>, mode: u32, n: u32| {
            p.use_vertex_attribute("pos", pos);
            p.use_vertex_attribute("color", col);
            p.draw_with(RenderStates::default(), cam.viewport(), || unsafe {
                ctx.draw_arrays(mode, 0, n as i32)
            });
        };
        let has_points = shown().any(|l| matches!(l, Layer::Points { .. }));
        if has_points {
            let vp = cam.viewport();
            let mut cache = self.edl_targets.borrow_mut();
            if !matches!(&*cache, Some((w, h, ..)) if *w == vp.width && *h == vp.height) {
                let color = Texture2D::new_empty::<[u8; 4]>(
                    ctx,
                    vp.width,
                    vp.height,
                    Interpolation::Nearest,
                    Interpolation::Nearest,
                    None,
                    Wrapping::ClampToEdge,
                    Wrapping::ClampToEdge,
                );
                let depth = DepthTexture2D::new::<f32>(ctx, vp.width, vp.height, Wrapping::ClampToEdge, Wrapping::ClampToEdge);
                *cache = Some((vp.width, vp.height, color, depth));
            }
            let (_, _, color, depth) = cache.as_mut().unwrap();
            // the points' camera draws at the origin of their own textures
            let mut own = cam.clone();
            own.set_viewport(Viewport::new_at_origo(vp.width, vp.height));
            RenderTarget::new(color.as_color_target(None), depth.as_depth_target())
                .clear(ClearState::color_and_depth(0.0, 0.0, 0.0, 0.0, 1.0))
                .write::<RendererError>(|| {
                    p.use_uniform("mvp", own.projection() * own.view());
                    for l in shown() {
                        if let Layer::Points { pos, col, n } = l {
                            p.use_vertex_attribute("pos", pos);
                            p.use_vertex_attribute("color", col);
                            let n = *n as i32;
                            p.draw_with(RenderStates::default(), own.viewport(), || unsafe {
                                ctx.draw_arrays(context::POINTS, 0, n)
                            });
                        }
                    }
                    Ok(())
                })
                .unwrap();
            let e = &self.edl;
            e.use_texture("colorTex", color);
            e.use_depth_texture("depthTex", depth);
            e.use_uniform("near", cam.z_near());
            e.use_uniform("far", cam.z_far());
            // sample radius ~1.5 px at 1080p, scaled with the image
            let r = (vp.height as f32 / 720.0).max(1.0);
            e.use_uniform("px", vec2(r / vp.width as f32, r / vp.height as f32));
            e.use_uniform("strength", 6.0f32);
            e.use_vertex_attribute("corner", &self.edl_tri);
            target
                .write::<RendererError>(|| {
                    e.draw_arrays(
                        RenderStates {
                            depth_test: DepthTest::LessOrEqual,
                            ..Default::default()
                        },
                        vp,
                        3,
                    );
                    Ok(())
                })
                .unwrap();
        }
        target
            .write::<RendererError>(|| {
                p.use_uniform("mvp", cam.projection() * cam.view());
                if let Some((pos, col)) = &self.axes {
                    draw(pos, col, context::LINES, 6);
                }
                Ok(())
            })
            .unwrap();
    }
}

/// Runs `f` with stderr closed. Mesa prints `pci id for fd N: 10de:…, driver
/// (null)` for every GPU node it has no driver for (the NVIDIA one) while
/// EGL starts, before any of its log settings apply. Real failures still come
/// back through `f`'s result.
pub fn quiet_stderr<T>(f: impl FnOnce() -> T) -> T {
    if std::env::var_os("TRIDI_DEBUG").is_some() {
        return f();
    }
    // SAFETY: plain fd juggling on 2; the saved copy is restored and closed
    unsafe {
        let saved = libc::dup(2);
        let null = libc::open(c"/dev/null".as_ptr(), libc::O_WRONLY);
        if saved < 0 || null < 0 {
            return f();
        }
        libc::dup2(null, 2);
        libc::close(null);
        let r = f();
        libc::dup2(saved, 2);
        libc::close(saved);
        r
    }
}

/// GL context with no window and no display: EGL device + surfaceless.
/// The second value keeps the EGL context and display alive.
pub fn headless() -> Result<(Context, impl Sized), String> {
    quiet_stderr(open_headless)
}

fn open_headless() -> Result<(Context, impl Sized), String> {
    use glutin::api::egl::{device::Device, display::Display};
    use glutin::config::{ConfigSurfaceTypes, ConfigTemplateBuilder};
    use glutin::context::{ContextApi, ContextAttributesBuilder, Version};
    use glutin::prelude::*;
    let devs: Vec<Device> = Device::query_devices().map_err(|e| e.to_string())?.collect();
    let software = |d: &Device| d.extensions().contains("EGL_MESA_device_software");
    // a GPU with a name that isn't NVIDIA (waking the dGPU costs ~2.7 s),
    // else llvmpipe; a device without a name is a node Mesa has no driver for
    let gpu = |d: &Device| {
        d.vendor().is_some_and(|v| !v.to_ascii_lowercase().contains("nvidia"))
            && !software(d)
            && !d.extensions().contains("EGL_NV_device_cuda")
    };
    let dev = match std::env::var("TRIDI_EGL_DEVICE").ok().and_then(|s| s.parse::<usize>().ok()) {
        Some(i) => devs.get(i),
        None => devs.iter().find(|d| gpu(d)).or(devs.iter().find(|d| software(d))),
    }
    .ok_or("no usable EGL device")?;
    if std::env::var_os("TRIDI_DEBUG").is_some() {
        for (i, d) in devs.iter().enumerate() {
            let tag = if std::ptr::eq(d, dev) { "  <- used" } else { "" };
            eprintln!("egl device {i}: {:?} {:?} software={}{tag}", d.vendor(), d.name(), software(d));
        }
    }
    let display = unsafe { Display::with_device(dev, None) }.map_err(|e| e.to_string())?;
    let tmpl = ConfigTemplateBuilder::new().with_surface_type(ConfigSurfaceTypes::empty()).build();
    let config = unsafe { display.find_configs(tmpl) }
        .map_err(|e| e.to_string())?
        .next()
        .ok_or("no EGL config")?;
    let attrs = ContextAttributesBuilder::new()
        .with_context_api(ContextApi::OpenGl(Some(Version::new(3, 3))))
        .build(None);
    let gl_ctx = unsafe { display.create_context(&config, &attrs) }
        .map_err(|e| e.to_string())?
        .make_current_surfaceless()
        .map_err(|e| e.to_string())?;
    let gl = unsafe { context::Context::from_loader_function_cstr(|s| display.get_proc_address(s)) };
    let ctx = Context::from_gl_context(Arc::new(gl)).map_err(|e| e.to_string())?;
    Ok((ctx, (gl_ctx, display)))
}

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
