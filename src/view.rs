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
    time::{Duration, Instant},
};
use three_d::{
    AxisAlignedBoundingBox, Camera, ClearState, Context, Event, GUI, HasContext, InnerSpace, MetricSpace, Modifiers, MouseButton,
    PhysicalPoint, RenderTarget, Vec3, Viewport, context, egui,
};
use three_d_asset::Geometry;
use tridi::{Input, Item, Purpose, Scene, Theme};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, MouseButton as WinitButton, MouseScrollDelta, StartCause, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
    keyboard::{Key, NamedKey},
    platform::wayland::WindowAttributesExtWayland,
    raw_window_handle::HasWindowHandle,
    window::{CursorIcon, Window, WindowId},
};

use crate::{
    pick::{self, Pick},
    tell, window_keys,
};

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

/// How long the picks panel says it copied.
const COPIED_FOR: Duration = Duration::from_secs(2);

/// The picks panel's rows at most: the wheel scrolls the rest.
const PANEL_ROWS: usize = 8;

/// A file's name in the picks panel, characters at most.
const NAME_MAX: usize = 28;

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
    /// a cloud's points kept for picking, with their records in the file
    cloud: Option<tridi::Cloud>,
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
        paths: paths.to_vec(),
        clouds: paths.iter().map(|_| None).collect(),
        picks: vec![],
        press: None,
        files,
        theme,
        up: tridi::up_for(paths.iter().all(|p| tridi::is_gltf(Path::new(p)))),
        out,
        gl: None,
        err: None,
        psize: POINT_SIZE,
        help: false,
        panel: false,
        panel_rows: vec![],
        panel_top: None,
        clipboard: None,
        copied: None,
        rows: vec![],
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
                let result = tridi::load(Path::new(p), theme.mesh, Purpose::View, &mut |item| {
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
            // before `input` takes the points
            cloud: Some(tridi::Cloud {
                points: c.points.clone(),
                colors: None,
                faces: None,
                index: c.index.clone(),
            }),
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
            cloud: None,
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
#[expect(
    clippy::struct_excessive_bools,
    reason = "the panels and the view's state: independent flags, not a state machine"
)]
struct Viewer {
    files: Vec<File>,
    /// the files as given on the command line: what the pick lines name
    paths: Vec<String>,
    /// each file's points, if it's a cloud and shown: what Shift + click picks from
    clouds: Vec<Option<tridi::Cloud>>,
    /// the points picked, in order: Shift + right click takes the last off
    picks: Vec<Pick>,
    /// the button and where it was pressed with Shift, logical pixels
    press: Option<(MouseButton, (f32, f32))>,
    /// files not read yet
    pending: usize,
    theme: &'static Theme,
    up: Vec3,
    /// stdout: the files turned off and on, the points picked and removed
    out: Box<dyn Write>,
    gl: Option<Gl>,
    /// what closed the window, if not the user
    err: Option<eyre::Report>,
    psize: f32,
    /// I: the legend, on from the start with several files
    legend: bool,
    /// H or ?: the keys
    help: bool,
    /// P: the picks panel, opened by the first pick
    panel: bool,
    /// the picks panel's rows on screen, logical pixels, and their pick: a click copies it
    panel_rows: Vec<(egui::Rect, usize)>,
    /// the first pick the panel shows, scrolled with the wheel; None follows the newest
    panel_top: Option<usize>,
    /// made at the first copy, and kept: on X11 what's copied goes with it
    clipboard: Option<arboard::Clipboard>,
    /// when the panel last copied, and what it says about it
    copied: Option<(Instant, String)>,
    /// the legend's rows on screen, logical pixels, and their file: a click toggles it
    rows: Vec<(egui::Rect, usize)>,
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
        if let (Some(c), Some(slot)) = (r.cloud, self.clouds.get_mut(i)) {
            *slot = Some(c);
        }
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
            Key::Named(NamedKey::Delete) => "delete".to_owned(),
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
            "p" => self.panel = !self.panel,
            "delete" => {
                while !self.picks.is_empty() {
                    self.unpick(None);
                }
            }
            "c" if self.mods.ctrl && self.panel => self.copy(None),
            "i" => self.legend = !self.legend,
            "a" => {
                // any on: all off; all off: all on
                let on = !gl.scene.visible().iter().any(|v| *v);
                let flip: Vec<usize> = (self.files.iter().enumerate())
                    .filter(|(_, f)| {
                        matches!(f.state, State::Shown { layer, .. }
                            if gl.scene.visible().get(layer).is_some_and(|v| *v != on))
                    })
                    .map(|(i, _)| i)
                    .collect();
                for i in flip {
                    self.toggle(i);
                }
            }
            d => {
                if let Some(i) = d.parse::<usize>().ok().filter(|n| (1..=9).contains(n)).map(|n| n - 1) {
                    self.toggle(i);
                }
            }
        }
        true
    }

    /// Turns the i-th file off or on, if it's shown.
    fn toggle(&mut self, i: usize) {
        if let Some(gl) = self.gl.as_mut()
            && let Some(f) = self.files.get(i)
            && let State::Shown { layer, .. } = f.state
            && let Some(on) = gl.scene.toggle(layer)
        {
            tell(&mut *self.out, format_args!("{}: {}", f.name, if on { "on" } else { "off" }));
        }
    }

    /// A Shift click: left picks the point at `at`, right takes the last pick off.
    fn clicked(&mut self, b: MouseButton, at: (f32, f32)) {
        if b == MouseButton::Left {
            self.pick(at);
        } else {
            self.unpick(None);
        }
    }

    /// Shift + click at `at` (logical pixels): the point of a shown cloud
    /// under it, if any, kept and told on stdout.
    fn pick(&mut self, at: (f32, f32)) {
        let Some(gl) = self.gl.as_ref() else { return };
        let mvp = gl.cam.projection() * gl.cam.view();
        let (visible, sizes, psize) = (gl.scene.visible(), gl.scene.sizes(), self.psize);
        let clouds = self.files.iter().zip(&self.clouds).enumerate().filter_map(|(i, (f, c))| {
            let State::Shown { layer, .. } = f.state else { return None };
            let c = c.as_ref().filter(|_| visible.get(layer).is_some_and(|v| *v))?;
            // a big point is picked anywhere it's drawn
            let radius = pick::PICK_RADIUS.max(psize * sizes.get(layer).copied().unwrap_or(1.0) / 2.0);
            Some((i, c.points.as_slice(), radius))
        });
        let Some((file, i)) = pick::under(clouds, mvp, logical_size(&gl.window), at) else {
            return;
        };
        let Some(c) = self.clouds.get(file).and_then(Option::as_ref) else {
            return;
        };
        let Some(&at) = c.points.get(i) else { return };
        let p = Pick {
            file,
            index: c.file_index(i),
            at,
        };
        let path = self.paths.get(file).map_or("", String::as_str);
        tell(&mut *self.out, format_args!("{}", pick::line("picked", &p, path)));
        // the panel shows up with the first pick: no terminal, no stdout
        self.panel |= self.picks.is_empty();
        self.panel_top = None;
        self.picks.push(p);
    }

    /// Pick `n` (0-based) taken off, or the last one, told on stdout.
    fn unpick(&mut self, n: Option<usize>) {
        let n = n.unwrap_or(self.picks.len().saturating_sub(1));
        if n < self.picks.len() {
            let p = self.picks.remove(n);
            let path = self.paths.get(p.file).map_or("", String::as_str);
            tell(&mut *self.out, format_args!("{}", pick::line("removed", &p, path)));
        }
    }

    /// A click on the picks panel's row `n`: left copies that pick, right
    /// takes it off.
    fn panel_click(&mut self, b: Option<MouseButton>, n: usize) {
        match b {
            Some(MouseButton::Left) => self.copy(Some(n)),
            Some(MouseButton::Right) => self.unpick(Some(n)),
            _ => {}
        }
    }

    /// Copies pick `n` (0-based), or all of them, as their stdout lines; the
    /// panel says how it went for a while.
    fn copy(&mut self, n: Option<usize>) {
        let picks = match n {
            Some(n) => self.picks.get(n..=n).unwrap_or_default(),
            None => &self.picks,
        };
        if picks.is_empty() {
            return;
        }
        let text = pick::copy(picks, &self.paths);
        let what = if picks.len() == 1 {
            "1 point".to_owned()
        } else {
            format!("{} points", picks.len())
        };
        if self.clipboard.is_none() {
            self.clipboard = arboard::Clipboard::new().map_err(|e| warn!("no clipboard: {e}")).ok();
        }
        let done = self.clipboard.as_mut().map(|c| c.set_text(text));
        let said = match done {
            Some(Ok(())) => format!("copied {what}"),
            Some(Err(e)) => {
                warn!("cannot copy: {e}");
                "cannot copy".to_owned()
            }
            None => "no clipboard".to_owned(),
        };
        self.copied = Some((Instant::now(), said));
    }

    /// Where the picks are on screen, numbered from 1: those of shown files,
    /// in view.
    fn marks(&self) -> Vec<(usize, (f32, f32))> {
        let Some(gl) = self.gl.as_ref() else { return vec![] };
        let mvp = gl.cam.projection() * gl.cam.view();
        let size = logical_size(&gl.window);
        let shown = |file: usize| layer(&self.files, file).is_some_and(|l| gl.scene.visible().get(l).is_some_and(|v| *v));
        (self.picks.iter().enumerate())
            .filter(|(_, p)| shown(p.file))
            .filter_map(|(n, p)| Some((n + 1, pick::screen(p.at, mvp, size)?)))
            .collect()
    }

    /// What the picks panel shows, if it's open: the picks in view, each
    /// with its place in `picks`, its row, its file's name when there are
    /// several files, and whether that file is shown.
    fn panel_view(&self) -> PanelView {
        let mut view = PanelView::default();
        let Some(gl) = self.gl.as_ref().filter(|_| self.panel) else {
            return view;
        };
        let shown = |file: usize| layer(&self.files, file).is_some_and(|l| gl.scene.visible().get(l).is_some_and(|v| *v));
        let range = pick::shown(self.picks.len(), self.panel_top, PANEL_ROWS);
        (view.above, view.below) = (range.start, self.picks.len() - range.end);
        for (n, p) in self.picks.iter().enumerate().take(range.end).skip(range.start) {
            let name = (self.files.len() > 1)
                .then(|| self.files.get(p.file).map(|f| pick::short_name(&f.name, NAME_MAX)))
                .flatten();
            let prev = n.checked_sub(1).and_then(|k| self.picks.get(k));
            view.rows.push((n, pick::row(n + 1, p, prev), name, shown(p.file)));
        }
        view
    }

    fn request_redraw(&self) {
        if let Some(gl) = &self.gl {
            gl.window.request_redraw();
        }
    }

    /// Draws the scene and the panels, and shows the frame.
    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        let marks = self.marks();
        let lines = self.panel_view();
        // "copied" stays a while, then a timer redraws without it
        self.copied = self.copied.take().filter(|(t, _)| t.elapsed() < COPIED_FOR);
        if let Some((t, _)) = &self.copied {
            event_loop.set_control_flow(ControlFlow::WaitUntil(*t + COPIED_FOR));
        }
        let status = self.copied.as_ref().map(|(_, s)| s.as_str());
        let Some(gl) = self.gl.as_mut() else { return };
        let dpr = scale(&gl.window);
        let vp = viewport(&gl.window);
        gl.cam.set_viewport(vp);
        let screen = RenderTarget::screen(&gl.context, vp.width, vp.height);
        let [r, g, b] = self.theme.bg;
        screen.clear(ClearState::color_and_depth(r, g, b, 1.0, 1.0));
        gl.scene.render(&screen, &gl.cam, self.psize * dpr);
        let (visible, sizes, legend, help) = (gl.scene.visible(), gl.scene.sizes(), self.legend, self.help);
        let ms = gl.start.elapsed().as_secs_f64() * 1000.0;
        let (hover, over) = (row(&self.rows, self.cursor), row(&self.panel_rows, self.cursor));
        let (rows, panel_rows) = (&mut self.rows, &mut self.panel_rows);
        gl.gui.update(&mut [], ms, vp, dpr, |ui| {
            *rows = overlay(ui, &self.files, visible, sizes, hover, legend, help);
            *panel_rows = picks_panel(ui, &lines, over, status);
            draw_marks(ui, &marks, over);
        });
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

    /// The picks panel's "copied" timer ran out: drawn again without it.
    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        if matches!(cause, StartCause::ResumeTimeReached { .. }) {
            event_loop.set_control_flow(ControlFlow::Wait);
            self.request_redraw();
        }
    }

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
                let (b, pressed) = (mouse_button(button), state == ElementState::Pressed);
                // on a pick's row, left copies it, right takes it off
                if let Some(n) = row(&self.panel_rows, self.cursor).filter(|_| pressed) {
                    self.panel_click(b, n);
                    return self.request_redraw();
                }
                // on a legend row, left turns its file off or on, right puts its points
                // back to the others' size; neither moves the view
                if let Some(i) = row(&self.rows, self.cursor).filter(|_| pressed) {
                    match b {
                        Some(MouseButton::Left) => self.toggle(i),
                        Some(MouseButton::Right) => {
                            if let Some(l) = layer(&self.files, i) {
                                let s = gl.scene.sizes().get(l).copied().unwrap_or(1.0);
                                gl.scene.resize(l, 1.0 / s, 0.0, f32::MAX);
                            }
                        }
                        _ => {}
                    }
                    self.button = None;
                    return self.request_redraw();
                }
                self.button = b.filter(|_| pressed);
                // Shift + click picks, Shift + right click takes the last pick off
                if let Some((click, at)) = shift_click(&mut self.press, self.cursor, self.mods.shift, b, pressed) {
                    self.clicked(click, at);
                    return self.request_redraw();
                }
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
                let on_row = |c| (row(&self.rows, c), row(&self.panel_rows, c));
                let was = on_row(self.cursor);
                self.cursor = Some((p.x, p.y));
                // the row under the cursor is highlighted: the one the wheel and clicks act on
                let now = on_row(self.cursor);
                if now != was {
                    gl.window.set_cursor(if now == (None, None) {
                        CursorIcon::Default
                    } else {
                        CursorIcon::Pointer
                    });
                    gl.window.request_redraw();
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let y = wheel(delta, dpr);
                // the wheel on the picks panel scrolls it, a row a step
                if row(&self.panel_rows, self.cursor).is_some() {
                    self.panel_top = pick::scroll(self.picks.len(), self.panel_top, PANEL_ROWS, y > 0.0);
                    return self.request_redraw();
                }
                // the wheel on a legend row sizes that file's points, not the view
                if let Some(i) = row(&self.rows, self.cursor) {
                    if let Some(l) = layer(&self.files, i) {
                        let by = POINT_STEP.powf(y / WHEEL_NOTCH);
                        gl.scene.resize(l, by, POINT_MIN / self.psize, POINT_MAX / self.psize);
                    }
                    return self.request_redraw();
                }
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

/// What's drawn over the scene; egui gets no input, the window hit-tests the
/// legend's rows itself. Top left, with `legend`: each file with its tint,
/// its size and its points' size if not the others', the ones turned off,
/// still loading or failed dimmed, the one under the cursor (`hover`) in
/// bold. Top right, with `help`, the keys; else a hint bottom left that H
/// shows them. Returns the legend's rows and their file, empty without it.
fn overlay(
    ui: &egui::Ui,
    files: &[File],
    visible: &[bool],
    sizes: &[f32],
    hover: Option<usize>,
    legend: bool,
    help: bool,
) -> Vec<(egui::Rect, usize)> {
    let panel = |id: &str, at: egui::Align2, off: [f32; 2], add: &mut dyn FnMut(&mut egui::Ui)| {
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
        panel("help", egui::Align2::RIGHT_TOP, [-12.0, 12.0], &mut |ui| {
            // the help's lines: a group's name alone, then its keys, two
            // spaces or more between a key and what it does
            egui::Grid::new("keys").spacing([16.0, 4.0]).show(ui, |ui| {
                for (i, l) in window_keys().lines().skip(1).map(str::trim).enumerate() {
                    if let Some((k, d)) = l.split_once("  ") {
                        ui.strong(k);
                        ui.label(d.trim());
                    } else {
                        let top = if i == 0 { 0.0 } else { 8.0 };
                        ui.vertical(|ui| {
                            ui.add_space(top);
                            ui.label(egui::RichText::new(l).small().weak());
                        });
                    }
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
    let mut rows = vec![];
    if !legend {
        return rows;
    }
    panel("legend", egui::Align2::LEFT_TOP, [12.0, 12.0], &mut |ui| {
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
                        let size = match sizes.get(*layer) {
                            Some(s) if (s - 1.0).abs() > 0.01 => format!(", points ×{s:.1}"),
                            _ => String::new(),
                        };
                        let on = visible.get(*layer).copied().unwrap_or(true);
                        (on, *tint, format!("{count} {unit}{size}{more}"))
                    }
                    State::Loading => (false, None, "loading…".to_owned()),
                    State::Failed => (false, None, "failed".to_owned()),
                };
                let alpha = if on { 1.0 } else { OFF_ALPHA };
                let fg = ui.visuals().text_color().gamma_multiply(alpha);
                let text = |t: String| egui::RichText::new(t).color(fg);
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
                let name = text(f.name.clone());
                ui.label(if hover == Some(i) { name.strong().underline() } else { name });
                let last = ui.label(text(count));
                // half the grid's spacing above and below: no gap between rows
                rows.push((r.union(last.rect).expand2(egui::vec2(0.0, 2.0)), i));
                ui.end_row();
            }
        });
    });
    rows
}

/// What the picks panel shows: its rows in view (place in the picks, the
/// row's cells, the file's name if several, whether its file is shown) and
/// how many picks are out of view above and below.
#[derive(Default)]
struct PanelView {
    rows: Vec<(usize, [String; 6], Option<String>, bool)>,
    above: usize,
    below: usize,
}

/// The picks panel, bottom right, while there are picks and it's open
/// (`view` empty otherwise): each pick in view with its number, index, x y z
/// (right-aligned) and file, those of hidden files dimmed, the one under the
/// cursor (`over`) in bold, and how many are scrolled out; below, what the
/// last copy did (`status`), or how to copy. Returns the rows and their pick.
fn picks_panel(ui: &egui::Ui, view: &PanelView, over: Option<usize>, status: Option<&str>) -> Vec<(egui::Rect, usize)> {
    let mut rows = vec![];
    if view.rows.is_empty() {
        return rows;
    }
    egui::Area::new(egui::Id::new("picks panel"))
        .anchor(egui::Align2::RIGHT_BOTTOM, [-12.0, -12.0])
        .interactable(false)
        .show(ui.ctx(), |ui| {
            let fill = ui.visuals().window_fill.gamma_multiply(PANEL_ALPHA);
            egui::Frame::popup(ui.style()).fill(fill).show(ui, |ui| {
                if view.above > 0 {
                    ui.weak(format!("↑ {} more", view.above));
                }
                egui::Grid::new("picks").min_col_width(0.0).spacing([10.0, 4.0]).show(ui, |ui| {
                    for (n, cells, name, on) in &view.rows {
                        let fg = ui.visuals().text_color().gamma_multiply(if *on { 1.0 } else { OFF_ALPHA });
                        let text = |t: &str| {
                            let t = egui::RichText::new(t).color(fg);
                            if over == Some(*n) { t.strong().underline() } else { t }
                        };
                        let mut last = ui.label(text(&cells[0]).strong());
                        let first = last.rect;
                        last = ui.label(text(&cells[1]));
                        for c in &cells[2..] {
                            // coordinates and distance right-aligned, so their digits line up
                            last = ui
                                .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(text(c).monospace()))
                                .inner;
                        }
                        if let Some(name) = name {
                            last = ui.label(text(name));
                        }
                        rows.push((first.union(last.rect).expand2(egui::vec2(0.0, 2.0)), *n));
                        ui.end_row();
                    }
                });
                if view.below > 0 {
                    ui.weak(format!("↓ {} more", view.below));
                }
                ui.weak(status.unwrap_or("click a row or Ctrl+C to copy"));
            });
        });
    rows
}

/// The picks' markers, over the scene and under the panels: a ring and a dot, outlined so they
/// read on any color, and the pick's number; the one whose panel row is
/// under the cursor (`over`, 0-based) bigger, in the accent color.
fn draw_marks(ui: &egui::Ui, marks: &[(usize, (f32, f32))], over: Option<usize>) {
    let painter = ui
        .ctx()
        .layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("picks")));
    let (text, halo, accent) = (
        ui.visuals().strong_text_color(),
        ui.visuals().window_fill,
        ui.visuals().selection.bg_fill,
    );
    for &(n, (x, y)) in marks {
        let c = egui::pos2(x, y);
        let lit = over == n.checked_sub(1);
        let (fg, r, w) = if lit { (accent, 10.0, 2.5_f32) } else { (text, 6.0, 1.5_f32) };
        painter.circle_stroke(c, r, egui::Stroke::new(w + 2.0, halo));
        painter.circle_stroke(c, r, egui::Stroke::new(w, fg));
        painter.circle_filled(c, 1.5, fg);
        let at = c + egui::vec2(r + 2.0, -r);
        let font = egui::FontId::proportional(13.0);
        // the number outlined too: drawn in the halo color around, then on top
        for (dx, dy) in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)] {
            painter.text(
                at + egui::vec2(dx, dy),
                egui::Align2::LEFT_BOTTOM,
                n.to_string(),
                font.clone(),
                halo,
            );
        }
        painter.text(at, egui::Align2::LEFT_BOTTOM, n.to_string(), font, fg);
    }
}

/// A Shift click from the mouse buttons' events: the button and where it
/// was kept in `press` when pressed; on release, where it ended, if Shift is
/// still down, it's the same button and it moved too little to be a drag (a
/// Shift drag pans). Only left and right count.
fn shift_click(
    press: &mut Option<(MouseButton, (f32, f32))>,
    cursor: Option<(f32, f32)>,
    shift: bool,
    b: Option<MouseButton>,
    pressed: bool,
) -> Option<(MouseButton, (f32, f32))> {
    if pressed {
        // every press starts over: one without Shift leaves nothing behind
        *press = b
            .zip(cursor)
            .filter(|(b, _)| shift && matches!(b, MouseButton::Left | MouseButton::Right));
        return None;
    }
    let (down, from) = press.take()?;
    let to = cursor?;
    (shift && b == Some(down) && pick::is_click(from, to)).then_some((down, to))
}

/// A wheel turn in three-d's wheel units, up positive.
fn wheel(delta: MouseScrollDelta, dpr: f32) -> f32 {
    match delta {
        MouseScrollDelta::LineDelta(_, y) => y * WHEEL_NOTCH,
        MouseScrollDelta::PixelDelta(d) => d.to_logical::<f32>(f64::from(dpr)).y * WHEEL_PIXEL,
    }
}

/// The buttons that move the view, click legend rows and pick.
fn mouse_button(b: WinitButton) -> Option<MouseButton> {
    #[expect(clippy::wildcard_enum_match_arm, reason = "the three buttons that move the view")]
    match b {
        WinitButton::Left => Some(MouseButton::Left),
        WinitButton::Right => Some(MouseButton::Right),
        WinitButton::Middle => Some(MouseButton::Middle),
        _ => None,
    }
}

/// The scene's layer of the i-th file, once shown: files come in as they're read.
fn layer(files: &[File], i: usize) -> Option<usize> {
    match files.get(i)?.state {
        State::Shown { layer, .. } => Some(layer),
        State::Loading | State::Failed => None,
    }
}

/// The file whose legend row is under the cursor.
fn row(rows: &[(egui::Rect, usize)], cursor: Option<(f32, f32)>) -> Option<usize> {
    let (x, y) = cursor?;
    rows.iter().find(|(r, _)| r.contains(egui::pos2(x, y))).map(|(_, i)| *i)
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

/// The window's size in logical pixels, as the cursor is.
fn logical_size(w: &Window) -> (f32, f32) {
    let s = w.inner_size().to_logical::<f32>(w.scale_factor());
    (s.width, s.height)
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

    #[test]
    fn shift_click_not_drag() {
        let (mut press, l) = (None, Some(MouseButton::Left));
        assert_eq!(shift_click(&mut press, Some((10.0, 10.0)), true, l, true), None);
        assert_eq!(
            shift_click(&mut press, Some((11.0, 12.0)), true, l, false),
            Some((MouseButton::Left, (11.0, 12.0)))
        );
        // a drag pans, and without Shift (or with the middle button) nothing picks
        shift_click(&mut press, Some((10.0, 10.0)), true, l, true);
        assert_eq!(shift_click(&mut press, Some((40.0, 10.0)), true, l, false), None);
        shift_click(&mut press, Some((10.0, 10.0)), false, l, true);
        assert_eq!(shift_click(&mut press, Some((10.0, 10.0)), false, l, false), None);
        let m = Some(MouseButton::Middle);
        shift_click(&mut press, Some((10.0, 10.0)), true, m, true);
        assert_eq!(shift_click(&mut press, Some((10.0, 10.0)), true, m, false), None);
    }

    #[test]
    fn shift_click_needs_shift_on_the_same_button_throughout() {
        let (mut press, l, r) = (None, Some(MouseButton::Left), Some(MouseButton::Right));
        // pressed without Shift, released with it: no pick
        shift_click(&mut press, Some((10.0, 10.0)), true, l, true);
        shift_click(&mut press, Some((10.0, 10.0)), false, l, false);
        shift_click(&mut press, Some((50.0, 50.0)), false, l, true);
        assert_eq!(shift_click(&mut press, Some((10.0, 10.0)), true, l, false), None);
        // Shift + left pressed, right released: neither picks nor removes
        shift_click(&mut press, Some((10.0, 10.0)), true, l, true);
        assert_eq!(shift_click(&mut press, Some((10.0, 10.0)), true, r, false), None);
    }
}

// }}}
