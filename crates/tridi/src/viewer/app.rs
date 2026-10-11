//! The event loop: the window opened with the first file read, the files
//! added as they come, the keys and the mouse.

// ========================================== Imports ========================================== {{{

use glutin::prelude::*;
use glutin_winit::GlWindow;
use std::{error::Error, fmt, io::Write, iter};
use three_d::{ClearState, Event, Modifiers, MouseButton, PhysicalPoint, RenderTarget, Vec3};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, MouseButton as WinitButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{Key, NamedKey},
    platform::wayland::EventLoopBuilderExtWayland,
    window::WindowId,
};

use super::{
    Shown, ViewError, ViewResult, Viewer,
    load::{File, Loaded, Ready, State, label, load_all},
    navigate::{WHEEL_NOTCH, WHEEL_PIXEL, ZOOM_MAX, ZOOM_MIN, navigate},
    overlay::overlay,
    window::{Gl, scale, viewport},
};
use crate::{Input, LoadError, LoadResult, Theme, is_gltf, up_for};

// }}}

// ========================================= Constants ========================================= {{{

/// Point size in logical pixels: the start, the factor of + and -, the bounds.
const POINT_SIZE: f32 = 2.0;

const POINT_STEP: f32 = 1.25;

const POINT_MIN: f32 = 1.0;

const POINT_MAX: f32 = 20.0;

// }}}

// ============================================ App ============================================ {{{

/// Opens the files in one window and blocks until it's closed. The files
/// load in parallel and show up as they're read, a STEP file solid by solid:
/// the window opens with the first piece, so a slow file holds back nothing
/// but itself.
pub(crate) fn run(v: Viewer) -> ViewResult<Shown> {
    let Viewer {
        paths,
        theme,
        app_id,
        mut out,
    } = v;
    let files: Vec<File> = paths
        .iter()
        .map(|p| File {
            name: label(p),
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
    let mut builder = EventLoop::<Loaded>::with_user_event();
    // a library may be called off the main thread (a test is): Linux allows it
    EventLoopBuilderExtWayland::with_any_thread(&mut builder, true);
    let event_loop = builder.build().map_err(ViewError::NoDisplay)?;
    let up = up_for(paths.iter().all(|p| is_gltf(p)));
    load_all(&paths, theme, &event_loop.create_proxy());
    let mut app = App {
        pending: files.len(),
        legend: files.len() > 1,
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
        moved: false,
        cursor: None,
        button: None,
        mods: Modifiers::default(),
    };
    event_loop.run_app(&mut app).map_err(ViewError::EventLoop)?;
    app.err.map_or(Ok(Shown { failed: app.failed }), Err)
}

/// Our own window (winit 0.30 + glutin) instead of three-d's, which pins
/// winit 0.28: on GNOME, where winit draws the title bar itself, at scale 2
/// that one sends a 45 px high buffer and the compositor kills the window.
/// three-d only gets the GL context.
struct App {
    files: Vec<File>,
    /// files not read yet
    pending: usize,
    theme: &'static Theme,
    /// the Wayland `app_id` the dock groups the window by
    #[expect(clippy::struct_field_names, reason = "winit and the builder call it app_id")]
    app_id: String,
    up: Vec3,
    /// stdout: the files turned off and on
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
            // each file that failed was reported as it was read
            self.err = Some(ViewError::NothingOpened);
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

// ========================================== Helpers ========================================== {{{

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
