//! STEP (.step/.stp) through monstertruck. Tessellation runs in a child
//! process, `tridi step-mesh IN`, that hands back each solid as it's done
//! and is killed once none has come for IDLE: on some real files the
//! library never finishes a solid and its memory grows ~20 MB/s while it
//! tries (a thread can't be stopped, a process can). A panic stays in the
//! child too.

// ======================================== Sub-modules ======================================== {{{

mod styles;
mod tessellate;

// }}}

// ========================================== Imports ========================================== {{{

use std::{
    env,
    io::{self, Read, Write},
    path::Path,
    process::{Command, Stdio},
    sync::mpsc::{self, RecvTimeoutError, Sender},
    thread,
    time::{Duration, Instant},
};
use three_d::{Srgba, Vec3};
use three_d_asset::{Indices, Positions, TriMesh};

use crate::step::tessellate::tessellate;

// }}}

// ========================================= Constants ========================================= {{{

// ponytail: a guess. Slow and stuck look the same from here: an 84 MB assembly parses in 7 s,
// then goes 34 s between two solids that do come, and 5 never do
const IDLE: Duration = Duration::from_secs(60);

/// A vertex on the wire: position, normal, color.
pub(crate) const FLOATS: usize = 9;

/// The color of a face the file gives none: negative, as no real color is.
pub(crate) const NONE: [f32; 3] = [-1.0; 3];

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

// ========================================== Parent =========================================== {{{

/// What comes out of the child: how many solids, then each one's triangles.
enum Frame {
    Total(usize),
    Solid(Vec<u8>),
}

/// Parent side: run the child, hand each solid's triangles to `part` as it
/// comes. `grey` stands in for the faces without a color, when others have
/// one. Gives up on the rest once no solid has come for IDLE, or `budget`
/// is spent; returns how many solids it gave up on.
#[instrument(skip(grey, part))]
pub fn load<F: FnMut(TriMesh)>(path: &Path, grey: [f32; 3], budget: Option<Duration>, mut part: F) -> StepResult<usize> {
    let start = Instant::now();
    let until = budget.and_then(|b| start.checked_add(b));
    let exe = env::current_exe().map_err(StepError::Spawn)?;
    let mut child = Command::new(exe)
        .arg("step-mesh")
        .arg(path)
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
    let (mut total, mut done, mut shown, mut timed_out) = (0, 0, 0, false);
    loop {
        let wait = until.map_or(IDLE, |u| IDLE.min(u.saturating_duration_since(Instant::now())));
        match rx.recv_timeout(wait) {
            Ok(Frame::Total(n)) => total = n,
            Ok(Frame::Solid(bytes)) => {
                done += 1;
                if !bytes.is_empty() {
                    shown += 1;
                    part(decode(&bytes, grey));
                }
            }
            // the child closed its stdout: done, or dead
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                timed_out = true;
                #[expect(clippy::let_underscore_must_use, reason = "it may be gone already; the timeout is the news")]
                let _ = child.kill();
                break;
            }
        }
    }
    let status = child.wait().map_err(StepError::Spawn)?;
    let msg = errs.join().unwrap_or_default();
    let skipped = total.saturating_sub(done);
    let elapsed_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
    debug!(elapsed_ms, total, done, skipped, "tessellated");
    // whatever came is shown; only a file that gave nothing is an error
    if shown > 0 {
        return Ok(skipped);
    }
    if timed_out {
        return Err(StepError::Timeout {
            secs: start.elapsed().as_secs(),
        });
    }
    let msg = msg.trim();
    let message = if msg.is_empty() {
        format!("tessellation failed ({status})")
    } else {
        msg.to_owned()
    };
    Err(StepError::Failed { message })
}

/// The child's stdout as frames, until it ends: a u32 count of solids, then
/// per solid a u32 byte length and the bytes.
fn read_frames(mut r: impl Read, tx: &Sender<Frame>) {
    fn word(r: &mut impl Read) -> Option<usize> {
        let mut w = [0; 4];
        r.read_exact(&mut w).ok()?;
        usize::try_from(u32::from_le_bytes(w)).ok()
    }
    let Some(n) = word(&mut r) else { return };
    if tx.send(Frame::Total(n)).is_err() {
        return;
    }
    while let Some(len) = word(&mut r) {
        let mut bytes = vec![0; len];
        if r.read_exact(&mut bytes).is_err() || tx.send(Frame::Solid(bytes)).is_err() {
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
        // NONE is negative, a real color 0..1
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

/// Child side: `tridi step-mesh IN`, frames as `read_frames` reads them.
pub fn mesh_to_stdout(path: &str) -> StepResult<()> {
    // die with the parent: a viewer killed while waiting must not leave us running
    // SAFETY: prctl with PR_SET_PDEATHSIG only sets a flag on this process
    unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) };
    tessellate(
        Path::new(path),
        |n| write_word(&mut io::stdout().lock(), n),
        |v| write_solid(&mut io::stdout().lock(), v),
    )
}

fn write_word(w: &mut impl Write, n: usize) -> io::Result<()> {
    let n = u32::try_from(n).map_err(io::Error::other)?;
    w.write_all(&n.to_le_bytes())?;
    // stdout is line-buffered: a frame waits for no newline
    w.flush()
}

/// One solid's frame, whole: the threads take turns on the lock.
fn write_solid(w: &mut impl Write, v: &[f32]) -> io::Result<()> {
    let bytes: Vec<u8> = v.iter().flat_map(|f| f.to_le_bytes()).collect();
    let n = u32::try_from(bytes.len()).map_err(io::Error::other)?;
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
    use std::sync::Mutex;

    // the child and the wire, not the process: `load` runs current_exe,
    // which under cargo test is the test binary; the process and IDLE were
    // checked by hand. The solids that come back, in one mesh
    fn mesh(name: &str) -> TriMesh {
        let wire = Mutex::new(Vec::new());
        tessellate(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(name),
            |n| write_word(&mut *wire.lock().unwrap(), n),
            |v| write_solid(&mut *wire.lock().unwrap(), v),
        )
        .unwrap();
        let (tx, rx) = mpsc::channel();
        read_frames(&wire.into_inner().unwrap()[..], &tx);
        drop(tx);
        let (mut total, mut bytes) = (0, Vec::new());
        for f in rx {
            match f {
                Frame::Total(n) => total = n,
                Frame::Solid(b) => {
                    total -= 1;
                    bytes.extend(b);
                }
            }
        }
        assert_eq!(total, 0, "every solid came back");
        decode(&bytes, [0.5; 3])
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

    #[test]
    fn not_step_is_error() {
        let f = tmp("bad.step", b"ISO-10303-21;\ngarbage");
        assert!(matches!(tessellate(&f, |_| Ok(()), |_| Ok(())), Err(StepError::NotStep)));
    }
}

// }}}
