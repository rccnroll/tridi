//! STEP (.step/.stp) through the system's OpenCASCADE. Tessellation runs in
//! a child process, `tridi step-mesh IN`, that hands back each part as it's
//! done and is killed once none has come for a while: a part can take long
//! to heal, a crash in C++ or a runaway memory stays in the child (a thread
//! can't be stopped, a process can). The parts the fine pass didn't hand
//! back get a second, coarse one.

// ======================================== Sub-modules ======================================== {{{

mod tessellate;

// }}}

// ========================================== Imports ========================================== {{{

use std::{
    env,
    io::{self, Read, Write},
    iter,
    path::Path,
    process::{Command, Stdio},
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
    thread,
    time::{Duration, Instant},
};
use three_d::{Srgba, Vec3};
use three_d_asset::{Indices, Positions, TriMesh};

use crate::step::tessellate::tessellate;

// }}}

// ========================================= Constants ========================================= {{{

/// The wait for the child to read the file and list its parts: an 84 MB
/// assembly takes 6 s.
const PARSE: Duration = Duration::from_secs(60);

// ponytail: a guess, slow and stuck look the same from here. On an 84 MB assembly the slowest
// part takes 15 s, and all 98 come in 22 s
const IDLE: Duration = Duration::from_secs(60);

/// A thumbnail's wait for the solids: the file manager waits on us.
const THUMB_BUDGET: Duration = Duration::from_secs(15);

/// A vertex on the wire: position, normal, color.
pub(crate) const FLOATS: usize = 9;

// }}}

// ========================================== Errors =========================================== {{{

/// Why a STEP file gave no triangles.
#[derive(Debug, thiserror::Error)]
pub enum StepError {
    #[error("cannot run the tessellator")]
    Spawn(#[source] io::Error),
    #[error("gave up tessellating after {secs} s")]
    Timeout { secs: u64 },
    /// The child failed; `message` is what it printed, or its exit status.
    #[error("{message}")]
    Failed { message: String },
    #[error("cannot read the file")]
    Read(#[source] io::Error),
    #[error("cannot hand the triangles back")]
    Write(#[source] io::Error),
    /// The parser's own error is a multi-line dump of its tokenizer state,
    /// nothing a user can act on: not kept.
    #[error("not a STEP file")]
    NotStep,
    #[error("no solid in the STEP file could be read")]
    NoSolid,
}

pub type StepResult<T> = Result<T, StepError>;

// }}}

// ========================================== Quality ========================================== {{{

/// What a file is read for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// the window: a fine pass, then a coarse one for what it missed
    View,
    /// a thumbnail: coarse, and what came in `THUMB_BUDGET`
    Thumbnail,
}

/// How the child tessellates: chords within 0.1% of a part's size, or
/// 0.5% for Coarse (`occt.cpp`), the fallback and the thumbnail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quality {
    Fine,
    Coarse,
}

// }}}

// ========================================== Parent =========================================== {{{

/// What comes out of the child: the ids of the solids it'll do, then each
/// one's id and triangles.
enum Frame {
    Ids(Vec<u64>),
    Solid(u64, Vec<u8>),
}

/// Parent side: hand each solid's triangles to `part` as it comes, over
/// the passes `purpose` asks for. `grey` stands in for the faces without a
/// color, when others have one. Returns how many solids it gave up on.
#[instrument(skip(grey, part))]
pub fn load<F: FnMut(TriMesh)>(path: &Path, grey: [f32; 3], purpose: Purpose, mut part: F) -> StepResult<usize> {
    let start = Instant::now();
    let (passes, until): (&[(Quality, Duration)], _) = match purpose {
        Purpose::View => (&[(Quality::Fine, IDLE), (Quality::Coarse, IDLE)], None),
        Purpose::Thumbnail => (&[(Quality::Coarse, IDLE)], start.checked_add(THUMB_BUDGET)),
    };
    let mut shown = 0;
    let mut part = |m| {
        shown += 1;
        part(m);
    };
    // None until a child has listed the solids
    let (mut missing, mut failure): (Option<Vec<u64>>, Option<StepError>) = (None, None);
    for &(quality, idle) in passes {
        if missing.as_ref().is_some_and(Vec::is_empty) {
            break;
        }
        match run(path, quality, missing.as_deref().unwrap_or_default(), idle, until, grey, &mut part) {
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

/// One child, on the solids in `only` (all if empty). Returns the solids it
/// didn't hand back, and why it stopped if it didn't finish; an error if it
/// never listed them.
fn run(
    path: &Path,
    quality: Quality,
    only: &[u64],
    idle: Duration,
    until: Option<Instant>,
    grey: [f32; 3],
    part: &mut dyn FnMut(TriMesh),
) -> StepResult<(Vec<u64>, Option<StepError>)> {
    let start = Instant::now();
    let exe = env::current_exe().map_err(StepError::Spawn)?;
    let mut cmd = Command::new(exe);
    cmd.arg("step-mesh").arg(path);
    if quality == Quality::Coarse {
        cmd.arg("--coarse");
    }
    if !only.is_empty() {
        let ids: Vec<String> = only.iter().map(u64::to_string).collect();
        cmd.arg("--only").arg(ids.join(","));
    }
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(StepError::Spawn)?;
    let (Some(out), Some(mut err)) = (child.stdout.take(), child.stderr.take()) else {
        return Err(StepError::Spawn(io::Error::other("the tessellator has no pipes")));
    };
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || read_frames(out, &tx));
    // on its own thread: a child stuck on a full stderr would stop sending
    let errs = thread::spawn(move || {
        let mut e = String::new();
        #[expect(
            clippy::let_underscore_must_use,
            reason = "a pipe that breaks means a child that died: its status says how"
        )]
        let _ = err.read_to_string(&mut e);
        e
    });
    let (missing, timed_out) = receive(&rx, idle, until, grey, part);
    if timed_out {
        #[expect(clippy::let_underscore_must_use, reason = "it may be gone already; the timeout is the news")]
        let _ = child.kill();
    }
    let status = child.wait().map_err(StepError::Spawn)?;
    let msg = errs.join().unwrap_or_default();
    let failure = if timed_out {
        Some(StepError::Timeout {
            secs: start.elapsed().as_secs(),
        })
    } else if status.success() {
        None
    } else {
        let msg = msg.trim();
        Some(StepError::Failed {
            message: if msg.is_empty() {
                format!("tessellation failed ({status})")
            } else {
                msg.to_owned()
            },
        })
    };
    match (missing, failure) {
        (Some(m), f) => Ok((m, f)),
        (None, Some(e)) => Err(e),
        // the child ended well without listing anything: can't be
        (None, None) => Err(StepError::NoSolid),
    }
}

/// The frames, until the child is done or quiet too long: PARSE for the
/// list, `idle` between two solids, and never past `until`. Returns the
/// listed solids that didn't come (None if no list came), and whether it
/// timed out.
fn receive(
    rx: &Receiver<Frame>,
    idle: Duration,
    until: Option<Instant>,
    grey: [f32; 3],
    part: &mut dyn FnMut(TriMesh),
) -> (Option<Vec<u64>>, bool) {
    let mut missing: Option<Vec<u64>> = None;
    loop {
        let limit = if missing.is_some() { idle } else { PARSE };
        let wait = until.map_or(limit, |u| limit.min(u.saturating_duration_since(Instant::now())));
        match rx.recv_timeout(wait) {
            Ok(Frame::Ids(ids)) => missing = Some(ids),
            Ok(Frame::Solid(id, bytes)) => {
                if let Some(m) = missing.as_mut() {
                    m.retain(|&x| x != id);
                }
                if !bytes.is_empty() {
                    part(decode(&bytes, grey));
                }
            }
            // the child closed its stdout: done, or dead
            Err(RecvTimeoutError::Disconnected) => return (missing, false),
            Err(RecvTimeoutError::Timeout) => return (missing, true),
        }
    }
}

/// The child's stdout as frames, until it ends: a u32 count and that many
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
fn decode(bytes: &[u8], grey: [f32; 3]) -> TriMesh {
    let f: Vec<f32> = bytes.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect();
    let (mut pos, mut nor, mut col) = (Vec::new(), Vec::new(), Vec::new());
    let mut colored = false;
    for &[px, py, pz, nx, ny, nz, red, green, blue] in f.as_chunks::<FLOATS>().0 {
        pos.push(Vec3::new(px, py, pz));
        nor.push(Vec3::new(nx, ny, nz));
        // a face the file gives no color comes negative, a real color 0..1
        let own = red >= 0.0;
        colored |= own;
        let [r, g, b] = if own { [red, green, blue] } else { grey };
        col.push(Srgba::from([r, g, b, 1.0]));
    }
    // no color anywhere: leave it to the theme's material
    TriMesh {
        positions: Positions::F32(pos),
        normals: Some(nor),
        colors: colored.then_some(col),
        indices: Indices::None,
        ..Default::default()
    }
}

// }}}

// =========================================== Child =========================================== {{{

/// Child side: `tridi step-mesh IN [--coarse] [--only ID,...]`, frames as
/// `read_frames` reads them.
pub fn mesh_to_stdout(path: &str, quality: Quality, only: &[u64]) -> StepResult<()> {
    // die with the parent: a viewer killed while waiting must not leave us running
    // SAFETY: prctl with PR_SET_PDEATHSIG only sets a flag on this process
    unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) };
    tessellate(
        Path::new(path),
        quality,
        only,
        |ids| write_ids(&mut io::stdout().lock(), ids),
        |id, v| write_solid(&mut io::stdout().lock(), id, v),
    )
}

fn write_ids(w: &mut impl Write, ids: &[u64]) -> io::Result<()> {
    let n = u32::try_from(ids.len()).map_err(io::Error::other)?;
    w.write_all(&n.to_le_bytes())?;
    for id in ids {
        w.write_all(&id.to_le_bytes())?;
    }
    // stdout is line-buffered: a frame waits for no newline
    w.flush()
}

/// One solid's frame, whole: the threads take turns on the lock.
fn write_solid(w: &mut impl Write, id: u64, v: &[f32]) -> io::Result<()> {
    let bytes: Vec<u8> = v.iter().flat_map(|f| f.to_le_bytes()).collect();
    let n = u32::try_from(bytes.len()).map_err(io::Error::other)?;
    w.write_all(&id.to_le_bytes())?;
    w.write_all(&n.to_le_bytes())?;
    w.write_all(&bytes)?;
    w.flush()
}

// }}}

// =========================================== Tests =========================================== {{{

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::tmp;
    use std::{fs, sync::Mutex};

    // the child and the wire, not the process: `load` runs current_exe,
    // which under cargo test is the test binary; the process and the
    // timeouts were checked by hand. The solids that come back, in one mesh
    fn mesh_with(name: &str, quality: Quality) -> TriMesh {
        let wire = Mutex::new(Vec::new());
        tessellate(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(name),
            quality,
            &[],
            |ids| write_ids(&mut *wire.lock().unwrap(), ids),
            |id, v| write_solid(&mut *wire.lock().unwrap(), id, v),
        )
        .unwrap();
        let (tx, rx) = mpsc::channel();
        read_frames(&wire.into_inner().unwrap()[..], &tx);
        drop(tx);
        let mut got = Vec::new();
        let (missing, timed_out) = receive(&rx, Duration::from_secs(1), None, [0.5; 3], &mut |m| got.push(m));
        assert!(!timed_out && missing == Some(vec![]), "every solid came back");
        // one solid in each fixture (the assembly's is placed twice)
        assert_eq!(got.len(), 1);
        got.pop().unwrap()
    }

    fn mesh(name: &str) -> TriMesh {
        mesh_with(name, Quality::Fine)
    }

    fn bbox(m: &TriMesh) -> ([f32; 3], [f32; 3]) {
        let p = m.positions.to_f32();
        p.iter().fold(([f32::MAX; 3], [f32::MIN; 3]), |(l, h), q| {
            (
                [l[0].min(q.x), l[1].min(q.y), l[2].min(q.z)],
                [h[0].max(q.x), h[1].max(q.y), h[2].max(q.z)],
            )
        })
    }

    #[test]
    fn cube() {
        let m = mesh("cube.step");
        assert_eq!(m.triangle_count(), 12);
        assert_eq!(bbox(&m), ([0.0; 3], [1.0; 3]));
        // flat faces: every normal is an axis
        assert!(
            m.normals
                .unwrap()
                .iter()
                .all(|n| (n.x.abs() + n.y.abs() + n.z.abs() - 1.0).abs() < 1e-4)
        );
    }

    // the cube as a part used twice, laid out like exporters do: the part's
    // SHAPE_REPRESENTATION holds only a placement, the brep is behind a
    // relationship; the second copy moved to x=10 and turned 90° about z
    #[test]
    fn assembly_placed() {
        let m = mesh("assembly.step");
        assert_eq!(m.triangle_count(), 24);
        // without the turn the second cube would end at x=11
        let (lo, hi) = bbox(&m);
        let near = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4);
        assert!(near(lo, [0.0; 3]) && near(hi, [10.0, 1.0, 1.0]), "{lo:?}..{hi:?}");
    }

    // the solid red, its first face green (a predefined colour); a cube with
    // no style keeps no colors, for the theme's material
    #[test]
    fn colors() {
        let m = mesh("colors.step");
        let c = m.colors.unwrap();
        let (red, green) = (Srgba::from([1.0, 0.0, 0.0, 1.0]), Srgba::from([0.0, 1.0, 0.0, 1.0]));
        assert_eq!(c.iter().filter(|&&x| x == green).count(), 6, "one face, two triangles");
        assert_eq!(c.iter().filter(|&&x| x == red).count(), 30);
        assert!(mesh("cube.step").colors.is_none());
    }

    // the file's values, not OpenCASCADE's linear ones (0.5 would be 0.214)
    #[test]
    fn colors_as_in_file() {
        let src = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/colors.step")).unwrap();
        let grey = src.replace("COLOUR_RGB('red, with a comma', 1., 0.E+00, 0.)", "COLOUR_RGB('', 0.5, 0.5, 0.5)");
        assert_ne!(grey, src, "the fixture's red is where the test expects it");
        let f = tmp("grey.step", grey.as_bytes());
        let got = Mutex::new(Vec::new());
        tessellate(
            &f,
            Quality::Fine,
            &[],
            |_| Ok(()),
            |_, v| {
                got.lock().unwrap().extend_from_slice(v);
                Ok(())
            },
        )
        .unwrap();
        let got = got.into_inner().unwrap();
        assert!(got.chunks(FLOATS).any(|v| v[6..9].iter().all(|c| (c - 0.5).abs() < 1e-3)));
    }

    // the fallback for the solids the fine pass misses: the same cube
    #[test]
    fn coarse() {
        let m = mesh_with("cube.step", Quality::Coarse);
        assert_eq!(m.triangle_count(), 12);
        assert_eq!(bbox(&m), ([0.0; 3], [1.0; 3]));
    }

    #[test]
    fn not_step_is_error() {
        let f = tmp("bad.step", b"ISO-10303-21;\ngarbage");
        assert!(matches!(
            tessellate(&f, Quality::Fine, &[], |_| Ok(()), |_, _| Ok(())),
            Err(StepError::NotStep)
        ));
    }
}

// }}}
