//! The event loop: the window opened with the first file read, the files
//! added as they come, the keys and the mouse.

// ========================================== Imports ========================================== {{{

use glutin::prelude::*;
use glutin_winit::GlWindow;
use std::{
    error::Error,
    fmt,
    io::Write,
    iter, mem,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use three_d::{ClearState, Event, Modifiers, MouseButton, PhysicalPoint, RenderTarget, Vec3, egui};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, MouseButton as WinitButton, MouseScrollDelta, StartCause, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow},
    keyboard::{Key, NamedKey},
    platform::run_on_demand::EventLoopExtRunOnDemand,
    window::{CursorIcon, Window, WindowId},
};

use super::{
    Shown, ViewError, ViewResult, Viewer,
    load::{File, Generation, Loaded, Ready, Source, State, load_all},
    navigate::{WHEEL_NOTCH, WHEEL_PIXEL, ZOOM_MAX, ZOOM_MIN, navigate},
    overlay::{PanelView, draw_marks, overlay, picks_panel, row},
    window::{Gl, scale, viewport, with_event_loop},
};
use crate::{
    Cloud, Input, LoadError, LoadResult, Theme, is_gltf,
    pick::{self, Pick},
    up_for,
};

// }}}

// ========================================= Constants ========================================= {{{

/// Point size in logical pixels: the start, the factor of + and -, the bounds.
const POINT_SIZE: f32 = 2.0;

const POINT_STEP: f32 = 1.25;

const POINT_MIN: f32 = 1.0;

const POINT_MAX: f32 = 20.0;

/// How long the picks panel says it copied.
const COPIED_FOR: Duration = Duration::from_secs(2);

/// The picks panel's rows at most: the wheel scrolls the rest.
const PANEL_ROWS: usize = 8;

/// A file's name in the picks panel, characters at most.
const NAME_MAX: usize = 28;

// }}}

// ============================================ App ============================================ {{{

/// Opens the files in one window and blocks until it's closed. The files
/// load in parallel and show up as they're read, a STEP file solid by solid:
/// the window opens with the first piece, so a slow file holds back nothing
/// but itself.
pub(crate) fn run(v: Viewer) -> ViewResult<Shown> {
    let Viewer {
        sources,
        theme,
        app_id,
        mut out,
    } = v;
    let files: Vec<File> = sources
        .iter()
        .map(|s| File {
            name: s.name(),
            state: State::Loading,
        })
        .collect();
    if files.len() > 1 {
        let keys: Vec<String> = files
            .iter()
            .enumerate()
            .take(9)
            .map(|(i, f)| format!("{} {}", i + 1, f.name))
            .collect();
        tell(&mut *out, format_args!("keys: {}", keys.join(", ")));
    }
    let paths: Vec<String> = sources.iter().map(Source::given).collect();
    let up = up_for(sources.iter().all(|s| matches!(s, Source::File(p) if is_gltf(p))));
    let generation = Generation::next();
    let mut app = App {
        generation,
        pending: files.len(),
        legend: files.len() > 1,
        clouds: paths.iter().map(|_| None).collect(),
        paths,
        picks: vec![],
        press: None,
        files,
        theme,
        app_id,
        up,
        out,
        gl: None,
        err: None,
        failed: vec![],
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
    let stop = Arc::new(AtomicBool::new(false));
    let ran = with_event_loop(|events| {
        load_all(sources, theme, generation, &stop, &events.create_proxy());
        let ran = events.run_app_on_demand(&mut app).map_err(ViewError::EventLoop);
        // the window is gone, however it went: its loaders stop, a STEP helper is killed
        stop.store(true, Ordering::Relaxed);
        ran
    });
    ran?;
    app.err.map_or(Ok(Shown { failed: app.failed }), Err)
}

/// Our own window (winit 0.30 + glutin) instead of three-d's, which pins
/// winit 0.28: on GNOME, where winit draws the title bar itself, at scale 2
/// that one sends a 45 px high buffer and the compositor kills the window.
/// three-d only gets the GL context.
#[expect(
    clippy::struct_excessive_bools,
    reason = "the panels and the view's state: independent flags, not a state machine"
)]
struct App {
    /// the `show()` this window is: a loader event of another is dropped
    generation: Generation,
    files: Vec<File>,
    /// the files as given (a cloud's name for one from memory): what the pick lines name
    paths: Vec<String>,
    /// each file's points, if it's a cloud and shown: what Shift + click picks from
    clouds: Vec<Option<Cloud>>,
    /// the points picked, in order: Shift + right click takes the last off
    picks: Vec<Pick>,
    /// the button and where it was pressed with Shift, logical pixels
    press: Option<(MouseButton, (f32, f32))>,
    /// files not read yet
    pending: usize,
    theme: &'static Theme,
    /// the Wayland `app_id` the dock groups the window by
    #[expect(clippy::struct_field_names, reason = "winit and the builder call it app_id")]
    app_id: String,
    up: Vec3,
    /// stdout: the files turned off and on, the points picked and removed
    out: Box<dyn Write>,
    gl: Option<Gl>,
    /// what closed the window, if not the user
    err: Option<ViewError>,
    /// the files that failed, and why
    failed: Vec<(String, LoadError)>,
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

impl App {
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
    fn done(&mut self, event_loop: &ActiveEventLoop, i: usize, result: LoadResult<usize>) {
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
                    warn!("{}: {}", f.name, chain(&e));
                    *state = State::Failed;
                    self.failed.push((f.name.clone(), e));
                }
                // a load that went well sent at least one piece
                (Ok(_), _) => {}
            }
        }
        if self.pending == 0 && self.gl.is_none() {
            self.err = Some(ViewError::NothingOpened {
                failed: mem::take(&mut self.failed),
            });
            event_loop.exit();
        }
        self.request_redraw();
    }

    /// Puts a piece in the scene, in layer `to` if its file has one already
    /// (a STEP file's next solid); returns the layer.
    fn show(&mut self, event_loop: &ActiveEventLoop, to: Option<usize>, input: Input) -> ViewResult<usize> {
        let Some(gl) = self.gl.as_mut() else {
            self.gl = Some(Gl::open(event_loop, &self.files, &self.app_id, self.theme, self.up, input)?);
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
            at: at.into(),
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
        if let Err(source) = gl.surface.swap_buffers(&gl.ctx) {
            self.err = Some(ViewError::Gl {
                step: "show the frame",
                source,
            });
            event_loop.exit();
        }
    }
}

impl ApplicationHandler<Loaded> for App {
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
        // a closed window's loader, late: the loop outlived it
        if !event.is_for(self.generation) {
            return;
        }
        match event {
            Loaded::Part { i, ready, .. } => self.part(event_loop, i, ready),
            Loaded::Done { i, result, .. } => self.done(event_loop, i, result),
        }
    }

    // run_app_on_demand returns with the loop alive: the window must go now
    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.gl = None;
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

// ========================================== Helpers ========================================== {{{

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

/// The window's size in logical pixels, as the cursor is.
fn logical_size(w: &Window) -> (f32, f32) {
    let s = w.inner_size().to_logical::<f32>(w.scale_factor());
    (s.width, s.height)
}

/// A line for the user on `out` (stdout): what tridi read, what it did.
fn tell(out: &mut dyn Write, line: fmt::Arguments) {
    #[expect(clippy::let_underscore_must_use, reason = "a closed stdout (a pipe to head) is no reason to stop")]
    let _ = writeln!(out, "{line}");
}

/// An error and its causes on one line, "a: b: c", as the binary prints them.
fn chain(e: &(dyn Error + 'static)) -> String {
    iter::successors(Some(e), |&e| e.source())
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(": ")
}

// }}}

// =========================================== Tests =========================================== {{{

#[cfg(test)]
mod tests {
    use super::*;

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
