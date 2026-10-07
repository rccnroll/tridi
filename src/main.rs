//! tridi - minimal viewer for point clouds (pcd, ply, xyz, xyzrgb, pts) and
//! meshes (glb, gltf, obj, stl, off, ply with faces, step/stp).
//!
//!     tridi [--theme T] FILE [FILE ...]        window
//!     tridi thumb [--theme T] IN OUT SIZE      PNG thumbnail, no window (for Nautilus)
//!     tridi theme [light|dark]                 show or switch the theme (viewer and thumbnails)
//!     tridi clear-thumbnails [DIR ...]         drop cached thumbnails of our formats
//!
//! T is `light` or `dark`, both Nord. The viewer uses the one `tridi theme`
//! saved (light if none); thumbnails get theirs from the thumbnailer entry.
//!
//! In the window: left drag orbits, right or middle drag (or shift + left)
//! pans, the wheel zooms; R resets the view, + and - change the point size,
//! 1-9 turn the N-th file off and on, Q or Esc quits.
//!
//! With several files each cloud gets its own color, to tell them apart at a
//! glance; with a single file the cloud is colored by Z height. In both cases
//! only if the cloud doesn't already carry its own colors. Meshes keep their
//! materials.

mod cloud;
mod mesh;
mod render;
mod step;
mod theme;

use mesh::Item;
use render::Input;
use std::path::Path;
use std::time::Instant;
use three_d::*;

/// The Wayland app_id the dock groups the window by, matching the .desktop.
const APP_ID: &str = "tridi";
const USAGE: &str = "usage: tridi [--theme light|dark] FILE [FILE ...]
       tridi thumb [--theme light|dark] IN OUT SIZE
       tridi theme [light|dark]
       tridi clear-thumbnails [DIR ...]";

fn is_gltf(p: &str) -> bool {
    let ext = Path::new(p).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    ext == "glb" || ext == "gltf"
}

fn extent(pts: impl Iterator<Item = Vec3>) -> Vec3 {
    let mut bb = AxisAlignedBoundingBox::EMPTY;
    bb.expand(&pts.collect::<Vec<_>>());
    bb.max() - bb.min()
}

/// Loads every file; one that fails is reported and skipped. None if nothing
/// could be read. Returns the scene inputs, the names of the files read (in
/// the same order) and the up axis: +Y when every file is glTF, +Z otherwise.
fn load_all(paths: &[String], theme: &theme::Theme) -> Option<(Vec<Input>, Vec<String>, Vec3)> {
    let (mut inputs, mut names, mut all_gltf) = (vec![], vec![], true);
    for (i, p) in paths.iter().enumerate() {
        let path = Path::new(p);
        let name = path.file_name().map_or(p.clone(), |n| n.to_string_lossy().into_owned());
        let item = match mesh::load(path, theme.mesh) {
            Ok(it) => it,
            Err(e) => {
                eprintln!("[tridi] {name}: {e}");
                continue;
            }
        };
        all_gltf &= is_gltf(p);
        names.push(name.clone());
        match item {
            Item::Cloud(c) => {
                let tint = (paths.len() > 1).then(|| theme.palette[i % theme.palette.len()]);
                let e = extent(c.points.iter().copied());
                let tag = match tint {
                    Some(t) if cloud::own_colors(&c, tint).is_none() => format!("  color [{:.2}, {:.2}, {:.2}]", t[0], t[1], t[2]),
                    _ => String::new(),
                };
                println!(
                    "{name}: {} points, extent [{:.3}, {:.3}, {:.3}]{tag}",
                    c.points.len(),
                    e.x,
                    e.y,
                    e.z
                );
                let colors = cloud::colorize(&c, tint, theme);
                inputs.push(Input::Points(c.points, colors));
            }
            Item::Mesh(m) => {
                // ponytail: no per-file tint on meshes (1.0 didn't have one either)
                let e = extent(m.geometries.iter().flat_map(|g| {
                    match &g.geometry {
                        three_d_asset::Geometry::Triangles(t) => t
                            .positions
                            .to_f32()
                            .into_iter()
                            .map(|v| (g.transformation * v.extend(1.0)).truncate())
                            .collect(),
                        _ => vec![],
                    }
                }));
                println!(
                    "{name}: {} triangles, extent [{:.3}, {:.3}, {:.3}]",
                    mesh::triangles(&m),
                    e.x,
                    e.y,
                    e.z
                );
                inputs.push(Input::Mesh(m));
            }
        }
    }
    (!inputs.is_empty()).then(|| (inputs, names, up_for(all_gltf)))
}

/// glTF is Y-up by its standard (decided 25/09: we follow it, even though
/// Open3D writes scan GLBs Z-up); scans and CAD are Z-up.
fn up_for(gltf: bool) -> Vec3 {
    if gltf { Vec3::unit_y() } else { Vec3::unit_z() }
}

/// Mouse navigation around the camera's target: left drag orbits, right or
/// middle drag (or shift + left) pans, the wheel zooms. three-d's
/// OrbitControl has no pan and a fixed target, so it's done here.
fn navigate(cam: &mut Camera, events: &mut [Event], dpr: f32, min: f32, max: f32) {
    for e in events.iter_mut() {
        match e {
            Event::MouseMotion {
                button: Some(b),
                delta,
                modifiers,
                handled,
                ..
            } if !*handled => {
                let pan = matches!(b, MouseButton::Right | MouseButton::Middle) || modifiers.shift;
                if pan {
                    // world units per logical pixel at the target's depth
                    let dist = cam.position().distance(cam.target());
                    let k = 2.0 * dist * (render::FOV.to_radians() / 2.0).tan() * dpr / cam.viewport().height as f32;
                    let right = cam.right_direction();
                    let up = right.cross(cam.view_direction()).normalize();
                    cam.translate((up * delta.1 - right * delta.0) * k);
                } else if *b == MouseButton::Left {
                    let t = cam.target();
                    cam.rotate_around_with_fixed_up(t, 0.008 * delta.0, 0.008 * delta.1);
                }
                *handled = true;
            }
            Event::MouseWheel { delta, handled, .. } if !*handled => {
                let (t, dist) = (cam.target(), cam.position().distance(cam.target()));
                cam.zoom_towards(t, dist * (1.0 - (-delta.1 * 0.01).exp()), min, max);
                *handled = true;
            }
            _ => {}
        }
    }
}

fn view(paths: &[String], theme: &'static theme::Theme) -> Result<(), String> {
    let (inputs, names, up) = load_all(paths, theme).ok_or("")?;
    if names.len() > 1 {
        let keys: Vec<String> = names.iter().enumerate().take(9).map(|(i, n)| format!("{} {n}", i + 1)).collect();
        println!("keys: {}", keys.join(", "));
    }
    let event_loop = winit::event_loop::EventLoop::new().map_err(|e| format!("no display: {e}"))?;
    let mut v = Viewer {
        names,
        theme,
        inputs: Some(inputs),
        up,
        gl: None,
        err: None,
        psize: 2.0,
        cursor: None,
        button: None,
        mods: Modifiers::default(),
    };
    event_loop.run_app(&mut v).map_err(|e| e.to_string())?;
    v.err.map_or(Ok(()), Err)
}

/// The window, its GL surface and what's drawn in it: made once the event
/// loop starts, as winit 0.30 wants.
struct Gl {
    scene: render::Scene,
    cam: Camera,
    // ponytail: zoom bounds fixed at load, from the scene's radius
    min: f32,
    max: f32,
    context: Context,
    ctx: glutin::context::PossiblyCurrentContext,
    surface: glutin::surface::Surface<glutin::surface::WindowSurface>,
    window: winit::window::Window,
}

/// Our own window (winit 0.30 + glutin) instead of three-d's, which pins
/// winit 0.28: on GNOME, where winit draws the title bar itself, at scale 2
/// that one sends a 45 px high buffer and the compositor kills the window.
/// three-d only gets the GL context.
struct Viewer {
    names: Vec<String>,
    theme: &'static theme::Theme,
    inputs: Option<Vec<Input>>,
    up: Vec3,
    gl: Option<Gl>,
    err: Option<String>,
    psize: f32,
    /// last cursor position, logical pixels
    cursor: Option<(f32, f32)>,
    button: Option<MouseButton>,
    mods: Modifiers,
}

impl Viewer {
    fn open(&mut self, el: &winit::event_loop::ActiveEventLoop) -> Result<Gl, String> {
        use glutin::display::GetGlDisplay;
        use glutin::prelude::*;
        use glutin_winit::GlWindow;
        use winit::platform::wayland::WindowAttributesExtWayland;
        use winit::raw_window_handle::HasWindowHandle;
        let attrs = winit::window::Window::default_attributes()
            // the app_id the dock groups the window by, matching the .desktop
            .with_title(format!("{APP_ID} — {}", self.names.join(", ")))
            .with_name(APP_ID, APP_ID);
        // 4x MSAA if there is one, as three-d's window had
        let tmpl = glutin::config::ConfigTemplateBuilder::new().with_depth_size(24);
        let (window, config) = render::quiet_stderr(|| {
            glutin_winit::DisplayBuilder::new()
                .with_window_attributes(Some(attrs))
                .build(el, tmpl, |cs| cs.max_by_key(|c| c.num_samples().min(4)).expect("no GL config"))
        })
        .map_err(|e| e.to_string())?;
        let window = window.ok_or("no window")?;
        let display = config.display();
        let handle = window.window_handle().map_err(|e| e.to_string())?.as_raw();
        let attrs = glutin::context::ContextAttributesBuilder::new().build(Some(handle));
        let sattrs = window.build_surface_attributes(Default::default()).map_err(|e| e.to_string())?;
        let (ctx, surface) = unsafe {
            let ctx = display.create_context(&config, &attrs).map_err(|e| e.to_string())?;
            let surface = display.create_window_surface(&config, &sattrs).map_err(|e| e.to_string())?;
            (ctx.make_current(&surface).map_err(|e| e.to_string())?, surface)
        };
        let vsync = glutin::surface::SwapInterval::Wait(std::num::NonZeroU32::MIN);
        surface.set_swap_interval(&ctx, vsync).ok();
        let gl = unsafe { context::Context::from_loader_function_cstr(|s| display.get_proc_address(s)) };
        let context = Context::from_gl_context(std::sync::Arc::new(gl)).map_err(|e| e.to_string())?;
        if std::env::var_os("TRIDI_DEBUG").is_some() {
            eprintln!("GL: {}", unsafe { context.get_parameter_string(context::RENDERER) });
        }
        let scene = render::Scene::new(&context, self.inputs.take().unwrap_or_default(), true, self.up)?;
        let cam = scene.camera(viewport(&window));
        // min above the near plane (radius * 0.01), or zooming in clips everything
        let (min, max) = (scene.radius * 0.05, scene.radius * 15.0);
        Ok(Gl {
            scene,
            cam,
            min,
            max,
            context,
            ctx,
            surface,
            window,
        })
    }
}

fn viewport(w: &winit::window::Window) -> Viewport {
    let s = w.inner_size();
    Viewport::new_at_origo(s.width.max(1), s.height.max(1))
}

impl winit::application::ApplicationHandler for Viewer {
    fn resumed(&mut self, el: &winit::event_loop::ActiveEventLoop) {
        if self.gl.is_some() {
            return;
        }
        match self.open(el) {
            Ok(gl) => self.gl = Some(gl),
            Err(e) => {
                self.err = Some(e);
                el.exit();
            }
        }
    }

    fn window_event(&mut self, el: &winit::event_loop::ActiveEventLoop, _: winit::window::WindowId, ev: winit::event::WindowEvent) {
        use glutin::prelude::*;
        use glutin_winit::GlWindow;
        use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
        use winit::keyboard::{Key as K, NamedKey};
        let Some(gl) = self.gl.as_mut() else { return };
        let dpr = gl.window.scale_factor() as f32;
        let mut events = vec![];
        match ev {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(_) => gl.window.resize_surface(&gl.surface, &gl.ctx),
            WindowEvent::ModifiersChanged(m) => {
                let m = m.state();
                self.mods = Modifiers {
                    alt: m.alt_key(),
                    ctrl: m.control_key(),
                    shift: m.shift_key(),
                    command: m.control_key(),
                };
            }
            WindowEvent::MouseInput { state, button, .. } => {
                self.button = (state == ElementState::Pressed)
                    .then_some(match button {
                        winit::event::MouseButton::Left => Some(MouseButton::Left),
                        winit::event::MouseButton::Right => Some(MouseButton::Right),
                        winit::event::MouseButton::Middle => Some(MouseButton::Middle),
                        _ => None,
                    })
                    .flatten();
            }
            WindowEvent::CursorMoved { position, .. } => {
                let p = position.to_logical::<f32>(dpr as f64);
                if let (Some((x, y)), Some(b)) = (self.cursor, self.button) {
                    let position = PhysicalPoint {
                        x: position.x as f32,
                        y: position.y as f32,
                    };
                    events.push(Event::MouseMotion {
                        button: Some(b),
                        delta: (p.x - x, p.y - y),
                        position,
                        modifiers: self.mods,
                        handled: false,
                    });
                }
                self.cursor = Some((p.x, p.y));
            }
            WindowEvent::MouseWheel { delta, .. } => {
                // the same scale three-d's window gave: 24 per wheel notch
                let y = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y * 24.0,
                    MouseScrollDelta::PixelDelta(d) => d.to_logical::<f32>(dpr as f64).y * 0.24,
                };
                events.push(Event::MouseWheel {
                    delta: (0.0, y),
                    position: PhysicalPoint { x: 0.0, y: 0.0 },
                    modifiers: self.mods,
                    handled: false,
                });
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => match event.logical_key.as_ref() {
                K::Named(NamedKey::Escape) => el.exit(),
                K::Character(c) => match c.to_lowercase().as_str() {
                    "r" => gl.cam = gl.scene.camera(viewport(&gl.window)),
                    "+" | "=" => self.psize = f32::min(self.psize * 1.25, 20.0),
                    "-" => self.psize = f32::max(self.psize / 1.25, 1.0),
                    "q" => el.exit(),
                    d => {
                        if let Some(i) = d.parse::<usize>().ok().filter(|n| (1..=9).contains(n)).map(|n| n - 1)
                            && let Some(on) = gl.scene.toggle(i)
                        {
                            println!("{}: {}", self.names[i], if on { "on" } else { "off" });
                        }
                    }
                },
                _ => return,
            },
            WindowEvent::RedrawRequested => {
                let vp = viewport(&gl.window);
                gl.cam.set_viewport(vp);
                let screen = RenderTarget::screen(&gl.context, vp.width, vp.height);
                let bg = self.theme.bg;
                screen.clear(ClearState::color_and_depth(bg[0], bg[1], bg[2], 1.0, 1.0));
                gl.scene.render(&screen, &gl.cam, self.psize * dpr);
                if let Err(e) = gl.surface.swap_buffers(&gl.ctx) {
                    self.err = Some(e.to_string());
                    el.exit();
                }
                return;
            }
            _ => return,
        }
        navigate(&mut gl.cam, &mut events, dpr, gl.min, gl.max);
        gl.window.request_redraw();
    }
}

fn thumb(inp: &str, out: &str, size: &str, theme: &theme::Theme) -> Result<(), String> {
    let t0 = Instant::now();
    let size: u32 = size.parse().ok().filter(|s| (1..=4096).contains(s)).ok_or("SIZE must be 1..4096")?;
    // Nautilus's sandbox clears the environment: without this glvnd would
    // also load NVIDIA's EGL, even for a user who limited it to Mesa
    let mesa = "/usr/share/glvnd/egl_vendor.d/50_mesa.json";
    if std::env::var_os("__EGL_VENDOR_LIBRARY_FILENAMES").is_none() && Path::new(mesa).exists() {
        // SAFETY: single-threaded, and before EGL is loaded
        unsafe { std::env::set_var("__EGL_VENDOR_LIBRARY_FILENAMES", mesa) };
    }
    let input = match mesh::load(Path::new(inp), theme.mesh)? {
        Item::Cloud(c) => {
            let colors = cloud::colorize(&c, None, theme);
            Input::Points(c.points, colors)
        }
        Item::Mesh(m) => Input::Mesh(m),
    };
    let (ctx, _keep) = render::headless()?;
    // no axis triad: at thumbnail size it's only noise
    let scene = render::Scene::new(&ctx, vec![input], false, up_for(is_gltf(inp)))?;
    let img = render::offscreen(&ctx, &scene, size, theme.bg)?;
    image::save_buffer(out, &img, size, size, image::ExtendedColorType::Rgba8).map_err(|e| e.to_string())?;
    if std::env::var_os("TRIDI_TIMING").is_some() {
        eprintln!("thumb {:?}", t0.elapsed());
    }
    Ok(())
}

/// Takes `--theme NAME` out of the arguments, if it's there.
fn take_theme(a: &mut Vec<String>) -> Result<Option<&'static theme::Theme>, String> {
    let Some(i) = a.iter().position(|s| s == "--theme") else {
        return Ok(None);
    };
    let name = a.get(i + 1).cloned().ok_or(USAGE)?;
    a.drain(i..i + 2);
    theme::by_name(&name).map(Some)
}

fn main() {
    let mut a: Vec<String> = std::env::args().skip(1).collect();
    let r = take_theme(&mut a).and_then(|t| match a.as_slice() {
        // thumbnails can't read the saved theme (sandbox): light unless told
        // internal: step.rs runs itself as a child to tessellate
        [cmd, i] if cmd == "step-mesh" => step::mesh_to_stdout(i),
        [cmd, i, o, s] if cmd == "thumb" => thumb(i, o, s, t.unwrap_or(&theme::LIGHT)),
        [cmd, ..] if cmd == "thumb" => Err(USAGE.into()),
        [cmd] if cmd == "theme" => theme::command(None),
        [cmd, name] if cmd == "theme" => theme::command(Some(name)),
        [cmd, dirs @ ..] if cmd == "clear-thumbnails" => {
            let home = std::env::var_os("HOME").map(|h| Path::new(&h).join(".cache/thumbnails"));
            let dirs: Vec<std::path::PathBuf> = if dirs.is_empty() {
                home.into_iter().collect()
            } else {
                dirs.iter().map(Into::into).collect()
            };
            let n: usize = dirs.iter().map(|d| theme::clear_thumbnails(d)).sum();
            println!("{n} cached thumbnails cleared");
            Ok(())
        }
        [] => Err(USAGE.into()),
        files => {
            theme::refresh();
            view(files, t.unwrap_or_else(theme::current))
        }
    });
    if let Err(e) = r {
        if !e.is_empty() {
            eprintln!("[tridi] {e}");
        }
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn motion(button: MouseButton, delta: (f32, f32)) -> Event {
        Event::MouseMotion {
            button: Some(button),
            delta,
            position: PhysicalPoint { x: 0.0, y: 0.0 },
            modifiers: Modifiers::default(),
            handled: false,
        }
    }

    #[test]
    fn pan_orbit_zoom() {
        let vp = Viewport::new_at_origo(800, 600);
        let mut cam = Camera::new_perspective(
            vp,
            vec3(0.0, -10.0, 0.0),
            Vec3::zero(),
            Vec3::unit_z(),
            degrees(render::FOV),
            0.1,
            100.0,
        );
        // pan: target and camera move together, the distance stays
        navigate(&mut cam, &mut [motion(MouseButton::Right, (100.0, 0.0))], 1.0, 1.0, 50.0);
        assert!(cam.target().x < -0.5, "right drag moves the view: target {:?}", cam.target());
        assert!((cam.position().distance(cam.target()) - 10.0).abs() < 1e-3);
        // orbit: around the (new) target, same distance
        let t = cam.target();
        navigate(&mut cam, &mut [motion(MouseButton::Left, (50.0, 20.0))], 1.0, 1.0, 50.0);
        assert!((cam.target() - t).magnitude() < 1e-4 && (cam.position().distance(t) - 10.0).abs() < 1e-3);
        // zoom: never closer than min
        for _ in 0..50 {
            let wheel = Event::MouseWheel {
                delta: (0.0, 500.0),
                position: PhysicalPoint { x: 0.0, y: 0.0 },
                modifiers: Modifiers::default(),
                handled: false,
            };
            navigate(&mut cam, &mut [wheel], 1.0, 1.0, 50.0);
        }
        assert!(cam.position().distance(cam.target()) >= 1.0 - 1e-3);
    }

    // 7. the dock's app_id is a constant: if it follows the title or the file
    //    name, the dock goes back to "unknown"
    #[test]
    fn app_id_is_fixed() {
        assert_eq!(super::APP_ID, "tridi");
        assert!(include_str!("main.rs").contains(".with_name(APP_ID, APP_ID)"));
    }
}
