//! STEP (.step/.stp) through monstertruck. Tessellation runs in a child
//! process, `pcdview step-mesh IN`, killed at DEADLINE: on some real files
//! the library never finishes and its memory grows ~20 MB/s while it tries
//! (a thread can't be stopped, a process can). A panic stays in the child too.

use monstertruck_io::step::load::*;
use monstertruck_meshing::prelude::*;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;
use three_d::Vec3;
use three_d_asset::{Indices, Positions, TriMesh};

// ponytail: one deadline for viewer and thumbnails; the biggest real part (476 faces) takes 0.5 s
const DEADLINE: Duration = Duration::from_secs(15);

/// Parent side: run the child, read its triangles.
pub fn load(path: &Path) -> Result<TriMesh, String> {
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
        return Err(if msg.is_empty() { format!("tessellation failed ({status})") } else { msg.to_string() });
    }
    Ok(decode(&bytes))
}

/// Child side: `pcdview step-mesh IN`.
pub fn mesh_to_stdout(path: &str) -> Result<(), String> {
    // die with the parent: a viewer killed while waiting must not leave us running
    unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) };
    let v = tessellate(Path::new(path))?;
    let bytes: Vec<u8> = v.iter().flat_map(|f| f.to_le_bytes()).collect();
    std::io::stdout().lock().write_all(&bytes).map_err(|e| e.to_string())
}

/// Six f32 per vertex (position, normal), three vertices per triangle.
fn decode(bytes: &[u8]) -> TriMesh {
    let f: Vec<f32> = bytes.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect();
    let (mut pos, mut nor) = (Vec::new(), Vec::new());
    for v in f.as_chunks::<6>().0 {
        pos.push(Vec3::new(v[0], v[1], v[2]));
        nor.push(Vec3::new(v[3], v[4], v[5]));
    }
    TriMesh { positions: Positions::F32(pos), normals: Some(nor), indices: Indices::None, ..Default::default() }
}

fn tessellate(path: &Path) -> Result<Vec<f32>, String> {
    let raw = std::fs::read(path).map_err(|e| e.to_string())?;
    // names and comments are often Latin-1: the geometry is ASCII either way
    let table = Table::from_step(&String::from_utf8_lossy(&raw)).map_err(|e| format!("not a STEP file: {e:?}"))?;
    // ponytail: every solid where it was modelled, assemblies not placed yet (ROADMAP)
    let mut shells: Vec<_> = table
        .manifold_solid_brep
        .values()
        .filter_map(|s| table.to_compressed_solid(s).ok())
        .flat_map(|s| s.boundaries)
        .collect();
    shells.extend(table.shell_based_surface_model.values().filter_map(|s| table.to_compressed_shells(s).ok()).flatten());

    let mut out = Vec::new();
    for shell in shells {
        let bbox: BoundingBox<Point3> = shell.vertices.iter().collect();
        let mut poly = shell.robust_triangulation(f64::max(bbox.diameter() * 0.001, TOLERANCE)).to_polygon();
        poly.put_together_same_attrs(TOLERANCE * 50.0).remove_degenerate_faces();
        let (p, n) = (poly.positions(), poly.normals());
        for t in poly.faces().triangle_iter() {
            let [a, b, c] = t.map(|v| p[v.pos]);
            let flat = (b - a).cross(c - a).normalize();
            for v in t {
                let q = p[v.pos];
                let m = v.nor.map_or(flat, |i| n[i]);
                out.extend([q.x, q.y, q.z, m.x, m.y, m.z].map(|x| x as f32));
            }
        }
    }
    if out.is_empty() {
        return Err("no solid in the STEP file could be read".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // the child path only: `load` runs current_exe, which under cargo test is
    // the test binary; the process and the deadline were checked by hand
    #[test]
    fn cube() {
        let f = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/cube.step");
        let v = tessellate(&f).unwrap();
        let bytes: Vec<u8> = v.iter().flat_map(|f| f.to_le_bytes()).collect();
        let m = decode(&bytes);
        assert_eq!(m.triangle_count(), 12);
        let Positions::F32(p) = &m.positions else { unreachable!() };
        let (lo, hi) = p.iter().fold((f32::MAX, f32::MIN), |(l, h), q| (l.min(q.x.min(q.y).min(q.z)), h.max(q.x.max(q.y).max(q.z))));
        assert_eq!((lo, hi), (0.0, 1.0));
        // flat faces: every normal is an axis
        assert!(m.normals.unwrap().iter().all(|n| (n.x.abs() + n.y.abs() + n.z.abs() - 1.0).abs() < 1e-4));
    }

    #[test]
    fn not_step_is_error() {
        let f = std::env::temp_dir().join(format!("pcdview-bad-{}.step", std::process::id()));
        std::fs::write(&f, b"ISO-10303-21;\ngarbage").unwrap();
        assert!(tessellate(&f).is_err());
    }
}
