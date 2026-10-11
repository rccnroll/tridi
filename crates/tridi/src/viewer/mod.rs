//! The window: files read in parallel and shown as they come, mouse
//! navigation, the keys, the legend. One call, [`show`], or a [`Viewer`]
//! to set it up first; both block until the window is closed.

// ======================================== Sub-modules ======================================== {{{

mod app;
mod load;
mod navigate;
mod overlay;
mod window;

// }}}

// ========================================== Imports ========================================== {{{

use glutin::error;
use std::{
    io::{self, Write},
    path::{Path, PathBuf},
};
use winit::error::EventLoopError;

use crate::{LIGHT, LoadError, RenderError, Theme};

// }}}

// ========================================= Constants ========================================= {{{

/// The Wayland `app_id` the dock groups the window by, matching tridi's .desktop.
pub const APP_ID: &str = "io.github.rccnroll.tridi";

/// The keys, as `tridi --help` lists them and H shows them in the window.
pub const KEYS: &str = "\
Window:
  left drag             orbit
  right or middle drag  pan (or shift + left drag)
  wheel                 zoom
  R                     reset the view
  + and -               point size
  1-9                   turn the N-th file off and on
  I                     show or hide the legend (on with several files)
  H or ?                show or hide these keys
  Q or Esc              quit";

// }}}

// ========================================== Errors =========================================== {{{

/// Why the window couldn't show.
#[derive(Debug, thiserror::Error)]
pub enum ViewError {
    #[error("no file to show")]
    Empty,
    /// No display to open a window on. It holds for the rest of the
    /// process: winit allows one event loop per process, and counts this try.
    #[error("no display")]
    NoDisplay(#[source] EventLoopError),
    #[error("cannot open the window: {message}")]
    Window { message: String },
    #[error("GL: cannot {step}")]
    Gl {
        step: &'static str,
        #[source]
        source: error::Error,
    },
    #[error(transparent)]
    Render(#[from] RenderError),
    /// winit allows one event loop per process: show windows from the
    /// thread that showed the first one. Once that thread has ended, no
    /// thread can show a window again in this process.
    #[error("windows can only be shown from the thread that showed the first one")]
    OtherThread,
    #[error("the window's event loop failed")]
    EventLoop(#[source] EventLoopError),
    /// every file failed; each one's error is in the log
    #[error("no file could be opened")]
    NothingOpened,
}

pub type ViewResult<T> = Result<T, ViewError>;

// }}}

// =========================================== Shown =========================================== {{{

/// What the window showed: the files that failed, and why.
#[derive(Debug)]
pub struct Shown {
    pub failed: Vec<(String, LoadError)>,
}

// }}}

// ========================================== Viewer =========================================== {{{

/// Sets up the window: files, theme, app id, where the per-file lines go.
pub struct Viewer {
    paths: Vec<PathBuf>,
    theme: &'static Theme,
    app_id: String,
    out: Box<dyn Write>,
}

impl Default for Viewer {
    fn default() -> Self {
        Self::new()
    }
}

impl Viewer {
    /// No files yet, the light theme, tridi's app id, the lines nowhere.
    #[must_use]
    pub fn new() -> Self {
        Self {
            paths: vec![],
            theme: &LIGHT,
            app_id: APP_ID.to_owned(),
            out: Box::new(io::sink()),
        }
    }

    #[must_use]
    pub fn file<P: AsRef<Path>>(mut self, path: P) -> Self {
        self.paths.push(path.as_ref().to_owned());
        self
    }

    #[must_use]
    pub fn files<I: IntoIterator<Item = P>, P: AsRef<Path>>(self, paths: I) -> Self {
        paths.into_iter().fold(self, Self::file)
    }

    #[must_use]
    pub fn theme(mut self, theme: &'static Theme) -> Self {
        self.theme = theme;
        self
    }

    /// The Wayland `app_id` the dock groups the window by; [`APP_ID`] by default.
    #[must_use]
    pub fn app_id(mut self, id: &str) -> Self {
        id.clone_into(&mut self.app_id);
        self
    }

    /// Where the lines tridi's binary prints go: each file's size once read,
    /// a file turned off or on. Nowhere by default.
    #[must_use]
    pub fn output(mut self, out: Box<dyn Write>) -> Self {
        self.out = out;
        self
    }

    /// Opens the window and blocks until it's closed. The files load in
    /// parallel and show up as they're read, a STEP file solid by solid.
    pub fn show(self) -> ViewResult<Shown> {
        if self.paths.is_empty() {
            return Err(ViewError::Empty);
        }
        app::run(self)
    }
}

/// [`Viewer`] with these files and the defaults.
pub fn show<I: IntoIterator<Item = P>, P: AsRef<Path>>(paths: I) -> ViewResult<Shown> {
    Viewer::new().files(paths).show()
}

// }}}

// =========================================== Tests =========================================== {{{

#[cfg(test)]
mod tests {
    use super::*;
    use std::{env, thread};

    #[test]
    fn empty_viewer_is_an_error() {
        assert!(matches!(Viewer::new().show(), Err(ViewError::Empty)));
    }

    // CI has no display; a developer's machine has one, and would open a window
    #[test]
    fn no_display_is_an_error() {
        if env::var_os("WAYLAND_DISPLAY").is_some() || env::var_os("DISPLAY").is_some() {
            return;
        }
        let r = Viewer::new().file("x.pcd").show();
        assert!(matches!(r, Err(ViewError::NoDisplay(_))), "{r:?}");
        // winit counted the failed try: a second one, on any thread, is still no display
        let r = Viewer::new().file("x.pcd").show();
        assert!(matches!(r, Err(ViewError::NoDisplay(_))), "{r:?}");
        let r = thread::spawn(|| Viewer::new().file("x.pcd").show()).join().unwrap();
        assert!(matches!(r, Err(ViewError::NoDisplay(_))), "{r:?}");
    }

    // winit allows one event loop per process: the thread that made it owns
    // it, another thread gets an error it can match. This claims the process's
    // one loop for the lib test binary: it must stay the only lib unit test
    // that builds the loop when a display is present.
    #[test]
    fn second_thread_is_an_error() {
        if env::var_os("WAYLAND_DISPLAY").is_none() && env::var_os("DISPLAY").is_none() {
            return;
        }
        // made here, on this test's thread, and kept for the process
        assert!(
            matches!(window::with_event_loop(|_| Ok(())), Ok(())),
            "the first thread gets the loop"
        );
        let r = thread::spawn(|| window::with_event_loop(|_| Ok(()))).join().unwrap();
        assert!(matches!(r, Err(ViewError::OtherThread)), "{r:?}");
    }

    #[test]
    fn keys_are_the_window_section() {
        assert!(
            KEYS.starts_with("Window:") && KEYS.ends_with("quit") && KEYS.contains("H or ?"),
            "{KEYS}"
        );
    }

    // a file name that isn't UTF-8 still gets a legend line
    #[test]
    fn any_file_name_has_a_label() {
        use std::{ffi::OsStr, os::unix::ffi::OsStrExt};
        let p = Path::new(OsStr::from_bytes(b"/tmp/\xff.pcd"));
        assert_eq!(load::label(p), "\u{fffd}.pcd");
        // no file name at all: the path itself
        assert_eq!(load::label(Path::new("/")), "/");
    }
}

// }}}
