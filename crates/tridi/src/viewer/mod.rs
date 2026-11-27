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
    path::Path,
};
use winit::error::EventLoopError;

use load::Source;

use crate::{Cloud, LIGHT, LoadError, RenderError, Theme};

// }}}

// ========================================= Constants ========================================= {{{

/// The Wayland `app_id` the dock groups the window by, matching tridi's .desktop.
pub const APP_ID: &str = "io.github.rccnroll.tridi";

/// The keys, as `tridi --help` lists them and H shows them in the window.
pub const KEYS: &str = "\
Window:
  View
    left drag             orbit
    right or middle drag  pan (or shift + left drag)
    wheel                 zoom
    R                     reset the view
    + and -               point size
  Files
    1-9 or legend row     turn the N-th file off and on (any file by click)
    A                     turn every file off, or all back on
    wheel on legend row   that file's point size
    right click legend    that file's point size back to the others'
    I                     show or hide the legend (on with several files)
  Picks
    shift + click         pick a point (printed on stdout)
    shift + right click   take the last pick off
    P                     show or hide the picks
    click on pick row     copy that pick (Ctrl+C: all of them)
    right click pick row  take that pick off
    Delete                take every pick off
  Other
    H or ?                show or hide these keys
    Q or Esc              quit";

// }}}

// ========================================== Errors =========================================== {{{

/// Why the window couldn't show.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
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
    /// a cloud given with no points
    #[error("{name}: no points")]
    EmptyCloud { name: String },
    /// a cloud given with more or fewer colors than points
    #[error("{name}: {colors} colors for {points} points")]
    ColorCount { name: String, points: usize, colors: usize },
    /// Every file failed: each one's name, and why.
    #[error("no file could be opened")]
    NothingOpened { failed: Vec<(String, LoadError)> },
}

pub type ViewResult<T> = Result<T, ViewError>;

// }}}

// =========================================== Shown =========================================== {{{

/// What the window showed: the files that failed, and why.
#[derive(Debug)]
#[non_exhaustive]
pub struct Shown {
    pub failed: Vec<(String, LoadError)>,
}

// }}}

// ========================================== Viewer =========================================== {{{

/// Sets up the window: files, theme, app id, where the per-file lines go.
pub struct Viewer {
    sources: Vec<Source>,
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
            sources: vec![],
            theme: &LIGHT,
            app_id: APP_ID.to_owned(),
            out: Box::new(io::sink()),
        }
    }

    #[must_use]
    pub fn file<P: AsRef<Path>>(mut self, path: P) -> Self {
        self.sources.push(Source::File(path.as_ref().to_owned()));
        self
    }

    /// A cloud from memory, shown as a file would be: `name` in the legend.
    /// `colors`, if given, must have one per point: `show()` says if not.
    /// Points that aren't finite (NaN, as PCL marks missing ones) are
    /// dropped with their colors, as from a file.
    #[must_use]
    pub fn cloud(mut self, name: &str, points: &[[f32; 3]], colors: Option<&[[f32; 3]]>) -> Self {
        let finite = |p: &&[f32; 3]| p.iter().all(|v| v.is_finite());
        // colors that don't match are kept as given, unfiltered, so that
        // `show()` reports the counts the caller passed
        let kept: Vec<usize> = (points.iter().enumerate()).filter(|(_, p)| finite(p)).map(|(i, _)| i).collect();
        let keep = |v: &[[f32; 3]]| kept.iter().filter_map(|&i| v.get(i).copied()).collect::<Vec<_>>();
        let (points, colors, index) = match colors {
            Some(c) if c.len() != points.len() => (points.to_vec(), Some(c.to_vec()), None),
            // a pick names the point's place in what the caller passed
            c => (keep(points), c.map(keep), (kept.len() != points.len()).then_some(kept)),
        };
        self.sources.push(Source::Cloud {
            name: name.to_owned(),
            cloud: Cloud {
                points,
                colors,
                faces: None,
                index,
            },
        });
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
    /// a file turned off or on, a point picked or removed (`picked #INDEX X Y
    /// Z NAME`, INDEX the point's place in the file or in the cloud as
    /// given). Nowhere by default.
    #[must_use]
    pub fn output(mut self, out: Box<dyn Write>) -> Self {
        self.out = out;
        self
    }

    /// Opens the window and blocks until it's closed. The files load in
    /// parallel and show up as they're read, a STEP file solid by solid;
    /// the clouds from memory show at once. A cloud with no points, or with
    /// colors that don't match its points, is an error before any window opens.
    ///
    /// While the window opens, the process's stderr (fd 2) is pointed at
    /// /dev/null for a moment, to silence what Mesa prints about each GPU it
    /// has no driver for: whatever another thread writes to stderr then is
    /// lost. `TRIDI_DEBUG` set in the environment, to any value, leaves
    /// stderr alone.
    pub fn show(self) -> ViewResult<Shown> {
        for s in &self.sources {
            if let Source::Cloud { name, cloud } = s
                && cloud.points.is_empty()
            {
                return Err(ViewError::EmptyCloud { name: name.clone() });
            }
            if let Source::Cloud {
                name,
                cloud:
                    Cloud {
                        points,
                        colors: Some(colors),
                        ..
                    },
            } = s
                && colors.len() != points.len()
            {
                return Err(ViewError::ColorCount {
                    name: name.clone(),
                    points: points.len(),
                    colors: colors.len(),
                });
            }
        }
        if self.sources.is_empty() {
            return Err(ViewError::Empty);
        }
        app::run(self)
    }
}

/// [`Viewer`] with these files and the defaults. stderr is silenced for a
/// moment while the window opens, as [`Viewer::show`] says.
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
    fn color_count_must_match() {
        let r = Viewer::new().cloud("c", &[[0.0; 3]; 3], Some(&[[1.0; 3]; 2])).show();
        assert!(
            matches!(&r, Err(ViewError::ColorCount { name, points: 3, colors: 2 }) if name == "c"),
            "{r:?}"
        );
    }

    #[test]
    fn non_finite_points_are_dropped() {
        let nan = [f32::NAN, 0.0, 0.0];
        let points = [nan, [1.0; 3], [0.0, f32::INFINITY, 0.0], [2.0; 3]];
        let v = Viewer::new().cloud("c", &points, Some(&[[0.1; 3], [0.2; 3], [0.3; 3], [0.4; 3]]));
        assert!(
            matches!(&v.sources[..], [Source::Cloud { cloud, .. }]
                if cloud.points == [[1.0; 3], [2.0; 3]] && cloud.colors == Some(vec![[0.2; 3], [0.4; 3]])),
            "the finite points and their colors"
        );
        // a pick names the point's place in what the caller passed
        assert!(
            matches!(&v.sources[..], [Source::Cloud { cloud, .. }] if cloud.file_index(1) == 3),
            "the caller's index"
        );
        let v = Viewer::new().cloud("all", &[[1.0; 3]], None);
        assert!(
            matches!(&v.sources[..], [Source::Cloud { cloud, .. }] if cloud.index.is_none()),
            "none dropped: no index"
        );
        // nothing left: no points
        let r = Viewer::new().cloud("n", &[nan], None).show();
        assert!(matches!(&r, Err(ViewError::EmptyCloud { name }) if name == "n"), "{r:?}");
        // a wrong color count is told as given, NaNs and all
        let r = Viewer::new().cloud("w", &[nan, [1.0; 3]], Some(&[[0.1; 3]])).show();
        assert!(matches!(&r, Err(ViewError::ColorCount { points: 2, colors: 1, .. })), "{r:?}");
    }

    #[test]
    fn empty_cloud_is_an_error() {
        let r = Viewer::new().cloud("x", &[], None).show();
        assert!(matches!(&r, Err(ViewError::EmptyCloud { name }) if name == "x"), "{r:?}");
    }

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
        // in groups, a key under its own: picking keys under Picks
        let picks = KEYS.find("\n  Picks").unwrap();
        let files = KEYS.find("\n  Files").unwrap();
        assert!(KEYS.find("\n  View").unwrap() < files && files < picks, "{KEYS}");
        assert!(
            KEYS.find("shift + click").unwrap() > picks && KEYS.find("Delete").unwrap() > picks,
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
