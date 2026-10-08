//! Drawing: the scene, the headless GL context the thumbnailer uses, and the
//! offscreen render.

mod headless;
mod offscreen;
mod scene;

pub use headless::{headless, quiet_stderr};
pub use offscreen::offscreen;
pub use scene::{FOV, Input, Scene};
