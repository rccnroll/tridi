//! The window: the files loaded, mouse navigation, the keys, and the legend
//! and keys panels drawn by egui.

// ========================================== Imports ========================================== {{{

use eyre::{OptionExt, WrapErr, eyre};
use glutin::{
    config::ConfigTemplateBuilder,
    context::{ContextAttributesBuilder, PossiblyCurrentContext},
    display::GetGlDisplay,
    prelude::*,
    surface::{Surface, SurfaceAttributesBuilder, SwapInterval, WindowSurface},
};
use glutin_winit::{DisplayBuilder, GlWindow};
use std::{
    io::Write,
    num::{NonZeroU32, NonZeroUsize},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::Instant,
};
use three_d::{
    AxisAlignedBoundingBox, Camera, ClearState, Context, Event, GUI, HasContext, InnerSpace, MetricSpace, Modifiers, MouseButton,
    PhysicalPoint, RenderTarget, Vec3, Viewport, context, egui,
};
use three_d_asset::Geometry;
use tridi::{Input, Item, Scene, Theme};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, MouseButton as WinitButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy},
    keyboard::{Key, NamedKey},
    platform::wayland::WindowAttributesExtWayland,
    raw_window_handle::HasWindowHandle,
    window::{Window, WindowId},
};

use crate::{tell, window_keys};

// }}}

// ========================================= Constants ========================================= {{{

/// The Wayland `app_id` the dock groups the window by, matching the .desktop.
const APP_ID: &str = "io.github.rccnroll.tridi";

/// Radians of orbit per logical pixel dragged.
const ORBIT_SPEED: f32 = 0.008;

/// Wheel zoom: the share of the distance one unit of wheel moves.
const ZOOM_SPEED: f32 = 0.01;

/// Wheel units per notch, and per logical pixel of a touchpad: the scale
/// three-d's own window gave.
const WHEEL_NOTCH: f32 = 24.0;

const WHEEL_PIXEL: f32 = 0.24;

/// Point size in logical pixels: the start, the factor of + and -, the bounds.
const POINT_SIZE: f32 = 2.0;

const POINT_STEP: f32 = 1.25;

const POINT_MIN: f32 = 1.0;

const POINT_MAX: f32 = 20.0;

/// Zoom bounds, in scene radii: the closest stays above the near plane
/// (radius * 0.01), or zooming in clips everything.
const ZOOM_MIN: f32 = 0.05;

const ZOOM_MAX: f32 = 15.0;

/// The legend's dimming of a file turned off, and the panels' see-through.
const OFF_ALPHA: f32 = 0.35;

const PANEL_ALPHA: f32 = 0.75;

// }}}

// ========================================== Loading ========================================== {{{

/// A file on the command line, as the legend shows it.
struct File {
    name: String,
    state: State,
}

enum State {
    Loading,
    Failed,
    /// its layer in the scene, its tint (None when it has its own colors, or
    /// is a mesh), how many points or triangles so far and their bbox;
    /// `loading` until the last of a STEP file's solids is in, then how many
    /// were given up on
    Shown {
        layer: usize,
        tint: Option<[f32; 3]>,
        count: usize,
        unit: &'static str,
        bb: AxisAlignedBoundingBox,
        loading: bool,
        skipped: usize,
    },
}

/// A file read on a loader thread, or one of a STEP file's solids, made
/// ready for the scene.
struct Ready {
    input: Input,
    tint: Option<[f32; 3]>,
    count: usize,
    unit: &'static str,
    bb: AxisAlignedBoundingBox,
}

/// What a loader thread sends the event loop.
enum Loaded {
    /// the i-th file, or one of its solids if it's STEP
    Part { i: usize, ready: Ready },
    /// the i-th file is all in: how many solids were given up on, or why
    /// nothing came
    Done { i: usize, result: tridi::LoadResult<usize> },
}

/// Opens the files in one window; `out` gets what the user reads on stdout.
/// The files load in parallel and show up as they're read, a STEP file
/// solid by solid: the window opens with the first piece, so a slow file
/// holds back nothing but itself.
pub fn view(paths: &[String], theme: &'static Theme, mut out: Box<dyn Write>) -> eyre::Result<()> {
    let files: Vec<File> = paths
        .iter()
        .map(|p| File {
            name: Path::new(p)
                .file_name()
                .map_or_else(|| p.clone(), |n| n.to_string_lossy().into_owned()),
            state: State::Loading,
        })
        .collect();
    if files.is_empty() {
        return Err(eyre!("no file to open"));
    }
    if files.len() > 1 {
        let keys: Vec<String> = files
            .iter()
            .enumerate()
            .take(9)
            .map(|(i, f)| format!("{} {}", i + 1, f.name))
            .collect();
        tell(&mut *out, format_args!("keys: {}", keys.join(", ")));
    }
    let event_loop = EventLoop::<Loaded>::with_user_event().build().wrap_err("no display")?;
    load_all(paths, theme, &event_loop.create_proxy());
    let mut v = Viewer {
        pending: files.len(),
        legend: files.len() > 1,
        files,
        theme,
        up: tridi::up_for(paths.iter().all(|p| tridi::is_gltf(Path::new(p)))),
        out,
        gl: None,
        err: None,
        psize: POINT_SIZE,
        help: false,
        moved: false,
        cursor: None,
        button: None,
        mods: Modifiers::default(),
    };
    event_loop.run_app(&mut v).wrap_err("the window's event loop failed")?;
    v.err.map_or(Ok(()), Err)
}

/// Reads the files on as many threads as there are cores, each sent to the
/// event loop as it's ready. A file's tint is its place on the command line,
/// so it doesn't change with the order they finish in.
fn load_all(paths: &[String], theme: &'static Theme, proxy: &EventLoopProxy<Loaded>) {
    let paths: Arc<[String]> = paths.into();
    let next = Arc::new(AtomicUsize::new(0));
    let workers = thread::available_parallelism().map_or(1, NonZeroUsize::get).min(paths.len());
    for _ in 0..workers {
        let (paths, next, proxy) = (Arc::clone(&paths), Arc::clone(&next), proxy.clone());
        thread::spawn(move || {
            loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(p) = paths.get(i) else { break };
                let tint = theme.palette.iter().cycle().nth(i).copied().filter(|_| paths.len() > 1);
                let result = tridi::load(Path::new(p), theme.mesh, None, &mut |item| {
                    #[expect(clippy::let_underscore_must_use, reason = "a closed window is found out below")]
                    let _ = proxy.send_event(Loaded::Part {
                        i,
                        ready: ready(item, tint, theme),
                    });
                });
                // the window was closed: nobody waits for the rest
                if proxy.send_event(Loaded::Done { i, result }).is_err() {
                    break;
                }
            }
        });
    }
}

/// Colors what was read; `tint` is its file's color among several files.
fn ready(item: Item, tint: Option<[f32; 3]>, theme: &Theme) -> Ready {
    match item {
        Item::Cloud(c) => Ready {
            bb: bbox(c.points.iter().copied()),
            // the legend shows the tint only if it's what the points get
            tint: tint.filter(|_| tridi::own_colors(&c, tint).is_none()),
            count: c.points.len(),
            unit: "points",
            input: {
                let colors = tridi::colorize(&c, tint, theme);
                Input::Points(c.points, colors)
            },
        },
        Item::Mesh(m) => Ready {
            // ponytail: no per-file tint on meshes (1.0 didn't have one either)
            bb: bbox(m.geometries.iter().flat_map(|g| {
                match &g.geometry {
                    Geometry::Triangles(t) => t
                        .positions
                        .to_f32()
                        .into_iter()
                        .map(|v| (g.transformation * v.extend(1.0)).truncate())
                        .collect(),
                    Geometry::Points(_) => vec![],
                }
            })),
            tint: None,
            count: tridi::triangles(&m),
            unit: "triangles",
            input: Input::Mesh(m),
        },
    }
}

fn bbox<I: Iterator<Item = Vec3>>(pts: I) -> AxisAlignedBoundingBox {
    let mut bb = AxisAlignedBoundingBox::EMPTY;
    bb.expand(&pts.collect::<Vec<_>>());
    bb
}

// }}}

// ======================================== Navigation ========================================= {{{

/// Mouse navigation around the camera's target: left drag orbits, right or
/// middle drag (or shift + left) pans, the wheel zooms. three-d's
/// `OrbitControl` has no pan and a fixed target, so it's done here.
fn navigate(cam: &mut Camera, events: &mut [Event], dpr: f32, min: f32, max: f32) {
    for e in events.iter_mut() {
        #[expect(clippy::wildcard_enum_match_arm, reason = "only the mouse moves the view")]
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
                    let k = 2.0 * dist * (tridi::FOV.to_radians() / 2.0).tan() * dpr / tridi::px_f32(cam.viewport().height);
                    let right = cam.right_direction();
                    let up = right.cross(cam.view_direction()).normalize();
                    cam.translate((up * delta.1 - right * delta.0) * k);
                } else if *b == MouseButton::Left {
                    let t = cam.target();
                    cam.rotate_around_with_fixed_up(t, ORBIT_SPEED * delta.0, ORBIT_SPEED * delta.1);
                }
                *handled = true;
            }
            Event::MouseWheel { delta, handled, .. } if !*handled => {
                let (t, dist) = (cam.target(), cam.position().distance(cam.target()));
                cam.zoom_towards(t, dist * (1.0 - (-delta.1 * ZOOM_SPEED).exp()), min, max);
                *handled = true;
            }
            _ => {}
        }
    }
}

// }}}

// ========================================== Viewer =========================================== {{{

/// The window, its GL surface and what's drawn in it: made once the event
/// loop starts, as winit 0.30 wants.
struct Gl {
    scene: Scene,
    /// draws the legend and the keys
    gui: GUI,
    /// egui's clock, for its fade-in
    start: Instant,
    cam: Camera,
    context: Context,
    ctx: PossiblyCurrentContext,
    surface: Surface<WindowSurface>,
    window: Window,
}

/// Our own window (winit 0.30 + glutin) instead of three-d's, which pins
/// winit 0.28: on GNOME, where winit draws the title bar itself, at scale 2
/// that one sends a 45 px high buffer and the compositor kills the window.
/// three-d only gets the GL context.
struct Viewer {
    files: Vec<File>,
    /// files not read yet
    pending: usize,
    theme: &'static Theme,
    up: Vec3,
    /// stdout: the files turned off and on
    out: Box<dyn Write>,
    gl: Option<Gl>,
    /// what closed the window, if not the user
    err: Option<eyre::Report>,
    psize: f32,
    /// I: the legend, on from the start with several files
    legend: bool,
    /// H or ?: the keys
    help: bool,
    /// the user moved the view: a file read later no longer reframes it
    moved: bool,
    /// last cursor position, logical pixels
    cursor: Option<(f32, f32)>,
    button: Option<MouseButton>,
    mods: Modifiers,
}

impl Viewer {
    /// Opens the window with the first file read.
    fn open(&self, event_loop: &ActiveEventLoop, input: Input) -> eyre::Result<Gl> {
        let names: Vec<&str> = self.files.iter().map(|f| f.name.as_str()).collect();
        let attrs = Window::default_attributes()
            .with_title(format!("tridi — {}", names.join(", ")))
            // the app_id the dock groups the window by, matching the .desktop
            .with_name(APP_ID, APP_ID);
        // 4x MSAA if there is one, as three-d's window had
        let tmpl = ConfigTemplateBuilder::new().with_depth_size(24);
        let (window, config) = tridi::quiet_stderr(|| {
            DisplayBuilder::new()
                .with_window_attributes(Some(attrs))
                .build(event_loop, tmpl, |cs| {
                    #[expect(clippy::expect_used, reason = "glutin calls the picker with at least one config")]
                    cs.max_by_key(|c| c.num_samples().min(4)).expect("no GL config")
                })
        })
        // glutin-winit's error is a Box<dyn Error> without Send: keep its text
        .map_err(|e| eyre!("cannot open the window: {e}"))?;
        let window = window.ok_or_eyre("cannot open the window")?;
        let display = config.display();
        let handle = window.window_handle().wrap_err("no window handle")?.as_raw();
        let attrs = ContextAttributesBuilder::new().build(Some(handle));
        let sattrs = window
            .build_surface_attributes(SurfaceAttributesBuilder::default())
            .wrap_err("no surface")?;
        // SAFETY: the config, the window handle and the surface attributes
        // all come from this display and this window, which outlive them
        let (ctx, surface) = unsafe {
            let ctx = display.create_context(&config, &attrs).wrap_err("cannot create the GL context")?;
            let surface = display
                .create_window_surface(&config, &sattrs)
                .wrap_err("cannot create the GL surface")?;
            (ctx.make_current(&surface).wrap_err("cannot make the GL context current")?, surface)
        };
        if let Err(e) = surface.set_swap_interval(&ctx, SwapInterval::Wait(NonZeroU32::MIN)) {
            debug!("no vsync: {e}");
        }
        // SAFETY: the context made current above, whose functions EGL returns
        let gl = unsafe { context::Context::from_loader_function_cstr(|s| display.get_proc_address(s)) };
        let context = Context::from_gl_context(Arc::new(gl)).wrap_err("cannot load GL")?;
        // SAFETY: a plain query on the current context
        debug!(renderer = unsafe { context.get_parameter_string(context::RENDERER) }, "GL");
        let scene = Scene::new(&context, vec![input], true, self.up)?;
        let cam = scene.camera(viewport(&window));
        let gui = GUI::new(&context);
        let dark = self.theme.name == "dark";
        gui.context()
            .set_visuals(if dark { egui::Visuals::dark() } else { egui::Visuals::light() });
        Ok(Gl {
            scene,
            gui,
            start: Instant::now(),
            cam,
            context,
            ctx,
            surface,
            window,
        })
    }

    /// A piece read: into the scene, the window opened if it's the first.
    fn part(&mut self, event_loop: &ActiveEventLoop, i: usize, r: Ready) {
        let to = match self.files.get(i) {
            Some(File {
                state: State::Shown { layer, .. },
                ..
            }) => Some(*layer),
            _ => None,
        };
        let layer = match self.show(event_loop, to, r.input) {
            Ok(layer) => layer,
            Err(e) => {
                self.err = Some(e);
                return event_loop.exit();
            }
        };
        if let Some(f) = self.files.get_mut(i) {
            if let State::Shown { count, bb, .. } = &mut f.state {
                *count += r.count;
                bb.expand_with_aabb(r.bb);
            } else {
                f.state = State::Shown {
                    layer,
                    tint: r.tint,
                    count: r.count,
                    unit: r.unit,
                    bb: r.bb,
                    loading: true,
                    skipped: 0,
                };
            }
        }
        self.request_redraw();
    }

    /// A file all in: what it is told on stdout, or why it failed.
    fn done(&mut self, event_loop: &ActiveEventLoop, i: usize, result: tridi::LoadResult<usize>) {
        self.pending = self.pending.saturating_sub(1);
        if let Some(f) = self.files.get_mut(i) {
            match (result, &mut f.state) {
                (
                    Ok(n),
                    State::Shown {
                        tint,
                        count,
                        unit,
                        bb,
                        loading,
                        skipped,
                        ..
                    },
                ) => {
                    (*loading, *skipped) = (false, n);
                    let e = bb.max() - bb.min();
                    let tag = tint.map_or_else(String::new, |[r, g, b]| format!("  color [{r:.2}, {g:.2}, {b:.2}]"));
                    let skip = if n > 0 { format!(", {n} solids skipped") } else { String::new() };
                    tell(
                        &mut *self.out,
                        format_args!("{}: {count} {unit}, extent [{:.3}, {:.3}, {:.3}]{tag}{skip}", f.name, e.x, e.y, e.z),
                    );
                }
                (Err(e), state) => {
                    warn!("{}: {:#}", f.name, eyre::Report::new(e));
                    *state = State::Failed;
                }
                // a load that went well sent at least one piece
                (Ok(_), _) => {}
            }
        }
        if self.pending == 0 && self.gl.is_none() {
            // each file that failed was reported as it was read
            self.err = Some(eyre!("no file could be opened"));
            event_loop.exit();
        }
        self.request_redraw();
    }

    /// Puts a piece in the scene, in layer `to` if its file has one already
    /// (a STEP file's next solid); returns the layer.
    fn show(&mut self, event_loop: &ActiveEventLoop, to: Option<usize>, input: Input) -> eyre::Result<usize> {
        let Some(gl) = self.gl.as_mut() else {
            self.gl = Some(self.open(event_loop, input)?);
            return Ok(0);
        };
        let layer = match (to, input) {
            (Some(layer), Input::Mesh(m)) => {
                gl.scene.add_to(layer, &m)?;
                layer
            }
            // only meshes come in pieces
            (_, input) => gl.scene.add(input)?,
        };
        if !self.moved {
            gl.cam = gl.scene.camera(viewport(&gl.window));
        }
        Ok(layer)
    }

    /// A key pressed; true if the scene has to be drawn again.
    fn key(&mut self, event_loop: &ActiveEventLoop, key: &Key) -> bool {
        let Some(gl) = self.gl.as_mut() else { return false };
        #[expect(clippy::wildcard_enum_match_arm, reason = "the keys we use; every other one does nothing")]
        let c = match key {
            Key::Named(NamedKey::Escape) => "q".to_owned(),
            Key::Character(c) => c.to_lowercase(),
            _ => return false,
        };
        match c.as_str() {
            "r" => {
                gl.cam = gl.scene.camera(viewport(&gl.window));
                self.moved = false;
            }
            "+" | "=" => self.psize = f32::min(self.psize * POINT_STEP, POINT_MAX),
            "-" => self.psize = f32::max(self.psize / POINT_STEP, POINT_MIN),
            "q" => event_loop.exit(),
            // H is Open3D's help key, ? everyone else's
            "h" | "?" => self.help = !self.help,
            "i" => self.legend = !self.legend,
            d => {
                if let Some(i) = d.parse::<usize>().ok().filter(|n| (1..=9).contains(n)).map(|n| n - 1)
                    && let Some(f) = self.files.get(i)
                    && let State::Shown { layer, .. } = f.state
                    && let Some(on) = gl.scene.toggle(layer)
                {
                    tell(&mut *self.out, format_args!("{}: {}", f.name, if on { "on" } else { "off" }));
                }
            }
        }
        true
    }

    fn request_redraw(&self) {
        if let Some(gl) = &self.gl {
            gl.window.request_redraw();
        }
    }

    /// Draws the scene and the panels, and shows the frame.
    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        let Some(gl) = self.gl.as_mut() else { return };
        let dpr = scale(&gl.window);
        let vp = viewport(&gl.window);
        gl.cam.set_viewport(vp);
        let screen = RenderTarget::screen(&gl.context, vp.width, vp.height);
        let [r, g, b] = self.theme.bg;
        screen.clear(ClearState::color_and_depth(r, g, b, 1.0, 1.0));
        gl.scene.render(&screen, &gl.cam, self.psize * dpr);
        let (visible, legend, help) = (gl.scene.visible(), self.legend, self.help);
        let ms = gl.start.elapsed().as_secs_f64() * 1000.0;
        gl.gui
            .update(&mut [], ms, vp, dpr, |ui| overlay(ui, &self.files, visible, legend, help));
        if let Err(e) = screen.write(|| gl.gui.render()) {
            warn!("cannot draw the panels: {e}");
        }
        // egui sizes a new area on one frame and shows it on the next
        if gl.gui.context().has_requested_repaint() {
            gl.window.request_redraw();
        }
        if let Err(e) = gl.surface.swap_buffers(&gl.ctx) {
            self.err = Some(eyre::Report::new(e).wrap_err("cannot show the frame"));
            event_loop.exit();
        }
    }
}

impl ApplicationHandler<Loaded> for Viewer {
    // the window opens with the first file read, in `user_event`
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {}

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: Loaded) {
        match event {
            Loaded::Part { i, ready } => self.part(event_loop, i, ready),
            Loaded::Done { i, result } => self.done(event_loop, i, result),
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _window_id: WindowId, event: WindowEvent) {
        let Some(gl) = self.gl.as_mut() else { return };
        let dpr = scale(&gl.window);
        let mut events = vec![];
        #[expect(clippy::wildcard_enum_match_arm, reason = "winit has dozens of events; these are ours")]
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
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
                #[expect(clippy::wildcard_enum_match_arm, reason = "the three buttons that move the view")]
                let b = match button {
                    WinitButton::Left => Some(MouseButton::Left),
                    WinitButton::Right => Some(MouseButton::Right),
                    WinitButton::Middle => Some(MouseButton::Middle),
                    _ => None,
                };
                self.button = b.filter(|_| state == ElementState::Pressed);
            }
            WindowEvent::CursorMoved { position, .. } => {
                let p = position.to_logical::<f32>(f64::from(dpr));
                if let (Some((x, y)), Some(b)) = (self.cursor, self.button) {
                    let at = position.cast::<f32>();
                    events.push(Event::MouseMotion {
                        button: Some(b),
                        delta: (p.x - x, p.y - y),
                        position: PhysicalPoint { x: at.x, y: at.y },
                        modifiers: self.mods,
                        handled: false,
                    });
                }
                self.cursor = Some((p.x, p.y));
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let y = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y * WHEEL_NOTCH,
                    MouseScrollDelta::PixelDelta(d) => d.to_logical::<f32>(f64::from(dpr)).y * WHEEL_PIXEL,
                };
                events.push(Event::MouseWheel {
                    delta: (0.0, y),
                    position: PhysicalPoint { x: 0.0, y: 0.0 },
                    modifiers: self.mods,
                    handled: false,
                });
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                if self.key(event_loop, &event.logical_key.clone()) {
                    self.request_redraw();
                }
                return;
            }
            WindowEvent::RedrawRequested => return self.redraw(event_loop),
            _ => return,
        }
        self.moved |= !events.is_empty();
        let r = gl.scene.radius();
        navigate(&mut gl.cam, &mut events, dpr, r * ZOOM_MIN, r * ZOOM_MAX);
        gl.window.request_redraw();
    }
}

// }}}

// ========================================== Overlay ========================================== {{{

/// What's drawn over the scene, none of it clickable (the keys toggle, the
/// mouse moves the view). Top left, with `legend`: each file with its key,
/// its tint and its size, the ones turned off, still loading or failed
/// dimmed. Top right, with
/// `help`, the keys; else a hint bottom left that H shows them.
fn overlay(ui: &egui::Ui, files: &[File], visible: &[bool], legend: bool, help: bool) {
    let panel = |id: &str, at: egui::Align2, off: [f32; 2], add: &dyn Fn(&mut egui::Ui)| {
        egui::Area::new(egui::Id::new(id))
            .anchor(at, off)
            .interactable(false)
            .show(ui.ctx(), |ui| {
                // see-through, so the scene shows under it
                let fill = ui.visuals().window_fill.gamma_multiply(PANEL_ALPHA);
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
        let loading = files
            .iter()
            .any(|f| matches!(f.state, State::Loading | State::Shown { loading: true, .. }));
        egui::Area::new(egui::Id::new("hint"))
            .anchor(egui::Align2::LEFT_BOTTOM, [12.0, -12.0])
            .interactable(false)
            .show(ui.ctx(), |ui| ui.weak(if loading { "loading…   H  keys" } else { "H  keys" }));
    }
    if !legend {
        return;
    }
    panel("legend", egui::Align2::LEFT_TOP, [12.0, 12.0], &|ui| {
        egui::Grid::new("files").min_col_width(0.0).spacing([10.0, 4.0]).show(ui, |ui| {
            for (i, f) in files.iter().enumerate() {
                let (on, tint, count) = match &f.state {
                    State::Shown {
                        layer,
                        tint,
                        count,
                        unit,
                        loading,
                        skipped,
                        ..
                    } => {
                        let more = match (loading, skipped) {
                            (true, _) => ", loading…".to_owned(),
                            (false, 0) => String::new(),
                            (false, n) => format!(", {n} solids skipped"),
                        };
                        (visible.get(*layer).copied().unwrap_or(true), *tint, format!("{count} {unit}{more}"))
                    }
                    State::Loading => (false, None, "loading…".to_owned()),
                    State::Failed => (false, None, "failed".to_owned()),
                };
                let alpha = if on { 1.0 } else { OFF_ALPHA };
                let fg = ui.visuals().text_color().gamma_multiply(alpha);
                let text = |t: String| egui::RichText::new(t).color(fg);
                ui.label(text(if i < 9 { format!("{}", i + 1) } else { String::new() }));
                let (r, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                if let Some(c) = tint {
                    // the palette is written to the screen as is: sRGB
                    let [r8, g8, b8] = c.map(byte);
                    let c = egui::Color32::from_rgb(r8, g8, b8).gamma_multiply(alpha);
                    ui.painter().rect_filled(r, 2.0, c);
                } else {
                    // own colors or a mesh: an empty square
                    ui.painter()
                        .rect_stroke(r, 2.0, egui::Stroke::new(1.0_f32, fg), egui::StrokeKind::Inside);
                }
                ui.label(text(f.name.clone()));
                ui.label(text(count));
                ui.end_row();
            }
        });
    });
}

/// A 0..1 channel as a byte.
fn byte(v: f32) -> u8 {
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to 0..255 first"
    )]
    let b = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    b
}

// }}}

// ========================================== Helpers ========================================== {{{

/// The window's scale factor, for GL's float math.
fn scale(w: &Window) -> f32 {
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        reason = "a scale factor is a small number"
    )]
    let s = w.scale_factor() as f32;
    s
}

fn viewport(w: &Window) -> Viewport {
    let s = w.inner_size();
    Viewport::new_at_origo(s.width.max(1), s.height.max(1))
}

// }}}

// =========================================== Tests =========================================== {{{

#[cfg(test)]
mod tests {
    use super::*;
    use three_d::{Zero, degrees, vec3};

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
            degrees(tridi::FOV),
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

    // 7. the dock's app_id is a constant matching the .desktop and the
    //    metainfo: if it follows the title or the file name, the dock goes
    //    back to "unknown"; the title says tridi, not the app id
    #[test]
    fn app_id_is_fixed() {
        assert_eq!(super::APP_ID, "io.github.rccnroll.tridi");
        let src = include_str!("view.rs");
        assert!(src.contains(".with_name(APP_ID, APP_ID)"), "the window's app_id");
        assert!(src.contains(r#".with_title(format!("tridi — {}""#), "the title");
        let meta = include_str!("../share/metainfo/io.github.rccnroll.tridi.metainfo.xml");
        assert!(meta.contains("<id>io.github.rccnroll.tridi</id>"), "the metainfo id");
        assert!(
            meta.contains(r#"<launchable type="desktop-id">io.github.rccnroll.tridi.desktop</launchable>"#),
            "the metainfo's desktop entry"
        );
    }
}

// }}}
