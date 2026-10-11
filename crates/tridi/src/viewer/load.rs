//! The files read on loader threads and sent to the event loop as they come,
//! made ready for the scene.

// ========================================== Imports ========================================== {{{

use std::{
    num::NonZeroUsize,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
};
use three_d::{AxisAlignedBoundingBox, Vec3};
use three_d_asset::Geometry;
use winit::event_loop::EventLoopProxy;

use crate::{Input, Item, LoadResult, Purpose, Theme, colorize, load, own_colors, triangles};

// }}}

// ========================================== Loading ========================================== {{{

/// A file to show, as the legend shows it.
pub(crate) struct File {
    pub(crate) name: String,
    pub(crate) state: State,
}

pub(crate) enum State {
    Loading,
    Failed,
    /// its layer in the scene, its tint (None when it has its own colors, or
    /// is a mesh), how many points or triangles so far and their bbox;
    /// `loading` until the last of a STEP file's solids is in, then how many
    /// were given up on
    Shown {
        layer: usize,
        tint: Option<[f32; 3]>,
        count: usize,
        unit: &'static str,
        bb: AxisAlignedBoundingBox,
        loading: bool,
        skipped: usize,
    },
}

/// A file read on a loader thread, or one of a STEP file's solids, made
/// ready for the scene.
pub(crate) struct Ready {
    pub(crate) input: Input,
    pub(crate) tint: Option<[f32; 3]>,
    pub(crate) count: usize,
    pub(crate) unit: &'static str,
    pub(crate) bb: AxisAlignedBoundingBox,
}

/// What a loader thread sends the event loop.
pub(crate) enum Loaded {
    /// the i-th file, or one of its solids if it's STEP
    Part { i: usize, ready: Ready },
    /// the i-th file is all in: how many solids were given up on, or why
    /// nothing came
    Done { i: usize, result: LoadResult<usize> },
}

/// Reads the files on as many threads as there are cores, each sent to the
/// event loop as it's ready. A file's tint is its place on the command line,
/// so it doesn't change with the order they finish in.
pub(crate) fn load_all(paths: &[PathBuf], theme: &'static Theme, proxy: &EventLoopProxy<Loaded>) {
    let paths: Arc<[PathBuf]> = paths.into();
    let next = Arc::new(AtomicUsize::new(0));
    let workers = thread::available_parallelism().map_or(1, NonZeroUsize::get).min(paths.len());
    for _ in 0..workers {
        let (paths, next, proxy) = (Arc::clone(&paths), Arc::clone(&next), proxy.clone());
        thread::spawn(move || {
            loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(p) = paths.get(i) else { break };
                let tint = theme.palette.iter().cycle().nth(i).copied().filter(|_| paths.len() > 1);
                let result = load(p, theme.mesh, Purpose::View, &|| false, &mut |item| {
                    #[expect(clippy::let_underscore_must_use, reason = "a closed window is found out below")]
                    let _ = proxy.send_event(Loaded::Part {
                        i,
                        ready: ready(item, tint, theme),
                    });
                });
                // the window was closed: nobody waits for the rest
                if proxy.send_event(Loaded::Done { i, result }).is_err() {
                    break;
                }
            }
        });
    }
}

/// Colors what was read; `tint` is its file's color among several files.
fn ready(item: Item, tint: Option<[f32; 3]>, theme: &Theme) -> Ready {
    match item {
        Item::Cloud(c) => Ready {
            bb: bbox(c.points.iter().copied().map(Vec3::from)),
            // the legend shows the tint only if it's what the points get
            tint: tint.filter(|_| own_colors(&c, tint).is_none()),
            count: c.points.len(),
            unit: "points",
            input: {
                let colors = colorize(&c, tint, theme);
                Input::Points(c.points, colors)
            },
        },
        Item::Mesh(m) => Ready {
            // ponytail: no per-file tint on meshes (1.0 didn't have one either)
            bb: bbox(m.geometries.iter().flat_map(|g| {
                match &g.geometry {
                    Geometry::Triangles(t) => t
                        .positions
                        .to_f32()
                        .into_iter()
                        .map(|v| (g.transformation * v.extend(1.0)).truncate())
                        .collect(),
                    Geometry::Points(_) => vec![],
                }
            })),
            tint: None,
            count: triangles(&m),
            unit: "triangles",
            input: Input::Mesh(m),
        },
    }
}

fn bbox<I: Iterator<Item = Vec3>>(pts: I) -> AxisAlignedBoundingBox {
    let mut bb = AxisAlignedBoundingBox::EMPTY;
    bb.expand(&pts.collect::<Vec<_>>());
    bb
}

// }}}

// =========================================== Label =========================================== {{{

/// A file's name in the legend, the title and the lines on stdout: the last
/// component, or the whole path if it has none; lossy if not UTF-8.
pub(crate) fn label(p: &Path) -> String {
    p.file_name()
        .map_or_else(|| p.to_string_lossy(), |n| n.to_string_lossy())
        .into_owned()
}

// }}}
