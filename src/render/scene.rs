//! The scene: one layer per opened file (points with eye-dome lighting, or
//! a mesh) and the axis triad.

use three_d::*;

use crate::render::{RenderError, RenderResult};

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
    pub fn new(ctx: &Context, inputs: Vec<Input>, axes: bool, up: Vec3) -> RenderResult<Scene> {
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
                    let model = Model::<PhysicalMaterial>::new(ctx, &m).map_err(RenderError::Mesh)?;
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
            program: Program::from_source(ctx, VS, FS).map_err(|source| RenderError::Gl {
                step: "build the point shader",
                source,
            })?,
            edl: Program::from_source(ctx, EDL_VS, EDL_FS).map_err(|source| RenderError::Gl {
                step: "build the eye-dome shader",
                source,
            })?,
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
