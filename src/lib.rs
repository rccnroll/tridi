//! tridi's library: file readers, the scene and its rendering. The binary
//! (`main.rs`) is the window, the thumbnailer command and the desktop
//! integration on top. Not a stable API: it exists for the binary (the fuzz
//! targets include `formats/` alone).

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::indexing_slicing,
        reason = "a test may panic: that is its failure"
    )
)]

mod color;
mod formats;
mod model;
mod render;
mod step;
#[cfg(test)]
mod test_support;
mod theme;

pub use color::{colorize, own_colors};
pub use formats::{Cloud, FormatError, FormatResult, load_cloud, parse_cloud};
pub use model::{Item, LoadError, LoadResult, is_gltf, load, triangles, up_for};
pub use render::{FOV, Input, RenderError, RenderResult, Scene, headless, offscreen, px_f32, quiet_stderr};
pub use step::{StepError, StepResult, mesh_to_stdout as step_mesh_to_stdout};
pub use theme::{DARK, LIGHT, THEMES, Theme};
