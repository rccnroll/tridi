//! Drawing: the scene, and for thumbnails the headless GL context and the
//! offscreen render.

// ======================================== Sub-modules ======================================== {{{

#[cfg(feature = "thumbnail")]
mod headless;
#[cfg(feature = "thumbnail")]
mod offscreen;
mod scene;

// }}}

// ======================================== Re-exports ========================================= {{{

#[cfg(feature = "thumbnail")]
pub use headless::headless;
#[cfg(feature = "thumbnail")]
pub use offscreen::offscreen;
pub use scene::{FOV, Input, Scene};

// }}}

// ========================================== Imports ========================================== {{{

use glutin::error;
use std::env;

// }}}

// ========================================== Errors =========================================== {{{

/// Why drawing couldn't start.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
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

// ======================================= Quiet stderr ======================================== {{{

/// Runs `f` with stderr closed. Mesa prints `pci id for fd N: 10de:…, driver
/// (null)` for every GPU node it has no driver for (the NVIDIA one) while
/// EGL starts, before any of its log settings apply. Real failures still come
/// back through `f`'s result.
pub fn quiet_stderr<T, F: FnOnce() -> T>(f: F) -> T {
    if env::var_os("TRIDI_DEBUG").is_some() {
        return f();
    }
    // SAFETY: plain fd juggling on 2; the saved copy is restored and closed
    unsafe {
        let saved = libc::dup(2);
        let null = libc::open(c"/dev/null".as_ptr(), libc::O_WRONLY);
        if saved < 0 || null < 0 {
            return f();
        }
        libc::dup2(null, 2);
        libc::close(null);
        let r = f();
        libc::dup2(saved, 2);
        libc::close(saved);
        r
    }
}

// }}}
