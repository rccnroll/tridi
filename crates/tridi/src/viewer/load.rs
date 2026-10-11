//! The files read on loader threads and sent to the event loop as they come,
//! made ready for the scene.

// ========================================== Imports ========================================== {{{

use std::{
    num::NonZeroUsize,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    thread,
};
use three_d::{AxisAlignedBoundingBox, Vec3};
use three_d_asset::Geometry;
use winit::event_loop::EventLoopProxy;

use crate::{Cloud, Input, Item, LoadResult, Purpose, Theme, colorize, load, own_colors, triangles};

// }}}

// ========================================== Loading ========================================== {{{

/// What a window shows, in legend order.
pub(crate) enum Source {
    File(PathBuf),
    Cloud { name: String, cloud: Cloud },
}

impl Source {
    /// Its name in the legend, the title and the lines on stdout.
    pub(crate) fn name(&self) -> String {
        match self {
            Self::File(p) => label(p),
            Self::Cloud { name, .. } => name.clone(),
        }
    }
}

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

/// Which `show()` a loader thread works for: the event loop outlives a
/// window, and a slow STEP file may still send pieces after it closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Generation(u64);

impl Generation {
    /// A new one for each window, never seen before in this process.
    pub(crate) fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

/// What a loader thread sends the event loop.
pub(crate) enum Loaded {
    /// the i-th file, or one of its solids if it's STEP
    Part { generation: Generation, i: usize, ready: Ready },
    /// the i-th file is all in: how many solids were given up on, or why
    /// nothing came
    Done {
        generation: Generation,
        i: usize,
        result: LoadResult<usize>,
    },
}

impl Loaded {
    /// Whether it was sent for the window of generation `g`.
    pub(crate) fn is_for(&self, g: Generation) -> bool {
        match self {
            Self::Part { generation, .. } | Self::Done { generation, .. } => *generation == g,
        }
    }
}

/// Hands the clouds to the event loop at once and reads the files on as many threads as there are cores, each sent to the
/// event loop as it's ready. A file's tint is its place on the command line,
/// so it doesn't change with the order they finish in. Once `stop` is set
/// (the window closed) each thread ends: before its next file, or within a
/// STEP file, whose helper is killed.
pub(crate) fn load_all(
    sources: Vec<Source>,
    theme: &'static Theme,
    generation: Generation,
    stop: &Arc<AtomicBool>,
    proxy: &EventLoopProxy<Loaded>,
) {
    let total = sources.len();
    let tint_of = move |i: usize| theme.palette.iter().cycle().nth(i).copied().filter(|_| total > 1);
    let mut files = vec![];
    for (i, source) in sources.into_iter().enumerate() {
        match source {
            Source::File(p) => files.push((i, p)),
            Source::Cloud { cloud, .. } => {
                let ready = ready(Item::Cloud(cloud), tint_of(i), theme);
                // the event loop is gone: nobody to show it to
                if proxy.send_event(Loaded::Part { generation, i, ready }).is_err()
                    || proxy
                        .send_event(Loaded::Done {
                            generation,
                            i,
                            result: Ok(0),
                        })
                        .is_err()
                {
                    return;
                }
            }
        }
    }
    let files: Arc<[(usize, PathBuf)]> = files.into();
    let next = Arc::new(AtomicUsize::new(0));
    let workers = thread::available_parallelism().map_or(1, NonZeroUsize::get).min(files.len());
    for _ in 0..workers {
        let (files, next, stop, proxy) = (Arc::clone(&files), Arc::clone(&next), Arc::clone(stop), proxy.clone());
        thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let Some((i, p)) = files.get(next.fetch_add(1, Ordering::Relaxed)) else {
                    break;
                };
                let (i, tint) = (*i, tint_of(*i));
                let result = load(p, theme.mesh, Purpose::View, &|| stop.load(Ordering::Relaxed), &mut |item| {
                    // the window closed, the helper not killed yet: nobody to show it to
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    #[expect(clippy::let_underscore_must_use, reason = "a closed window is found out below")]
                    let _ = proxy.send_event(Loaded::Part {
                        generation,
                        i,
                        ready: ready(item, tint, theme),
                    });
                });
                // the window is gone (a STEP load says Cancelled): nobody to tell
                if stop.load(Ordering::Relaxed) {
                    debug!("{}: window closed, not shown", label(p));
                    break;
                }
                // the event loop is gone (the thread that ran it ended)
                if proxy.send_event(Loaded::Done { generation, i, result }).is_err() {
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

// =========================================== Tests =========================================== {{{

#[cfg(test)]
mod tests {
    use super::*;

    // a window's late pieces never reach the next one
    #[test]
    fn stale_events_are_dropped() {
        let ours = Generation::next();
        let old = Generation::next();
        assert_ne!(ours, old);
        assert!(
            Loaded::Done {
                generation: ours,
                i: 0,
                result: Ok(0)
            }
            .is_for(ours)
        );
        assert!(
            !Loaded::Done {
                generation: old,
                i: 0,
                result: Ok(0)
            }
            .is_for(ours)
        );
    }
}

// }}}
