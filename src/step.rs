//! STEP (.step/.stp) through monstertruck. Tessellation runs in a child
//! process, `tridi step-mesh IN`, killed at DEADLINE: on some real files
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
use three_d::{Srgba, Vec3};
use three_d_asset::{Indices, Positions, TriMesh};

// ponytail: one deadline for viewer and thumbnails; the biggest real part (476 faces) takes 0.5 s
const DEADLINE: Duration = Duration::from_secs(15);

/// A vertex on the wire: position, normal, color (NONE when the file gives
/// the face none).
const FLOATS: usize = 9;
const NONE: [f32; 3] = [-1.0; 3];

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

fn tessellate(path: &Path) -> Result<Vec<f32>, String> {
    let raw = std::fs::read(path).map_err(|e| e.to_string())?;
    // names and comments are often Latin-1: the geometry is ASCII either way
    let text = String::from_utf8_lossy(&raw);
    let table = Table::from_step(&text).map_err(|e| format!("not a STEP file: {e:?}"))?;
    let ents = entities(&text);
    let colors = colors(&ents);
    let mut out = Vec::new();
    // each solid tessellated once, however many times the assembly uses it
    let mut cache = HashMap::<u64, Vec<f32>>::new();
    if let Ok(assy) = table.step_assy() {
        let assy = assy.map(
            |n| NodeEntity {
                shape: n.attrs.shape_representation,
                attrs: (),
            },
            |e| EdgeEntity {
                matrix: Matrix4::try_from(&e.matrix).unwrap_or(Matrix4::identity()),
                attrs: (),
            },
        );
        for top in assy.top_nodes() {
            for p in assy.paths_iter(top.index()) {
                let Some(rep) = *p.terminal_node().shape() else { continue };
                let m = p.matrix();
                for item in solids_of(&table, rep) {
                    let local = cache.entry(item).or_insert_with(|| mesh_item(&table, &ents, &colors, item));
                    for v in local.as_chunks::<FLOATS>().0 {
                        let q = m.transform_point(Point3::new(v[0].into(), v[1].into(), v[2].into()));
                        let n = m.transform_vector(Vector3::new(v[3].into(), v[4].into(), v[5].into())).normalize();
                        out.extend([q.x, q.y, q.z, n.x, n.y, n.z].map(|x| x as f32));
                        out.extend(&v[6..]);
                    }
                }
            }
        }
    }
    // no assembly the library understands: every solid where it was modelled
    if out.is_empty() {
        for &id in table.manifold_solid_brep.keys().chain(table.shell_based_surface_model.keys()) {
            out.extend(mesh_item(&table, &ents, &colors, id));
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
            let (PlaceHolder::Ref(Name::Entity(a)), PlaceHolder::Ref(Name::Entity(b))) = (&srr.rep_1, &srr.rep_2) else {
                continue;
            };
            let other = if *a == r {
                *b
            } else if *b == r {
                *a
            } else {
                continue;
            };
            if !seen.contains(&other) {
                seen.push(other);
                todo.push(other);
            }
        }
    }
    found
}

/// Triangles of one MANIFOLD_SOLID_BREP or SHELL_BASED_SURFACE_MODEL, in its
/// own coordinates; empty if the library can't convert it. A face takes its
/// own color, else the solid's.
fn mesh_item(table: &Table, ents: &Ents, colors: &HashMap<u64, [f32; 3]>, id: u64) -> Vec<f32> {
    let shells = if let Some(s) = table.manifold_solid_brep.get(&id) {
        table.to_compressed_solid(s).map(|s| s.boundaries).unwrap_or_default()
    } else if let Some(s) = table.shell_based_surface_model.get(&id) {
        table.to_compressed_shells(s).unwrap_or_default()
    } else {
        Vec::new()
    };
    let solid = colors.get(&id).copied().unwrap_or(NONE);
    // the outer shell's faces as the file lists them: the library keeps their
    // order but drops the ones it can't read, so they line up only if none was
    let listed = outer_faces(ents, id);
    let mut out = Vec::new();
    for (k, shell) in shells.into_iter().enumerate() {
        let bbox: BoundingBox<Point3> = shell.vertices.iter().collect();
        let tri = shell.robust_triangulation(f64::max(bbox.diameter() * 0.001, TOLERANCE));
        let aligned = k == 0 && listed.len() == tri.faces.len();
        for (i, face) in tri.faces.iter().enumerate() {
            let Some(surface) = &face.surface else { continue };
            let mut poly = if face.orientation { surface.clone() } else { surface.inverse() };
            poly.put_together_same_attrs(TOLERANCE * 50.0).remove_degenerate_faces();
            let color = aligned.then(|| colors.get(&listed[i])).flatten().copied().unwrap_or(solid);
            let (p, n) = (poly.positions(), poly.normals());
            for t in poly.faces().triangle_iter() {
                let [a, b, c] = t.map(|v| p[v.pos]);
                let flat = (b - a).cross(c - a).normalize();
                for v in t {
                    let q = p[v.pos];
                    let m = v.nor.map_or(flat, |i| n[i]);
                    out.extend([q.x, q.y, q.z, m.x, m.y, m.z].map(|x| x as f32));
                    out.extend(color);
                }
            }
        }
    }
    out
}

/// The DATA section as `id -> (NAME, arguments)`, simple entities only
/// (complex ones, `#1 = ( A() B() );`, are relationships we don't need
/// here). monstertruck drops the style entities, so colors are read here.
type Ents<'a> = HashMap<u64, (&'a str, &'a str)>;

fn entities(text: &str) -> Ents<'_> {
    let data = text.find("DATA;").map_or(text, |i| &text[i + 5..]);
    let mut out = HashMap::new();
    let (mut start, mut quoted) = (0, false);
    for (i, c) in data.char_indices() {
        match c {
            '\'' => quoted = !quoted,
            ';' if !quoted => {
                let stmt = data[start..i].trim();
                start = i + 1;
                let Some((id, rest)) = stmt.strip_prefix('#').and_then(|s| s.split_once('=')) else {
                    continue;
                };
                let (Ok(id), Some((name, args))) = (id.trim().parse(), rest.split_once('(')) else {
                    continue;
                };
                let name = name.trim();
                if !name.is_empty() {
                    out.insert(id, (name, args.trim_end().strip_suffix(')').unwrap_or(args)));
                }
            }
            _ => {}
        }
    }
    out
}

/// The `#n` references in some arguments, in order, strings skipped.
fn refs(args: &str) -> Vec<u64> {
    let (mut out, mut quoted, mut it) = (Vec::new(), false, args.chars().peekable());
    while let Some(c) = it.next() {
        match c {
            '\'' => quoted = !quoted,
            '#' if !quoted => {
                let digits: String = std::iter::from_fn(|| it.next_if(char::is_ascii_digit)).collect();
                if let Ok(n) = digits.parse() {
                    out.push(n);
                }
            }
            _ => {}
        }
    }
    out
}

/// Item id -> color, from STYLED_ITEM (and OVER_RIDING_STYLED_ITEM). An item
/// is a face, a solid, or a representation, whose color then goes to its
/// items that have none of their own.
fn colors(ents: &Ents) -> HashMap<u64, [f32; 3]> {
    let mut direct = HashMap::new();
    for (name, args) in ents.values() {
        let r = refs(args);
        let item = match *name {
            "STYLED_ITEM" if !r.is_empty() => r.len() - 1,
            "OVER_RIDING_STYLED_ITEM" if r.len() >= 2 => r.len() - 2,
            _ => continue,
        };
        if let Some(c) = r[..item].iter().find_map(|&s| colour(ents, s, 0)) {
            direct.insert(r[item], c);
        }
    }
    let mut all = direct.clone();
    for (id, c) in &direct {
        if ents.get(id).is_some_and(|(name, _)| name.ends_with("REPRESENTATION")) {
            for item in refs(ents[id].1) {
                all.entry(item).or_insert(*c);
            }
        }
    }
    all
}

/// The surface color a style chain ends in (PRESENTATION_STYLE_ASSIGNMENT ->
/// SURFACE_STYLE_USAGE -> … -> COLOUR_RGB), curve and point styles skipped.
fn colour(ents: &Ents, id: u64, depth: u8) -> Option<[f32; 3]> {
    let (name, args) = ents.get(&id)?;
    match *name {
        "COLOUR_RGB" => {
            // after the name, which may hold commas
            let rgb: Vec<f32> = args.rsplit('\'').next()?.split(',').filter_map(|v| v.trim().parse().ok()).collect();
            (rgb.len() == 3).then(|| [rgb[0], rgb[1], rgb[2]])
        }
        "DRAUGHTING_PRE_DEFINED_COLOUR" => Some(match args.trim().trim_matches('\'') {
            "red" => [1.0, 0.0, 0.0],
            "green" => [0.0, 1.0, 0.0],
            "blue" => [0.0, 0.0, 1.0],
            "yellow" => [1.0, 1.0, 0.0],
            "magenta" => [1.0, 0.0, 1.0],
            "cyan" => [0.0, 1.0, 1.0],
            "black" => [0.0, 0.0, 0.0],
            "white" => [1.0, 1.0, 1.0],
            _ => return None,
        }),
        "CURVE_STYLE" | "POINT_STYLE" | "TEXT_STYLE" => None,
        _ if depth < 10 => refs(args).into_iter().find_map(|r| colour(ents, r, depth + 1)),
        _ => None,
    }
}

/// The faces of a solid's outer shell, ORIENTED_FACE resolved to the face.
fn outer_faces(ents: &Ents, solid: u64) -> Vec<u64> {
    let Some((_, args)) = ents.get(&solid) else { return Vec::new() };
    let Some(&shell) = refs(args).first() else { return Vec::new() };
    let Some((_, shell_args)) = ents.get(&shell) else {
        return Vec::new();
    };
    refs(shell_args)
        .into_iter()
        .map(|f| match ents.get(&f) {
            Some(("ORIENTED_FACE", a)) => refs(a).first().copied().unwrap_or(f),
            _ => f,
        })
        .collect()
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
