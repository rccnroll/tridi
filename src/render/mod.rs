//! Drawing: the scene, the headless GL context the thumbnailer uses, and the
//! offscreen render.

// ======================================== Sub-modules ======================================== {{{

mod headless;
mod offscreen;
mod scene;

// }}}

// ======================================== Re-exports ========================================= {{{

pub use headless::{headless, quiet_stderr};
pub use offscreen::offscreen;
pub use scene::{FOV, Input, Scene};

// }}}

// ========================================== Imports ========================================== {{{

use glutin::error;

// }}}

// ========================================== Errors =========================================== {{{

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
        source: error::Error,
    },
    #[error("GL: cannot {step}")]
    Gl {
        step: &'static str,
        #[source]
        source: three_d::CoreError,
    },
    #[error("cannot upload a mesh")]
    Mesh(#[source] three_d::RendererError),
    /// GL counts vertices in an i32.
    #[error("{count} points are more than GL can draw at once")]
    TooManyPoints { count: usize },
}

pub type RenderResult<T> = Result<T, RenderError>;

// }}}

// ========================================== Helpers ========================================== {{{

/// A pixel count as f32, for GL's float math.
#[must_use]
pub fn px_f32(n: u32) -> f32 {
    #[expect(clippy::as_conversions, clippy::cast_precision_loss, reason = "image sizes are far below 2^24")]
    let n = n as f32;
    n
}

// }}}
