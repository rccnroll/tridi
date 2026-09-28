//! STEP (.step/.stp) through monstertruck. Tessellation runs in a child
//! process, `pcdview step-mesh IN`, killed at DEADLINE: on some real files
//! the library never finishes and its memory grows ~20 MB/s while it tries
//! (a thread can't be stopped, a process can). A panic stays in the child too.

use monstertruck_assembly::assy::{EdgeEntity, NodeEntity};
use monstertruck_io::step::load::step_p21::{ast::Name, tables::PlaceHolder};
use monstertruck_io::step::load::*;
use monstertruck_meshing::prelude::*;
use std::collections::HashMap;
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
    let mut out = Vec::new();
    // each solid tessellated once, however many times the assembly uses it
    let mut cache = HashMap::<u64, Vec<f32>>::new();
    if let Ok(assy) = table.step_assy() {
        let assy = assy.map(
            |n| NodeEntity { shape: n.attrs.shape_representation, attrs: () },
            |e| EdgeEntity { matrix: Matrix4::try_from(&e.matrix).unwrap_or(Matrix4::identity()), attrs: () },
        );
        for top in assy.top_nodes() {
            for p in assy.paths_iter(top.index()) {
                let Some(rep) = *p.terminal_node().shape() else { continue };
                let m = p.matrix();
                for item in solids_of(&table, rep) {
                    let local = cache.entry(item).or_insert_with(|| mesh_item(&table, item));
                    for v in local.as_chunks::<6>().0 {
                        let q = m.transform_point(Point3::new(v[0].into(), v[1].into(), v[2].into()));
                        let n = m.transform_vector(Vector3::new(v[3].into(), v[4].into(), v[5].into())).normalize();
                        out.extend([q.x, q.y, q.z, n.x, n.y, n.z].map(|x| x as f32));
                    }
                }
            }
        }
    }
    // no assembly the library understands: every solid where it was modelled
    if out.is_empty() {
        for &id in table.manifold_solid_brep.keys().chain(table.shell_based_surface_model.keys()) {
            out.extend(mesh_item(&table, id));
        }
    }
    if out.is_empty() {
        return Err("no solid in the STEP file could be read".into());
    }
    Ok(out)
}

/// The solids of a representation: its own items, and those of the
/// representations tied to it without a transform (a product's
/// SHAPE_REPRESENTATION often holds only a placement, and its brep sits in
/// an ADVANCED_BREP_SHAPE_REPRESENTATION linked by a relationship).
fn solids_of(table: &Table, rep: u64) -> Vec<u64> {
    let (mut todo, mut seen, mut found) = (vec![rep], vec![rep], Vec::new());
    while let Some(r) = todo.pop() {
        if let Some(sr) = table.shape_representation.get(&r) {
            for item in &sr.items {
                if let PlaceHolder::Ref(Name::Entity(id)) = item
                    && (table.manifold_solid_brep.contains_key(id) || table.shell_based_surface_model.contains_key(id))
                {
                    found.push(*id);
                }
            }
        }
        for srr in table.shape_representation_relationship.values() {
            let (PlaceHolder::Ref(Name::Entity(a)), PlaceHolder::Ref(Name::Entity(b))) = (&srr.rep_1, &srr.rep_2) else { continue };
            let other = if *a == r { *b } else if *b == r { *a } else { continue };
            if !seen.contains(&other) {
                seen.push(other);
                todo.push(other);
            }
        }
    }
    found
}

/// Triangles of one MANIFOLD_SOLID_BREP or SHELL_BASED_SURFACE_MODEL, in its
/// own coordinates; empty if the library can't convert it.
fn mesh_item(table: &Table, id: u64) -> Vec<f32> {
    let shells = if let Some(s) = table.manifold_solid_brep.get(&id) {
        table.to_compressed_solid(s).map(|s| s.boundaries).unwrap_or_default()
    } else if let Some(s) = table.shell_based_surface_model.get(&id) {
        table.to_compressed_shells(s).unwrap_or_default()
    } else {
        Vec::new()
    };
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
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // the child path only: `load` runs current_exe, which under cargo test is
    // the test binary; the process and the deadline were checked by hand
    fn mesh(name: &str) -> TriMesh {
        let v = tessellate(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(name)).unwrap();
        decode(&v.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>())
    }

    fn bbox(m: &TriMesh) -> ([f32; 3], [f32; 3]) {
        let Positions::F32(p) = &m.positions else { unreachable!() };
        p.iter().fold(([f32::MAX; 3], [f32::MIN; 3]), |(l, h), q| {
            ([l[0].min(q.x), l[1].min(q.y), l[2].min(q.z)], [h[0].max(q.x), h[1].max(q.y), h[2].max(q.z)])
        })
    }

    #[test]
    fn cube() {
        let m = mesh("cube.step");
        assert_eq!(m.triangle_count(), 12);
        assert_eq!(bbox(&m), ([0.0; 3], [1.0; 3]));
        // flat faces: every normal is an axis
        assert!(m.normals.unwrap().iter().all(|n| (n.x.abs() + n.y.abs() + n.z.abs() - 1.0).abs() < 1e-4));
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

    #[test]
    fn not_step_is_error() {
        let f = std::env::temp_dir().join(format!("pcdview-bad-{}.step", std::process::id()));
        std::fs::write(&f, b"ISO-10303-21;\ngarbage").unwrap();
        assert!(tessellate(&f).is_err());
    }
}
