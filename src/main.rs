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

#[derive(clap::Parser)]
#[command(
    version,
    disable_help_subcommand = true,
    override_usage = "tridi [OPTIONS] FILE...\n       tridi [OPTIONS] <COMMAND>",
    about = "Viewer for point clouds and meshes, and their thumbnails in Nautilus",
    after_help = AFTER_HELP
)]
struct Cli {
    /// Files to open together in one window
    #[arg(value_name = "FILE")]
    files: Vec<String>,
    /// Theme for this run, instead of the one `tridi theme` saved
    #[arg(long, global = true, value_name = "THEME", value_parser = ["light", "dark"])]
    theme: Option<String>,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(clap::Subcommand)]
enum Cmd {
    /// Write a PNG thumbnail of IN to OUT, without a window (what Nautilus runs)
    Thumb {
        /// The file to draw
        #[arg(value_name = "IN")]
        input: String,
        /// The PNG to write
        #[arg(value_name = "OUT")]
        output: String,
        /// Width and height in pixels
        #[arg(value_name = "SIZE", value_parser = clap::value_parser!(u32).range(1..=4096))]
        size: u32,
    },
    /// Show the theme, or switch viewer and thumbnails to another one
    Theme {
        /// The theme to switch to; without it, prints the current one
        #[arg(value_parser = ["light", "dark"])]
        name: Option<String>,
    },
    /// Drop the cached thumbnails of our formats (default ~/.cache/thumbnails)
    ClearThumbnails {
        /// Thumbnail cache directories
        dirs: Vec<std::path::PathBuf>,
    },
    /// Internal: step.rs runs itself as a child to tessellate
    #[command(hide = true)]
    StepMesh { input: String },
    /// Packaging: write the man page and the bash, zsh and fish completions to DIR
    #[command(hide = true)]
    Generate { dir: std::path::PathBuf },
}

const AFTER_HELP: &str = "\
Examples:
  tridi scan.pcd                   open a point cloud
  tridi a.pcd b.pcd part.step      open several files, one color each
  tridi --theme dark model.glb     dark theme for this run only
  tridi theme dark                 switch viewer and thumbnails to dark

Formats:
  point clouds  pcd, ply (no faces), xyz, xyzrgb, pts
  meshes        glb, gltf, obj, stl, off, ply (with faces)
  CAD           step, stp

Window:
  left drag             orbit
  right or middle drag  pan (or shift + left drag)
  wheel                 zoom
  R                     reset the view
  + and -               point size
  1-9                   turn the N-th file off and on
  I                     show or hide the legend (on with several files)
  H or ?                show or hide these keys
  Q or Esc              quit

Environment:
  TRIDI_DEBUG=1    print which GPU renders

Files:
  ~/.config/tridi/theme
      the theme `tridi theme` saved
  ~/.local/share/thumbnailers/tridi.thumbnailer
      our thumbnailer entry, which wins over f3d's

Bugs: https://github.com/rccnroll/tridi/issues";

fn is_gltf(p: &str) -> bool {
    let ext = Path::new(p).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    ext == "glb" || ext == "gltf"
}

fn extent(pts: impl Iterator<Item = Vec3>) -> Vec3 {
    let mut bb = AxisAlignedBoundingBox::EMPTY;
    bb.expand(&pts.collect::<Vec<_>>());
    bb.max() - bb.min()
}

/// The Window section of the help, which H shows in the viewer.
fn window_keys() -> &'static str {
    let s = &AFTER_HELP[AFTER_HELP.find("Window:").unwrap_or(0)..];
    &s[..s.find("\n\n").unwrap_or(s.len())]
}

/// A file read, as the legend shows it: its tint (None when it has its own
/// colors, or is a mesh) and how many points or triangles.
struct File {
    name: String,
    tint: Option<[f32; 3]>,
    count: String,
}

/// Loads every file; one that fails is reported and skipped. None if nothing
/// could be read. Returns the scene inputs, the files read (in the same
/// order) and the up axis: +Y when every file is glTF, +Z otherwise.
fn load_all(paths: &[String], theme: &theme::Theme) -> Option<(Vec<Input>, Vec<File>, Vec3)> {
    let (mut inputs, mut files, mut all_gltf) = (vec![], vec![], true);
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
        match item {
            Item::Cloud(c) => {
                let tint = (paths.len() > 1).then(|| theme.palette[i % theme.palette.len()]);
                let e = extent(c.points.iter().copied());
                let shown = tint.filter(|_| cloud::own_colors(&c, tint).is_none());
                let tag = shown.map_or(String::new(), |t| format!("  color [{:.2}, {:.2}, {:.2}]", t[0], t[1], t[2]));
                let count = format!("{} points", c.points.len());
                println!("{name}: {count}, extent [{:.3}, {:.3}, {:.3}]{tag}", e.x, e.y, e.z);
                let colors = cloud::colorize(&c, tint, theme);
                inputs.push(Input::Points(c.points, colors));
                files.push(File { name, tint: shown, count });
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
                let count = format!("{} triangles", mesh::triangles(&m));
                println!("{name}: {count}, extent [{:.3}, {:.3}, {:.3}]", e.x, e.y, e.z);
                inputs.push(Input::Mesh(m));
                files.push(File { name, tint: None, count });
            }
        }
    }
    (!inputs.is_empty()).then(|| (inputs, files, up_for(all_gltf)))
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
    let (inputs, files, up) = load_all(paths, theme).ok_or("")?;
    if files.len() > 1 {
        let keys: Vec<String> = files
            .iter()
            .enumerate()
            .take(9)
            .map(|(i, f)| format!("{} {}", i + 1, f.name))
            .collect();
        println!("keys: {}", keys.join(", "));
    }
    let event_loop = winit::event_loop::EventLoop::new().map_err(|e| format!("no display: {e}"))?;
    let legend = files.len() > 1;
    let mut v = Viewer {
        files,
        theme,
        inputs: Some(inputs),
        up,
        gl: None,
        err: None,
        psize: 2.0,
        legend,
        help: false,
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
    /// draws the legend and the keys
    gui: GUI,
    /// egui's clock, for its fade-in
    start: Instant,
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
    files: Vec<File>,
    theme: &'static theme::Theme,
    inputs: Option<Vec<Input>>,
    up: Vec3,
    gl: Option<Gl>,
    err: Option<String>,
    psize: f32,
    /// I: the legend, on from the start with several files
    legend: bool,
    /// H or ?: the keys
    help: bool,
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
            .with_title(format!(
                "{APP_ID} — {}",
                self.files.iter().map(|f| f.name.as_str()).collect::<Vec<_>>().join(", ")
            ))
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
        let gui = GUI::new(&context);
        let dark = self.theme.name == "dark";
        gui.context()
            .set_visuals(if dark { egui::Visuals::dark() } else { egui::Visuals::light() });
        Ok(Gl {
            scene,
            gui,
            start: Instant::now(),
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

/// What's drawn over the scene, none of it clickable (the keys toggle, the
/// mouse moves the view). Top left, with `legend`: each file with its key,
/// its tint and its size, the ones turned off dimmed. Top right, with
/// `help`, the keys; else a hint bottom left that H shows them.
fn overlay(ui: &mut egui::Ui, files: &[File], visible: &[bool], legend: bool, help: bool) {
    let panel = |id: &str, at: egui::Align2, off: [f32; 2], add: &dyn Fn(&mut egui::Ui)| {
        egui::Area::new(egui::Id::new(id))
            .anchor(at, off)
            .interactable(false)
            .show(ui.ctx(), |ui| {
                // see-through, so the scene shows under it
                let fill = ui.visuals().window_fill.gamma_multiply(0.75);
                egui::Frame::popup(ui.style()).fill(fill).show(ui, add)
            });
    };
    if help {
        panel("help", egui::Align2::RIGHT_TOP, [-12.0, 12.0], &|ui| {
            // the help's lines: two spaces or more between key and what it does
            egui::Grid::new("keys").spacing([16.0, 4.0]).show(ui, |ui| {
                for (k, d) in window_keys().lines().skip(1).filter_map(|l| l.trim().split_once("  ")) {
                    ui.strong(k);
                    ui.label(d.trim());
                    ui.end_row();
                }
            });
        });
    } else {
        egui::Area::new(egui::Id::new("hint"))
            .anchor(egui::Align2::LEFT_BOTTOM, [12.0, -12.0])
            .interactable(false)
            .show(ui.ctx(), |ui| ui.weak("H  keys"));
    }
    if !legend {
        return;
    }
    panel("legend", egui::Align2::LEFT_TOP, [12.0, 12.0], &|ui| {
        egui::Grid::new("files").min_col_width(0.0).spacing([10.0, 4.0]).show(ui, |ui| {
            for (i, (f, &on)) in files.iter().zip(visible).enumerate() {
                let alpha = if on { 1.0 } else { 0.35 };
                let fg = ui.visuals().text_color().gamma_multiply(alpha);
                let text = |t: String| egui::RichText::new(t).color(fg);
                ui.label(text(if i < 9 { format!("{}", i + 1) } else { String::new() }));
                let (r, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                match f.tint {
                    Some(c) => {
                        // the palette is written to the screen as is: sRGB
                        let [r8, g8, b8] = c.map(|v| (v * 255.0).round() as u8);
                        let c = egui::Color32::from_rgb(r8, g8, b8).gamma_multiply(alpha);
                        ui.painter().rect_filled(r, 2.0, c);
                    }
                    // own colors or a mesh: an empty square
                    None => {
                        ui.painter()
                            .rect_stroke(r, 2.0, egui::Stroke::new(1.0_f32, fg), egui::StrokeKind::Inside);
                    }
                }
                ui.label(text(f.name.clone()));
                ui.label(text(f.count.clone()));
                ui.end_row();
            }
        });
    });
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
                    // H is Open3D's help key, ? everyone else's
                    "h" | "?" => self.help = !self.help,
                    "i" => self.legend = !self.legend,
                    d => {
                        if let Some(i) = d.parse::<usize>().ok().filter(|n| (1..=9).contains(n)).map(|n| n - 1)
                            && let Some(on) = gl.scene.toggle(i)
                        {
                            println!("{}: {}", self.files[i].name, if on { "on" } else { "off" });
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
                let (visible, legend, help) = (gl.scene.visible(), self.legend, self.help);
                let ms = gl.start.elapsed().as_secs_f64() * 1000.0;
                gl.gui
                    .update(&mut [], ms, vp, dpr, |ui| overlay(ui, &self.files, visible, legend, help));
                screen.write(|| gl.gui.render()).ok();
                // egui sizes a new area on one frame and shows it on the next
                if gl.gui.context().has_requested_repaint() {
                    gl.window.request_redraw();
                }
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

fn thumb(inp: &str, out: &str, size: u32, theme: &theme::Theme) -> Result<(), String> {
    let t0 = Instant::now();
    // Nautilus's sandbox clears the environment: without this glvnd would
    // also load NVIDIA's EGL, even for a user who limited it to Mesa
    let mesa = "/usr/share/glvnd/egl_vendor.d/50_mesa.json";
    if std::env::var_os("__EGL_VENDOR_LIBRARY_FILENAMES").is_none() && Path::new(mesa).exists() {
        // SAFETY: single-threaded, and before EGL is loaded
        unsafe { std::env::set_var("__EGL_VENDOR_LIBRARY_FILENAMES", mesa) };
    }
    let input = match mesh::load(Path::new(inp), theme.mesh).map_err(|e| format!("{inp}: {e}"))? {
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

/// `tridi generate DIR`: the man pages (tridi.1, one per subcommand), and
/// tridi.bash, _tridi and tridi.fish, which the packages install.
fn generate(dir: &Path) -> std::io::Result<()> {
    use clap_complete::Shell;
    let mut cmd = <Cli as clap::CommandFactory>::command();
    std::fs::create_dir_all(dir)?;
    clap_mangen::generate_to(cmd.clone(), dir)?;
    for sh in [Shell::Bash, Shell::Zsh, Shell::Fish] {
        clap_complete::generate_to(sh, &mut cmd, "tridi", dir)?;
    }
    Ok(())
}

fn main() {
    let cli = <Cli as clap::Parser>::parse();
    let t = cli.theme.as_deref().map(|n| theme::by_name(n).expect("clap checked the name"));
    let r = match cli.cmd {
        Some(Cmd::StepMesh { input }) => step::mesh_to_stdout(&input),
        Some(Cmd::Generate { dir }) => generate(&dir).map_err(|e| e.to_string()),
        // thumbnails can't read the saved theme (sandbox): light unless told
        Some(Cmd::Thumb { input, output, size }) => thumb(&input, &output, size, t.unwrap_or(&theme::LIGHT)),
        Some(Cmd::Theme { name }) => theme::command(name.as_deref()),
        Some(Cmd::ClearThumbnails { dirs }) => {
            let home = std::env::var_os("HOME").map(|h| Path::new(&h).join(".cache/thumbnails"));
            let dirs = if dirs.is_empty() { home.into_iter().collect() } else { dirs };
            let n: usize = dirs.iter().map(|d| theme::clear_thumbnails(d)).sum();
            println!("{n} cached thumbnails cleared");
            Ok(())
        }
        None if cli.files.is_empty() => {
            <Cli as clap::CommandFactory>::command().print_help().ok();
            std::process::exit(2);
        }
        None => {
            theme::refresh();
            view(&cli.files, t.unwrap_or_else(theme::current))
        }
    };
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

    #[test]
    fn command_line() {
        use clap::{CommandFactory, Parser};
        Cli::command().debug_assert();
        // the thumbnailer entries' Exec lines, light and dark
        for a in [
            &["tridi", "thumb", "i", "o", "256"][..],
            &["tridi", "thumb", "--theme", "dark", "i", "o", "256"],
        ] {
            assert!(matches!(Cli::parse_from(a).cmd, Some(Cmd::Thumb { size: 256, .. })));
        }
        let c = Cli::parse_from(["tridi", "--theme", "dark", "a.pcd", "b.step"]);
        assert_eq!((c.files.len(), c.theme.as_deref()), (2, Some("dark")));
        assert!(Cli::try_parse_from(["tridi", "--help"]).is_err_and(|e| e.exit_code() == 0));
        assert!(Cli::try_parse_from(["tridi", "-x"]).is_err_and(|e| e.exit_code() == 2));
        // H shows the Window section, all of it and nothing after
        let k = window_keys();
        assert!(k.starts_with("Window:") && k.ends_with("quit") && k.contains("H or ?"), "{k}");
    }

    #[test]
    fn generate_writes_what_the_packages_install() {
        let dir = std::env::temp_dir().join(format!("tridi-generate-{}", std::process::id()));
        generate(&dir).unwrap();
        for f in [
            "tridi.1",
            "tridi-thumb.1",
            "tridi-theme.1",
            "tridi-clear-thumbnails.1",
            "tridi.bash",
            "_tridi",
            "tridi.fish",
        ] {
            assert!(dir.join(f).is_file(), "{f}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

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
