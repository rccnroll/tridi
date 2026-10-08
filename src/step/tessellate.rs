//! The child's work: the STEP file's solids tessellated with monstertruck,
//! placed where the assembly puts them, as FLOATS f32 per vertex.

use monstertruck_assembly::assy::{EdgeEntity, NodeEntity};
use monstertruck_io::step::load::step_p21::{ast::Name, tables::PlaceHolder};
use monstertruck_io::step::load::*;
use monstertruck_meshing::prelude::*;
use std::collections::HashMap;
use std::path::Path;

use crate::step::{
    FLOATS, NONE,
    styles::{Ents, colors, entities, outer_faces},
};

pub(crate) fn tessellate(path: &Path) -> Result<Vec<f32>, String> {
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
