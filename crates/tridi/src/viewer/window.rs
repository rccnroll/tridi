//! The window, its GL surface and what's drawn in it.

// ========================================== Imports ========================================== {{{

use glutin::{
    config::ConfigTemplateBuilder,
    context::{ContextAttributesBuilder, PossiblyCurrentContext},
    display::GetGlDisplay,
    prelude::*,
    surface::{Surface, SurfaceAttributesBuilder, SwapInterval, WindowSurface},
};
use glutin_winit::{DisplayBuilder, GlWindow};
use std::{num::NonZeroU32, sync::Arc, time::Instant};
use three_d::{Camera, Context, GUI, HasContext, Vec3, Viewport, context, egui};
use winit::{
    event_loop::ActiveEventLoop, platform::wayland::WindowAttributesExtWayland, raw_window_handle::HasWindowHandle, window::Window,
};

use super::{ViewError, ViewResult, load::File};
use crate::{Input, RenderError, Scene, Theme, quiet_stderr};

// }}}

// ========================================== Window =========================================== {{{

/// The window, its GL surface and what's drawn in it: made once the event
/// loop starts, as winit 0.30 wants.
pub(crate) struct Gl {
    pub(crate) scene: Scene,
    /// draws the legend and the keys
    pub(crate) gui: GUI,
    /// egui's clock, for its fade-in
    pub(crate) start: Instant,
    pub(crate) cam: Camera,
    pub(crate) context: Context,
    pub(crate) ctx: PossiblyCurrentContext,
    pub(crate) surface: Surface<WindowSurface>,
    pub(crate) window: Window,
}

impl Gl {
    /// Opens the window with the first file read.
    pub(crate) fn open(
        event_loop: &ActiveEventLoop,
        files: &[File],
        app_id: &str,
        theme: &Theme,
        up: Vec3,
        input: Input,
    ) -> ViewResult<Self> {
        let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
        let attrs = Window::default_attributes()
            .with_title(format!("tridi — {}", names.join(", ")))
            // the app_id the dock groups the window by, matching the .desktop
            .with_name(app_id, app_id);
        // 4x MSAA if there is one, as three-d's window had
        let tmpl = ConfigTemplateBuilder::new().with_depth_size(24);
        let (window, config) = quiet_stderr(|| {
            DisplayBuilder::new()
                .with_window_attributes(Some(attrs))
                .build(event_loop, tmpl, |cs| {
                    #[expect(clippy::expect_used, reason = "glutin calls the picker with at least one config")]
                    cs.max_by_key(|c| c.num_samples().min(4)).expect("no GL config")
                })
        })
        // glutin-winit's error is a Box<dyn Error> without Send: keep its text
        .map_err(|e| ViewError::Window { message: e.to_string() })?;
        let window = window.ok_or_else(|| ViewError::Window {
            message: "no window was made".to_owned(),
        })?;
        let display = config.display();
        let handle = window
            .window_handle()
            .map_err(|e| ViewError::Window {
                message: format!("no window handle: {e}"),
            })?
            .as_raw();
        let attrs = ContextAttributesBuilder::new().build(Some(handle));
        let sattrs = window
            .build_surface_attributes(SurfaceAttributesBuilder::default())
            .map_err(|e| ViewError::Window {
                message: format!("no surface: {e}"),
            })?;
        let gl_err = |step| move |source| ViewError::Gl { step, source };
        // SAFETY: the config, the window handle and the surface attributes
        // all come from this display and this window, which outlive them
        let (ctx, surface) = unsafe {
            let ctx = display.create_context(&config, &attrs).map_err(gl_err("create the context"))?;
            let surface = display
                .create_window_surface(&config, &sattrs)
                .map_err(gl_err("create the surface"))?;
            (ctx.make_current(&surface).map_err(gl_err("make the context current"))?, surface)
        };
        if let Err(e) = surface.set_swap_interval(&ctx, SwapInterval::Wait(NonZeroU32::MIN)) {
            debug!("no vsync: {e}");
        }
        // SAFETY: the context made current above, whose functions EGL returns
        let gl = unsafe { context::Context::from_loader_function_cstr(|s| display.get_proc_address(s)) };
        let context = Context::from_gl_context(Arc::new(gl)).map_err(|source| RenderError::Gl { step: "load GL", source })?;
        // SAFETY: a plain query on the current context
        debug!(renderer = unsafe { context.get_parameter_string(context::RENDERER) }, "GL");
        let scene = Scene::new(&context, vec![input], true, up)?;
        let cam = scene.camera(viewport(&window));
        let gui = GUI::new(&context);
        let dark = theme.name == "dark";
        gui.context()
            .set_visuals(if dark { egui::Visuals::dark() } else { egui::Visuals::light() });
        Ok(Self {
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
}

// }}}

// ========================================== Helpers ========================================== {{{

/// The window's scale factor, for GL's float math.
pub(crate) fn scale(w: &Window) -> f32 {
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        reason = "a scale factor is a small number"
    )]
    let s = w.scale_factor() as f32;
    s
}

pub(crate) fn viewport(w: &Window) -> Viewport {
    let s = w.inner_size();
    Viewport::new_at_origo(s.width.max(1), s.height.max(1))
}

// }}}

// =========================================== Tests =========================================== {{{

#[cfg(test)]
mod tests {
    // 7. the dock's app_id is a constant matching the .desktop and the
    //    metainfo: if it follows the title or the file name, the dock goes
    //    back to "unknown"; the title says tridi, not the app id
    #[test]
    fn app_id_is_fixed() {
        assert_eq!(crate::APP_ID, "io.github.rccnroll.tridi");
        let src = include_str!("window.rs");
        assert!(src.contains(".with_name(app_id, app_id)"), "the window's app_id");
        assert!(src.contains(r#".with_title(format!("tridi — {}""#), "the title");
        let meta = include_str!("../../../../share/metainfo/io.github.rccnroll.tridi.metainfo.xml");
        assert!(meta.contains("<id>io.github.rccnroll.tridi</id>"), "the metainfo id");
        assert!(
            meta.contains(r#"<launchable type="desktop-id">io.github.rccnroll.tridi.desktop</launchable>"#),
            "the metainfo's desktop entry"
        );
    }
}

// }}}
