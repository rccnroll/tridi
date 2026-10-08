//! What a file is: a cloud, or a mesh (glb/gltf/obj/stl through
//! three-d-asset, ply/off with faces from the readers, step/stp through
//! step), and which way is up.

use std::{panic, path::Path};
use three_d::{CpuMaterial, CpuModel, Mat4, Srgba, Vec3};
use three_d_asset::{Geometry, Indices, Positions, Primitive, TriMesh, io};

use crate::{
    formats::{Cloud, FormatError, load_cloud},
    step::{self, StepError},
};

/// Why a file couldn't be opened.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error(transparent)]
    Format(#[from] FormatError),
    #[error(transparent)]
    Step(#[from] StepError),
    #[error("not a valid .{ext} file")]
    Asset {
        ext: String,
        #[source]
        source: three_d_asset::Error,
    },
    /// three-d-asset panicked on it (a .obj that isn't UTF-8, for one).
    #[error("not a valid .{ext} file")]
    Panicked { ext: String },
    #[error("no mesh read")]
    NoMesh,
}

pub type LoadResult<T> = Result<T, LoadError>;

/// A file read: a cloud to color, or a mesh ready for the GPU.
pub enum Item {
    Cloud(Cloud),
    Mesh(CpuModel),
}

/// `grey` is the color of meshes without a material (the theme's).
pub fn load(path: &Path, grey: [f32; 3]) -> LoadResult<Item> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let mut model = match ext.as_str() {
        "glb" | "gltf" | "obj" | "stl" => load_asset(path, &ext)?,
        "step" | "stp" => model(step::load(path, grey)?),
        _ => {
            let c = load_cloud(path)?;
            if c.faces.is_none() {
                return Ok(Item::Cloud(c));
            }
            from_faces(c)
        }
    };
    if triangles(&model) == 0 {
        return Err(LoadError::NoMesh);
    }
    // a primitive without a material would be white on white: give it the grey
    let slot = model.materials.len();
    let mut used = false;
    for g in &mut model.geometries {
        if let Geometry::Triangles(t) = &mut g.geometry
            && t.normals.is_none()
        {
            t.compute_normals();
        }
        if g.material_index.is_none() {
            g.material_index = Some(slot);
            used = true;
        }
    }
    if used {
        let white = model
            .geometries
            .iter()
            .any(|g| matches!(&g.geometry, Geometry::Triangles(t) if t.colors.is_some()));
        // with vertex colors the albedo multiplies them: keep it white
        let albedo = if white {
            Srgba::WHITE
        } else {
            Srgba::from([grey[0], grey[1], grey[2], 1.0])
        };
        model.materials.push(CpuMaterial {
            albedo,
            roughness: 0.6,
            metallic: 0.0,
            ..Default::default()
        });
    }
    Ok(Item::Mesh(model))
}

#[must_use]
pub fn triangles(m: &CpuModel) -> usize {
    m.geometries
        .iter()
        .map(|g| match &g.geometry {
            Geometry::Triangles(t) => t.triangle_count(),
            Geometry::Points(_) => 0,
        })
        .sum()
}

/// Whether `p` is glTF, whose standard is Y-up.
#[must_use]
pub fn is_gltf(p: &Path) -> bool {
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    ext == "glb" || ext == "gltf"
}

/// glTF is Y-up by its standard (decided 25/09: we follow it, even though
/// `Open3D` writes scan GLBs Z-up); scans and CAD are Z-up.
#[must_use]
pub fn up_for(gltf: bool) -> Vec3 {
    if gltf { Vec3::unit_y() } else { Vec3::unit_z() }
}

fn load_asset(path: &Path, ext: &str) -> LoadResult<CpuModel> {
    let asset = |source| LoadError::Asset {
        ext: ext.to_owned(),
        source,
    };
    let mut raw = io::load(&[path]).map_err(asset)?;
    // three-d-asset picks the reader from the extension, case-sensitively:
    // SolidWorks writes .STL, so file it under a lowercase name
    let key = path.with_extension(ext);
    if key != path {
        let bytes = raw.remove(path).map_err(asset)?;
        raw.insert(&key, bytes);
    }
    // three-d-asset panics on some files (unwrap on a .obj that isn't UTF-8):
    // make it an error, quietly, so the other files still open
    // ponytail: the panic hook is global; fine single-threaded, parallel tests may lose a message
    let hook = panic::take_hook();
    panic::set_hook(Box::new(|_| {}));
    let r = panic::catch_unwind(panic::AssertUnwindSafe(|| raw.deserialize::<CpuModel>(&key)));
    panic::set_hook(hook);
    #[expect(clippy::map_err_ignore, reason = "the panic payload says nothing a user can act on")]
    let model = r.map_err(|_| LoadError::Panicked { ext: ext.to_owned() })?;
    model.map_err(asset)
}

fn from_faces(c: Cloud) -> CpuModel {
    let colors = c.colors.map(|v| v.iter().map(|c| Srgba::from([c.x, c.y, c.z, 1.0])).collect());
    model(TriMesh {
        positions: Positions::F32(c.points),
        indices: Indices::U32(c.faces.unwrap_or_default()),
        colors,
        ..Default::default()
    })
}

fn model(mesh: TriMesh) -> CpuModel {
    CpuModel {
        name: String::new(),
        geometries: vec![Primitive {
            name: String::new(),
            transformation: Mat4::from_scale(1.0),
            animations: vec![],
            geometry: Geometry::Triangles(mesh),
            material_index: None,
        }],
        materials: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{test_support::tmp, theme::LIGHT};
    use std::fs;

    /// The triangles of a mesh, None for a cloud.
    fn tris(item: Item) -> Option<usize> {
        match item {
            Item::Mesh(m) => Some(triangles(&m)),
            Item::Cloud(_) => None,
        }
    }

    // 8. GLB written by Open3D: data in the JSON chunk as data URIs, no BIN
    //    chunk. assimp can't read it back (1.0 needed a workaround); here it
    //    must be a mesh with its 12 box triangles
    #[test]
    fn open3d_glb() {
        let f = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/open3d_box.glb");
        let raw = fs::read(&f).unwrap();
        let json_len = usize::try_from(u32::from_le_bytes(raw[12..16].try_into().unwrap())).unwrap();
        assert!(20 + json_len >= raw.len(), "the fixture has a BIN chunk: the case isn't tested");
        assert_eq!(tris(load(&f, LIGHT.mesh).unwrap()), Some(12));
    }

    #[test]
    fn ply_with_faces_is_a_mesh() {
        // a quad as one 4-sided face: fan-triangulated into 2
        let ascii = b"ply\nformat ascii 1.0\nelement vertex 4\nproperty float x\nproperty float y\nproperty float z\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n0 0 0\n1 0 0\n1 1 0\n0 1 0\n4 0 1 2 3\n";
        assert_eq!(tris(load(&tmp("q.ply", ascii), LIGHT.mesh).unwrap()), Some(2));

        let mut bin = b"ply\nformat binary_little_endian 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nelement face 1\nproperty list uchar uint vertex_indices\nend_header\n".to_vec();
        for v in [0.0_f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
            bin.extend(v.to_le_bytes());
        }
        bin.push(3);
        for i in [0_u32, 1, 2] {
            bin.extend(i.to_le_bytes());
        }
        assert_eq!(tris(load(&tmp("t.ply", &bin), LIGHT.mesh).unwrap()), Some(1));

        let bad = b"ply\nformat ascii 1.0\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n0 0 0\n3 0 1 2\n";
        assert!(load(&tmp("bad_face.ply", bad), LIGHT.mesh).is_err(), "face index out of range");
    }

    #[test]
    fn off_and_coff() {
        let off = b"OFF\n# a triangle\n3 1 0\n0 0 0\n1 0 0\n0 1 0\n3 0 1 2\n";
        assert_eq!(tris(load(&tmp("t.off", off), LIGHT.mesh).unwrap()), Some(1));
        let coff = b"COFF 4 1 0\n0 0 0 255 0 0 255\n1 0 0 0 255 0 255\n1 1 0 0 0 255 255\n0 1 0 9 9 9 255\n4 0 1 2 3\n";
        assert_eq!(tris(load(&tmp("q.off", coff), LIGHT.mesh).unwrap()), Some(2));
    }

    #[test]
    fn stl_and_obj() {
        let stl = b"solid t\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid t\n";
        assert_eq!(tris(load(&tmp("t.stl", stl), LIGHT.mesh).unwrap()), Some(1));
        // SolidWorks' uppercase extension counts as .stl
        assert_eq!(tris(load(&tmp("T.STL", stl), LIGHT.mesh).unwrap()), Some(1));
        let obj = b"v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf 1 2 3 4\n";
        assert_eq!(tris(load(&tmp("q.obj", obj), LIGHT.mesh).unwrap()), Some(2));
    }

    #[test]
    fn broken_mesh_is_error() {
        assert!(load(&tmp("bad.glb", b"glTF garbage"), LIGHT.mesh).is_err());
        assert!(load(&tmp("bad.stl", b"solid x\nendsolid x\n"), LIGHT.mesh).is_err());
        assert!(load(&tmp("bad.off", b"OFF\n3 1 0\n0 0 0\n"), LIGHT.mesh).is_err());
        // a Windows object file, not a mesh: three-d-asset panics on it
        assert!(load(&tmp("coff.obj", b"d\x86\x07\x00\xff\xfe"), LIGHT.mesh).is_err());
    }
}
