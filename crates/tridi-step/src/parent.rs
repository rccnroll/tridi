//! The parent side of STEP: the helper (`helper.rs`) runs on the file and
//! hands back each solid as it's done, and is killed once none has come for
//! a while, or when `stop` says so: a part can take long to heal, a crash in
//! C++ or a runaway memory stays in the helper (a thread can't be stopped, a
//! process can). The solids the fine pass didn't hand back get a second,
//! coarse one.

// ========================================== Imports ========================================== {{{

use std::{
    fs,
    io::{self, Read},
    iter,
    path::Path,
    process::{Child, Stdio},
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
    thread,
    time::{Duration, Instant},
};

use crate::helper;

// }}}

// ========================================= Constants ========================================= {{{

/// The wait for the helper to read the file and list its solids: an 84 MB
/// assembly takes 6 s.
const PARSE: Duration = Duration::from_secs(60);

// ponytail: a guess, slow and stuck look the same from here. On an 84 MB assembly the slowest
// part takes 15 s, and all 98 come in 22 s
const IDLE: Duration = Duration::from_secs(60);

/// A thumbnail's wait for the solids: the file manager waits on us.
const THUMB_BUDGET: Duration = Duration::from_secs(15);

/// How often a wait looks at `stop`.
const POLL: Duration = Duration::from_millis(200);

/// A vertex on the wire: position, normal, color.
const FLOATS: usize = 9;

/// The helper's exit codes (`helper/main.cpp`).
const NOT_STEP: i32 = 1;
const NO_SOLID: i32 = 2;

// }}}

// ========================================== Errors =========================================== {{{

/// Why a STEP file gave no triangles.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StepError {
    /// The helper couldn't be put anywhere it can run.
    #[error("cannot set up the STEP helper")]
    Helper(#[source] io::Error),
    #[error("cannot run the STEP helper")]
    Spawn(#[source] io::Error),
    #[error("gave up tessellating after {secs} s")]
    Timeout { secs: u64 },
    /// The helper failed; `message` is what it printed, or its exit status.
    #[error("{message}")]
    Failed { message: String },
    #[error("cannot read the file")]
    Read(#[source] io::Error),
    /// The parser's own error is a multi-line dump of its tokenizer state,
    /// nothing a user can act on: not kept.
    #[error("not a STEP file")]
    NotStep,
    #[error("no solid in the STEP file could be read")]
    NoSolid,
    /// `stop` said so before the file was done.
    #[error("cancelled")]
    Cancelled,
}

pub type StepResult<T> = Result<T, StepError>;

// }}}

// ========================================== Quality ========================================== {{{

/// What a file is read for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// the window: a fine pass, then a coarse one for what it missed
    View,
    /// a thumbnail: coarse, and what came in 15 s
    Thumbnail,
}

/// How the helper tessellates: chords within 0.1% of a part's size, or
/// 0.5% for Coarse (`occt.cpp`), the fallback and the thumbnail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Quality {
    Fine,
    Coarse,
}

// }}}

// ========================================= StepMesh ========================================== {{{

/// One solid's triangles, placed where the assembly puts it: three vertices
/// per triangle, no index. `colors` are the file's, sRGB 0..1, one per
/// vertex (the `grey` given to `load` where a face has none); None if the
/// solid has no color at all.
#[derive(Debug, Clone, PartialEq)]
pub struct StepMesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub colors: Option<Vec<[f32; 3]>>,
}

// }}}

// ========================================== Parent =========================================== {{{

/// The helper's process, killed and reaped however `run` ends: an early
/// return, or a panic in the host's `part`. The normal path takes the
/// status first; then the kill is a no-op and the wait gives it again.
struct Reaped(Child);

impl Drop for Reaped {
    fn drop(&mut self) {
        #[expect(clippy::let_underscore_must_use, reason = "nothing to do about a helper that won't die")]
        let _ = (self.0.kill(), self.0.wait());
    }
}

/// What comes out of the helper: the ids of the solids it'll do, then each
/// one's id and triangles.
enum Frame {
    Ids(Vec<u64>),
    Solid(u64, Vec<u8>),
}

/// Why the frames stopped coming.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ended {
    /// the helper closed its stdout: done, or dead
    Closed,
    TimedOut,
    Cancelled,
}

/// Hands each solid's triangles to `part` as it comes, over the passes
/// `purpose` asks for. `grey` stands in for the faces without a color, when
/// others have one. `stop` is looked at every 200 ms; once it says so the
/// helper is killed and the result is `Cancelled`, whatever came before.
/// Returns how many solids it gave up on.
#[instrument(skip(grey, stop, part))]
pub fn load<F: FnMut(StepMesh)>(path: &Path, grey: [f32; 3], purpose: Purpose, stop: &dyn Fn() -> bool, mut part: F) -> StepResult<usize> {
    // OpenCASCADE tells a missing file from a bad one only on its console
    fs::File::open(path).map_err(StepError::Read)?;
    let start = Instant::now();
    let (passes, until): (&[Quality], _) = match purpose {
        Purpose::View => (&[Quality::Fine, Quality::Coarse], None),
        Purpose::Thumbnail => (&[Quality::Coarse], start.checked_add(THUMB_BUDGET)),
    };
    let mut shown = 0;
    let mut part = |m| {
        shown += 1;
        part(m);
    };
    // None until a helper has listed the solids
    let (mut missing, mut failure): (Option<Vec<u64>>, Option<StepError>) = (None, None);
    for &quality in passes {
        if missing.as_ref().is_some_and(Vec::is_empty) {
            break;
        }
        match run(path, quality, missing.as_deref().unwrap_or_default(), until, grey, stop, &mut part) {
            // stopped, or not STEP: the next pass would fail the same way
            Err(e @ (StepError::Cancelled | StepError::NotStep)) => return Err(e),
            // no list: the next pass would parse the same file the same way
            Err(e) => {
                failure = Some(e);
                break;
            }
            Ok((left, f)) => {
                debug!(?quality, ?left, "pass done");
                (missing, failure) = (Some(left), f);
            }
        }
    }
    let skipped = missing.map_or(0, |m| m.len());
    let elapsed_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
    debug!(elapsed_ms, shown, skipped, "tessellated");
    // whatever came is shown; only a file that gave nothing is an error
    if shown > 0 {
        return Ok(skipped);
    }
    Err(failure.unwrap_or(StepError::NoSolid))
}

/// One helper, on the solids in `only` (all if empty). Returns the solids
/// it didn't hand back, and why it stopped if it didn't finish; an error if
/// it never listed them or was cancelled.
fn run(
    path: &Path,
    quality: Quality,
    only: &[u64],
    until: Option<Instant>,
    grey: [f32; 3],
    stop: &dyn Fn() -> bool,
    part: &mut dyn FnMut(StepMesh),
) -> StepResult<(Vec<u64>, Option<StepError>)> {
    let start = Instant::now();
    let mut cmd = helper::command().map_err(StepError::Helper)?;
    cmd.arg(path);
    if quality == Quality::Coarse {
        cmd.arg("--coarse");
    }
    if !only.is_empty() {
        let ids: Vec<String> = only.iter().map(u64::to_string).collect();
        cmd.arg("--only").arg(ids.join(","));
    }
    let mut child = Reaped(
        helper::retry_busy(|| cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()).map_err(StepError::Spawn)?,
    );
    let (Some(out), Some(mut err)) = (child.0.stdout.take(), child.0.stderr.take()) else {
        return Err(StepError::Spawn(io::Error::other("the helper has no pipes")));
    };
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || read_frames(out, &tx));
    // on its own thread: a helper stuck on a full stderr would stop sending
    let errs = thread::spawn(move || {
        let mut e = String::new();
        #[expect(
            clippy::let_underscore_must_use,
            reason = "a pipe that breaks means a helper that died: its status says how"
        )]
        let _ = err.read_to_string(&mut e);
        e
    });
    let (missing, ended) = receive(&rx, until, grey, stop, part);
    if ended != Ended::Closed {
        debug!(?ended, "killing the helper");
        #[expect(clippy::let_underscore_must_use, reason = "it may be gone already; `ended` is the news")]
        let _ = child.0.kill();
    }
    let status = child.0.wait().map_err(StepError::Spawn)?;
    let msg = errs.join().unwrap_or_default();
    let failure = match ended {
        Ended::Cancelled => return Err(StepError::Cancelled),
        Ended::TimedOut => Some(StepError::Timeout {
            secs: start.elapsed().as_secs(),
        }),
        Ended::Closed => match status.code() {
            Some(0) => None,
            Some(NOT_STEP) => Some(StepError::NotStep),
            Some(NO_SOLID) => Some(StepError::NoSolid),
            // 3: we stopped reading; a signal: it crashed or was killed
            _ => Some(StepError::Failed {
                message: if msg.trim().is_empty() {
                    format!("tessellation failed ({status})")
                } else {
                    msg.trim().to_owned()
                },
            }),
        },
    };
    match (missing, failure) {
        (Some(m), f) => Ok((m, f)),
        (None, Some(e)) => Err(e),
        // the helper ended well without listing anything: can't be
        (None, None) => Err(StepError::NoSolid),
    }
}

/// The frames, until the helper is done, quiet too long (PARSE for the
/// list, IDLE between two solids, and never past `until`) or `stop` says
/// so. Returns the listed solids that didn't come (None if no list came),
/// and how it ended.
fn receive(
    rx: &Receiver<Frame>,
    until: Option<Instant>,
    grey: [f32; 3],
    stop: &dyn Fn() -> bool,
    part: &mut dyn FnMut(StepMesh),
) -> (Option<Vec<u64>>, Ended) {
    let mut missing: Option<Vec<u64>> = None;
    let mut last = Instant::now();
    loop {
        if stop() {
            return (missing, Ended::Cancelled);
        }
        let limit = if missing.is_some() { IDLE } else { PARSE };
        let deadline = until.map_or(last + limit, |u| u.min(last + limit));
        let Some(left) = deadline.checked_duration_since(Instant::now()) else {
            return (missing, Ended::TimedOut);
        };
        match rx.recv_timeout(left.min(POLL)) {
            Ok(Frame::Ids(ids)) => (missing, last) = (Some(ids), Instant::now()),
            Ok(Frame::Solid(id, bytes)) => {
                last = Instant::now();
                if let Some(m) = missing.as_mut() {
                    m.retain(|&x| x != id);
                }
                if !bytes.is_empty() {
                    part(decode(&bytes, grey));
                }
            }
            Err(RecvTimeoutError::Disconnected) => return (missing, Ended::Closed),
            // a POLL slice ended: look at stop and the deadline again
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

/// The helper's stdout as frames, until it ends: a u32 count and that many
/// u64 solid ids; then per solid its u64 id, a u32 byte length and the
/// bytes.
fn read_frames(mut r: impl Read, tx: &Sender<Frame>) {
    fn u32_at(r: &mut impl Read) -> Option<usize> {
        let mut w = [0; 4];
        r.read_exact(&mut w).ok()?;
        usize::try_from(u32::from_le_bytes(w)).ok()
    }
    fn u64_at(r: &mut impl Read) -> Option<u64> {
        let mut w = [0; 8];
        r.read_exact(&mut w).ok()?;
        Some(u64::from_le_bytes(w))
    }
    let Some(n) = u32_at(&mut r) else { return };
    let Some(ids) = iter::repeat_with(|| u64_at(&mut r)).take(n).collect::<Option<Vec<_>>>() else {
        return;
    };
    if tx.send(Frame::Ids(ids)).is_err() {
        return;
    }
    while let (Some(id), Some(len)) = (u64_at(&mut r), u32_at(&mut r)) {
        let mut bytes = vec![0; len];
        if r.read_exact(&mut bytes).is_err() || tx.send(Frame::Solid(id, bytes)).is_err() {
            return;
        }
    }
}

/// FLOATS f32 per vertex, three vertices per triangle.
fn decode(bytes: &[u8], grey: [f32; 3]) -> StepMesh {
    let f: Vec<f32> = bytes.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect();
    let (mut positions, mut normals, mut colors) = (Vec::new(), Vec::new(), Vec::new());
    let mut colored = false;
    for &[px, py, pz, nx, ny, nz, red, green, blue] in f.as_chunks::<FLOATS>().0 {
        positions.push([px, py, pz]);
        normals.push([nx, ny, nz]);
        // a face the file gives no color comes negative, a real color 0..1
        let own = red >= 0.0;
        colored |= own;
        colors.push(if own { [red, green, blue] } else { grey });
    }
    // no color anywhere: leave it to the caller's material
    StepMesh {
        positions,
        normals,
        colors: colored.then_some(colors),
    }
}

// }}}

// =========================================== Tests =========================================== {{{

#[cfg(test)]
mod tests {
    use super::*;
    use std::{env, fs, path::PathBuf, process};

    fn data(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/data").join(name)
    }

    /// Every solid of `name`, through the helper, for `purpose`.
    fn solids(name: &str, purpose: Purpose) -> Vec<StepMesh> {
        solids_at(&data(name), purpose)
    }

    fn solids_at(path: &Path, purpose: Purpose) -> Vec<StepMesh> {
        let mut got = vec![];
        let skipped = load(path, [0.2; 3], purpose, &|| false, |m| got.push(m)).unwrap();
        assert_eq!(skipped, 0, "every solid came back");
        got
    }

    fn triangles(ms: &[StepMesh]) -> usize {
        ms.iter().map(|m| m.positions.as_chunks::<3>().0.len()).sum()
    }

    fn bbox(ms: &[StepMesh]) -> ([f32; 3], [f32; 3]) {
        let ps = ms.iter().flat_map(|m| m.positions.iter());
        ps.fold(([f32::MAX; 3], [f32::MIN; 3]), |(l, h), q| {
            (
                [l[0].min(q[0]), l[1].min(q[1]), l[2].min(q[2])],
                [h[0].max(q[0]), h[1].max(q[1]), h[2].max(q[2])],
            )
        })
    }

    fn near(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4)
    }

    #[test]
    fn cube() {
        let ms = solids("cube.step", Purpose::View);
        assert_eq!(triangles(&ms), 12);
        assert_eq!(bbox(&ms), ([0.0; 3], [1.0; 3]));
        assert!(ms.iter().all(|m| m.normals.len() == m.positions.len()), "a normal per vertex");
        // flat faces: every normal is an axis
        let normals = ms.iter().flat_map(|m| m.normals.iter());
        assert!(normals.clone().all(|n| (n[0].abs() + n[1].abs() + n[2].abs() - 1.0).abs() < 1e-4));
        assert!(ms.iter().all(|m| m.colors.is_none()), "no colors in the file");
    }

    // the cube as a part used twice, laid out like exporters do: the part's
    // SHAPE_REPRESENTATION holds only a placement, the brep is behind a
    // relationship; the second copy moved to x=10 and turned 90° about z
    #[test]
    fn assembly_placed() {
        let ms = solids("assembly.step", Purpose::View);
        assert_eq!(triangles(&ms), 24);
        // without the turn the second cube would end at x=11
        let (lo, hi) = bbox(&ms);
        assert!(near(lo, [0.0; 3]) && near(hi, [10.0, 1.0, 1.0]), "{lo:?}..{hi:?}");
    }

    // the solid red, its first face green (a predefined colour)
    #[test]
    fn colors() {
        let ms = solids("colors.step", Purpose::View);
        let c: Vec<[f32; 3]> = ms.iter().flat_map(|m| m.colors.clone().unwrap_or_default()).collect();
        let count = |want: [f32; 3]| c.iter().filter(|&&x| near(x, want)).count();
        assert_eq!(count([0.0, 1.0, 0.0]), 6, "one face, two triangles");
        assert_eq!(count([1.0, 0.0, 0.0]), 30);
    }

    // the file's values, not OpenCASCADE's linear ones (0.5 would be 0.214)
    #[test]
    fn colors_as_in_file() {
        let src = fs::read_to_string(data("colors.step")).unwrap();
        let grey = src.replace("COLOUR_RGB('red, with a comma', 1., 0.E+00, 0.)", "COLOUR_RGB('', 0.5, 0.5, 0.5)");
        assert_ne!(grey, src, "the fixture's red is where the test expects it");
        let dir = env::temp_dir().join(format!("tridi-step-test-{}", process::id()));
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("grey.step");
        fs::write(&f, grey).unwrap();
        // `load`'s own grey is 0.2: a 0.5 is the file's
        let ms = solids_at(&f, Purpose::View);
        fs::remove_dir_all(dir).unwrap();
        let mut c = ms.iter().flat_map(|m| m.colors.iter().flatten());
        assert!(c.any(|&x| near(x, [0.5; 3])));
    }

    // thumbnails go coarse from the start: the cube is still a cube
    #[test]
    fn coarse() {
        let ms = solids("cube.step", Purpose::Thumbnail);
        assert_eq!(triangles(&ms), 12);
        assert_eq!(bbox(&ms), ([0.0; 3], [1.0; 3]));
    }

    #[test]
    fn not_step_is_error() {
        let r = load(&data("rgb_ascii.pcd"), [0.5; 3], Purpose::View, &|| false, |_| {});
        assert!(matches!(r, Err(StepError::NotStep)), "{r:?}");
    }

    #[test]
    fn missing_file_is_a_read_error() {
        let r = load(Path::new("/nonexistent.step"), [0.5; 3], Purpose::View, &|| false, |_| {});
        assert!(matches!(r, Err(StepError::Read(_))), "{r:?}");
    }

    // a closed window: the helper is killed, nothing comes
    #[test]
    fn cancel() {
        let r = load(&data("assembly.step"), [0.5; 3], Purpose::View, &|| true, |_| {});
        assert!(matches!(r, Err(StepError::Cancelled)), "{r:?}");
    }
}

// }}}
