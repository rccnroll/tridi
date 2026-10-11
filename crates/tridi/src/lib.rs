//! tridi's viewer as a library: point clouds, meshes and STEP files in a
//! window, with [`show`] or a [`Viewer`]. The `tridi` command (the
//! `tridi-cli` crate) is this window plus the thumbnailer and the desktop
//! integration.

// ======================================= Lint Settings ======================================= {{{

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::indexing_slicing,
        reason = "a test may panic: that is its failure"
    )
)]

// }}}

// ======================================== Sub-modules ======================================== {{{

mod color;
mod model;
mod render;
#[cfg(test)]
mod test_support;
mod theme;
#[cfg(feature = "thumbnail")]
mod thumbnail;
mod viewer;

// }}}

// ======================================== Re-exports ========================================= {{{

pub use model::LoadError;
pub use render::RenderError;
pub use theme::{DARK, LIGHT, THEMES, Theme};
#[cfg(feature = "thumbnail")]
#[doc(hidden)]
pub use thumbnail::{Thumbnail, ThumbnailError, ThumbnailResult, thumbnail};
pub use tridi_formats::{Cloud, FormatError, FormatResult, load_cloud, parse_cloud};
#[cfg(feature = "step")]
pub use tridi_step::{StepError, StepResult};
pub use viewer::{APP_ID, KEYS, Shown, ViewError, ViewResult, Viewer, show};

// for the modules: `crate::Scene`, not `crate::render::Scene`
use color::{colorize, own_colors};
use model::{Item, LoadResult, Purpose, is_gltf, load, triangles, up_for};
use render::{FOV, Input, Scene, px_f32, quiet_stderr};

// }}}

// ========================================== Imports ========================================== {{{

#[macro_use]
extern crate tracing;

// }}}
