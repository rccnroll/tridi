//! STEP (.step/.stp) through monstertruck. Tessellation runs in a child
//! process, `tridi step-mesh IN`, killed at DEADLINE: on some real files
//! the library never finishes and its memory grows ~20 MB/s while it tries
//! (a thread can't be stopped, a process can). A panic stays in the child too.

mod styles;
mod tessellate;

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;
use three_d::{Srgba, Vec3};
use three_d_asset::{Indices, Positions, TriMesh};

use tessellate::tessellate;

// ponytail: one deadline for viewer and thumbnails; the biggest real part (476 faces) takes 0.5 s
const DEADLINE: Duration = Duration::from_secs(15);

/// A vertex on the wire: position, normal, color (NONE when the file gives
/// the face none).
pub(crate) const FLOATS: usize = 9;
pub(crate) const NONE: [f32; 3] = [-1.0; 3];

/// Parent side: run the child, read its triangles. `grey` stands in for the
/// faces without a color, when others have one.
pub fn load(path: &Path, grey: [f32; 3]) -> Result<TriMesh, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut child = Command::new(exe)
        .arg("step-mesh")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let (mut out, mut err) = (child.stdout.take().unwrap(), child.stderr.take().unwrap());
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let (mut o, mut e) = (Vec::new(), String::new());
        let _ = out.read_to_end(&mut o);
        let _ = err.read_to_string(&mut e);
        let _ = tx.send((o, e));
    });
    let Ok((bytes, msg)) = rx.recv_timeout(DEADLINE) else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("gave up tessellating after {} s", DEADLINE.as_secs()));
    };
    let status = child.wait().map_err(|e| e.to_string())?;
    if !status.success() {
        let msg = msg.trim();
        return Err(if msg.is_empty() {
            format!("tessellation failed ({status})")
        } else {
            msg.to_string()
        });
    }
    Ok(decode(&bytes, grey))
}

/// Child side: `tridi step-mesh IN`.
pub fn mesh_to_stdout(path: &str) -> Result<(), String> {
    // die with the parent: a viewer killed while waiting must not leave us running
    unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) };
    let v = tessellate(Path::new(path))?;
    let bytes: Vec<u8> = v.iter().flat_map(|f| f.to_le_bytes()).collect();
    std::io::stdout().lock().write_all(&bytes).map_err(|e| e.to_string())
}

/// FLOATS f32 per vertex, three vertices per triangle.
fn decode(bytes: &[u8], grey: [f32; 3]) -> TriMesh {
    let f: Vec<f32> = bytes.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect();
    let (mut pos, mut nor, mut col) = (Vec::new(), Vec::new(), Vec::new());
    for v in f.as_chunks::<FLOATS>().0 {
        pos.push(Vec3::new(v[0], v[1], v[2]));
        nor.push(Vec3::new(v[3], v[4], v[5]));
        let c = if v[6..] == NONE { grey } else { [v[6], v[7], v[8]] };
        col.push(Srgba::from([c[0], c[1], c[2], 1.0]));
    }
    // no color anywhere: leave it to the theme's material
    let colored = f.as_chunks::<FLOATS>().0.iter().any(|v| v[6..] != NONE);
    TriMesh {
        positions: Positions::F32(pos),
        normals: Some(nor),
        colors: colored.then_some(col),
        indices: Indices::None,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // the child path only: `load` runs current_exe, which under cargo test is
    // the test binary; the process and the deadline were checked by hand
    fn mesh(name: &str) -> TriMesh {
        let v = tessellate(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(name)).unwrap();
        decode(&v.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>(), [0.5; 3])
    }

    fn bbox(m: &TriMesh) -> ([f32; 3], [f32; 3]) {
        let Positions::F32(p) = &m.positions else { unreachable!() };
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
        let f = std::env::temp_dir().join(format!("tridi-bad-{}.step", std::process::id()));
        std::fs::write(&f, b"ISO-10303-21;\ngarbage").unwrap();
        assert!(tessellate(&f).is_err());
    }
}
