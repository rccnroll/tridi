//! The child's work: the STEP file's solids tessellated with monstertruck,
//! in parallel, each handed on placed where the assembly puts it, as FLOATS
//! f32 per vertex.

// ========================================== Imports ========================================== {{{

use monstertruck_assembly::assy::{EdgeEntity, NodeEntity};
use monstertruck_io::step::load::Table;
use monstertruck_io::step::load::step_p21::{ast::Name, tables::PlaceHolder};
use monstertruck_meshing::prelude::*;
use rayon::ThreadPoolBuilder;
use std::{
    fs, io, iter,
    num::NonZeroUsize,
    path::Path,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    thread,
};

use crate::{
    formats::narrow,
    step::{
        FLOATS, NONE, Quality, StepError, StepResult,
        styles::{Colors, Ents, colors, entities, outer_faces},
    },
};

// }}}

// ========================================= Constants ========================================= {{{

/// Triangles past which a face is the library's blow-up, not the part: on
/// a real assembly two bicubic B-spline faces came out at 3.7M each, the
/// rest of their 429-face solid at a few thousand. Left out.
const FACE_CAP: usize = 100_000;

// }}}

// ======================================= Tessellation ======================================== {{{

/// `ids` first gets the solids to do: the file's, or those of them in `only`
/// if it isn't empty; then `part` gets each one's id and triangles, at every
/// place the assembly uses it, as soon as it's done (empty if the library
/// can't convert it). The solids are spread over the cores: one the library
/// never finishes holds back only itself.
pub(crate) fn tessellate(
    path: &Path,
    quality: Quality,
    only: &[u64],
    ids: impl FnOnce(&[u64]) -> io::Result<()>,
    part: impl Fn(u64, &[f32]) -> io::Result<()> + Sync,
) -> StepResult<()> {
    let raw = fs::read(path).map_err(StepError::Read)?;
    // names and comments are often Latin-1: the geometry is ASCII either way
    let text = String::from_utf8_lossy(&raw);
    #[expect(clippy::map_err_ignore, reason = "see StepError::NotStep")]
    let table = Table::from_step(&text).map_err(|_| StepError::NotStep)?;
    let ents = entities(&text);
    let colors = colors(&ents);
    let mut jobs = placements(&table);
    if !only.is_empty() {
        jobs.retain(|(id, _)| only.contains(id));
    }
    if jobs.is_empty() {
        return Err(StepError::NoSolid);
    }
    ids(&jobs.iter().map(|(id, _)| *id).collect::<Vec<_>>()).map_err(StepError::Write)?;
    let (next, any) = (AtomicUsize::new(0), AtomicBool::new(false));
    let workers = thread::available_parallelism().map_or(1, NonZeroUsize::get).min(jobs.len());
    thread::scope(|s| {
        let handles: Vec<_> = iter::repeat_with(|| {
            s.spawn(|| {
                // a pool of its own, one thread: monstertruck parallelizes in
                // rayon's global pool, where a big solid's faces queued every
                // small solid behind them (52 of 97 in 13 s, the other 45 at
                // 100 s); now a slow solid holds back only its worker
                let pool = ThreadPoolBuilder::new().num_threads(1).build().ok();
                while let Some((id, at)) = jobs.get(next.fetch_add(1, Ordering::Relaxed)) {
                    let mesh = || mesh_item(&table, &ents, &colors, *id, quality);
                    let v = place(&pool.as_ref().map_or_else(mesh, |p| p.install(mesh)), at);
                    any.fetch_or(!v.is_empty(), Ordering::Relaxed);
                    part(*id, &v)?;
                }
                Ok(())
            })
        })
        .take(workers)
        .collect();
        let mut r = Ok(());
        for h in handles {
            // a solid the library panics on is lost, like one it can't
            // convert; its thread's other solids go to the threads left
            if let Ok(Err(e)) = h.join() {
                r = Err(StepError::Write(e));
            }
        }
        r
    })?;
    if !any.into_inner() {
        return Err(StepError::NoSolid);
    }
    Ok(())
}

/// Each solid once, with every placement the assembly gives it; with no
/// assembly the library understands, every solid where it was modelled.
fn placements(table: &Table) -> Vec<(u64, Vec<Matrix4>)> {
    let mut jobs: Vec<(u64, Vec<Matrix4>)> = Vec::new();
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
                for item in solids_of(table, rep) {
                    match jobs.iter_mut().find(|(id, _)| *id == item) {
                        Some((_, at)) => at.push(p.matrix()),
                        None => jobs.push((item, vec![p.matrix()])),
                    }
                }
            }
        }
    }
    if jobs.is_empty() {
        jobs = (table.manifold_solid_brep.keys().chain(table.shell_based_surface_model.keys()))
            .map(|&id| (id, vec![Matrix4::identity()]))
            .collect();
    }
    jobs
}

/// A solid's triangles copied to each of its placements.
fn place(local: &[f32], at: &[Matrix4]) -> Vec<f32> {
    let mut out = Vec::with_capacity(local.len() * at.len());
    for m in at {
        for &[px, py, pz, nx, ny, nz, red, green, blue] in local.as_chunks::<FLOATS>().0 {
            let q = m.transform_point(Point3::new(px.into(), py.into(), pz.into()));
            let n = m.transform_vector(Vector3::new(nx.into(), ny.into(), nz.into())).normalize();
            out.extend([q.x, q.y, q.z, n.x, n.y, n.z].map(narrow));
            out.extend([red, green, blue]);
        }
    }
    out
}

/// The solids of a representation: its own items, and those of the
/// representations tied to it without a transform (a product's
/// `SHAPE_REPRESENTATION` often holds only a placement, and its brep sits in
/// an `ADVANCED_BREP_SHAPE_REPRESENTATION` linked by a relationship).
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
        #[expect(
            clippy::iter_over_hash_type,
            reason = "the order only changes the order of the triangles, not the mesh"
        )]
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

/// Triangles of one `MANIFOLD_SOLID_BREP` or `SHELL_BASED_SURFACE_MODEL`, in its
/// own coordinates; empty if the library can't convert it. A face takes its
/// own color, else the solid's.
fn mesh_item(table: &Table, ents: &Ents, colors: &Colors, id: u64, quality: Quality) -> Vec<f32> {
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
        let tri = match quality {
            Quality::Fine => shell.robust_triangulation(f64::max(bbox.diameter() * 0.001, TOLERANCE)),
            Quality::Coarse => shell.triangulation(f64::max(bbox.diameter() * 0.01, TOLERANCE)),
        };
        let aligned = k == 0 && listed.len() == tri.faces.len();
        for (i, face) in tri.faces.iter().enumerate() {
            let Some(surface) = &face.surface else { continue };
            if surface.faces().len() > FACE_CAP {
                continue;
            }
            let poly = if face.orientation { surface.clone() } else { surface.inverse() };
            let own = || colors.get(<[u64]>::get(&listed, i)?);
            let color = aligned.then(own).flatten().copied().unwrap_or(solid);
            let (positions, normals) = (poly.positions(), poly.normals());
            for corners in poly.faces().triangle_iter() {
                // the library's indices point into its own arrays; `<[_]>::get`,
                // because the prelude brings a trait with a `get` of its own
                let [Some(&a), Some(&b), Some(&c)] = corners.map(|v| <[_]>::get(positions, v.pos)) else {
                    continue;
                };
                // a degenerate triangle shows nothing, and has no normal
                let cross = (b - a).cross(c - a);
                if cross.magnitude2() <= 0.0 {
                    continue;
                }
                let flat = cross.normalize();
                for (v, at) in corners.iter().zip([a, b, c]) {
                    let normal = v.nor.and_then(|i| <[_]>::get(normals, i)).copied().unwrap_or(flat);
                    out.extend([at.x, at.y, at.z, normal.x, normal.y, normal.z].map(narrow));
                    out.extend(color);
                }
            }
        }
    }
    out
}

// }}}
