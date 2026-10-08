//! Drawing: the scene, the headless GL context the thumbnailer uses, and the
//! offscreen render.

mod headless;
mod offscreen;
mod scene;

/// Why drawing couldn't start.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("no usable EGL device")]
    NoDevice,
    #[error("no EGL config")]
    NoConfig,
    #[error("EGL: cannot {step}")]
    Egl {
        step: &'static str,
        #[source]
        source: glutin::error::Error,
    },
    #[error("GL: cannot {step}")]
    Gl {
        step: &'static str,
        #[source]
        source: three_d::CoreError,
    },
    #[error("cannot upload a mesh")]
    Mesh(#[source] three_d::RendererError),
}

pub type RenderResult<T> = Result<T, RenderError>;

pub use headless::{headless, quiet_stderr};
pub use offscreen::offscreen;
pub use scene::{FOV, Input, Scene};
