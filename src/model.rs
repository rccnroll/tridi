//! What a file is: a cloud, or a mesh (glb/gltf/obj/stl through
//! three-d-asset, ply/off with faces from the readers, step/stp through
//! step), and which way is up.

use std::path::Path;
use three_d::{CpuMaterial, CpuModel, Srgba, Vec3};
use three_d_asset::{Geometry, Indices, Positions, Primitive, TriMesh};

use crate::{
    formats::{Cloud, load_cloud},
    step,
};

/// A file read: a cloud to color, or a mesh ready for the GPU.
pub enum Item {
    Cloud(Cloud),
    Mesh(CpuModel),
}

/// `grey` is the color of meshes without a material (the theme's).
pub fn load(path: &Path, grey: [f32; 3]) -> Result<Item, String> {
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
        return Err(format!("no mesh read from {}", path.display()));
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

pub fn triangles(m: &CpuModel) -> usize {
    m.geometries
        .iter()
        .map(|g| match &g.geometry {
            Geometry::Triangles(t) => t.triangle_count(),
            _ => 0,
        })
        .sum()
}

/// Whether `p` is glTF, whose standard is Y-up.
pub fn is_gltf(p: &Path) -> bool {
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    ext == "glb" || ext == "gltf"
}

/// glTF is Y-up by its standard (decided 25/09: we follow it, even though
/// Open3D writes scan GLBs Z-up); scans and CAD are Z-up.
pub fn up_for(gltf: bool) -> Vec3 {
    if gltf { Vec3::unit_y() } else { Vec3::unit_z() }
}

fn load_asset(path: &Path, ext: &str) -> Result<CpuModel, String> {
    let mut raw = three_d_asset::io::load(&[path]).map_err(|e| e.to_string())?;
    // three-d-asset picks the reader from the extension, case-sensitively:
    // SolidWorks writes .STL, so file it under a lowercase name
    let key = path.with_extension(ext);
    if key != path {
        let bytes = raw.remove(path).map_err(|e| e.to_string())?;
        raw.insert(&key, bytes);
    }
    // three-d-asset panics on some files (unwrap on a .obj that isn't UTF-8):
    // make it an error, quietly, so the other files still open
    // ponytail: the panic hook is global; fine single-threaded, parallel tests may lose a message
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| raw.deserialize::<CpuModel>(&key)));
    std::panic::set_hook(hook);
    r.map_err(|_| format!("not a valid .{ext} file"))?.map_err(|e| e.to_string())
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
            transformation: three_d::Mat4::from_scale(1.0),
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
    use std::path::PathBuf;

    fn tmp(name: &str, data: &[u8]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tridi-mesh-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, data).unwrap();
        p
    }

    fn tris(item: Item) -> usize {
        match item {
            Item::Mesh(m) => triangles(&m),
            Item::Cloud(_) => panic!("expected a mesh, got a cloud"),
        }
    }

    // 8. GLB written by Open3D: data in the JSON chunk as data URIs, no BIN
    //    chunk. assimp can't read it back (1.0 needed a workaround); here it
    //    must be a mesh with its 12 box triangles
    #[test]
    fn open3d_glb() {
        let f = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/open3d_box.glb");
        let raw = std::fs::read(&f).unwrap();
        let json_len = u32::from_le_bytes(raw[12..16].try_into().unwrap()) as usize;
        assert!(20 + json_len >= raw.len(), "the fixture has a BIN chunk: the case isn't tested");
        assert_eq!(tris(load(&f, crate::theme::LIGHT.mesh).unwrap()), 12);
    }

    #[test]
    fn ply_with_faces_is_a_mesh() {
        // a quad as one 4-sided face: fan-triangulated into 2
        let ascii = b"ply\nformat ascii 1.0\nelement vertex 4\nproperty float x\nproperty float y\nproperty float z\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n0 0 0\n1 0 0\n1 1 0\n0 1 0\n4 0 1 2 3\n";
        assert_eq!(tris(load(&tmp("q.ply", ascii), crate::theme::LIGHT.mesh).unwrap()), 2);

        let mut bin = b"ply\nformat binary_little_endian 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nelement face 1\nproperty list uchar uint vertex_indices\nend_header\n".to_vec();
        for v in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
            bin.extend(v.to_le_bytes());
        }
        bin.push(3);
        for i in [0u32, 1, 2] {
            bin.extend(i.to_le_bytes());
        }
        assert_eq!(tris(load(&tmp("t.ply", &bin), crate::theme::LIGHT.mesh).unwrap()), 1);

        let bad = b"ply\nformat ascii 1.0\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n0 0 0\n3 0 1 2\n";
        assert!(
            load(&tmp("bad.ply", bad), crate::theme::LIGHT.mesh).is_err(),
            "face index out of range"
        );
    }

    #[test]
    fn off_and_coff() {
        let off = b"OFF\n# a triangle\n3 1 0\n0 0 0\n1 0 0\n0 1 0\n3 0 1 2\n";
        assert_eq!(tris(load(&tmp("t.off", off), crate::theme::LIGHT.mesh).unwrap()), 1);
        let coff = b"COFF 4 1 0\n0 0 0 255 0 0 255\n1 0 0 0 255 0 255\n1 1 0 0 0 255 255\n0 1 0 9 9 9 255\n4 0 1 2 3\n";
        assert_eq!(tris(load(&tmp("q.off", coff), crate::theme::LIGHT.mesh).unwrap()), 2);
    }

    #[test]
    fn stl_and_obj() {
        let stl = b"solid t\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid t\n";
        assert_eq!(tris(load(&tmp("t.stl", stl), crate::theme::LIGHT.mesh).unwrap()), 1);
        // SolidWorks' uppercase extension counts as .stl
        assert_eq!(tris(load(&tmp("T.STL", stl), crate::theme::LIGHT.mesh).unwrap()), 1);
        let obj = b"v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf 1 2 3 4\n";
        assert_eq!(tris(load(&tmp("q.obj", obj), crate::theme::LIGHT.mesh).unwrap()), 2);
    }

    #[test]
    fn broken_mesh_is_error() {
        assert!(load(&tmp("bad.glb", b"glTF garbage"), crate::theme::LIGHT.mesh).is_err());
        assert!(load(&tmp("bad.stl", b"solid x\nendsolid x\n"), crate::theme::LIGHT.mesh).is_err());
        assert!(load(&tmp("bad.off", b"OFF\n3 1 0\n0 0 0\n"), crate::theme::LIGHT.mesh).is_err());
        // a Windows object file, not a mesh: three-d-asset panics on it
        assert!(load(&tmp("coff.obj", b"d\x86\x07\x00\xff\xfe"), crate::theme::LIGHT.mesh).is_err());
    }
}
