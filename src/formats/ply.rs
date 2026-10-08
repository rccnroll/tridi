//! PLY: ascii and binary, vertices with optional colors, and an optional
//! face element (polygons fan-triangulated).

use three_d::vec3;

use crate::formats::{Cloud, Field, scalar, split_header};

pub(crate) fn read(raw: &[u8]) -> Result<Cloud, String> {
    if !raw.starts_with(b"ply") {
        return Err("not a PLY file".into());
    }
    let (head, body) = split_header(raw, "end_header")?;
    let format = head
        .iter()
        .find(|l| l[0] == "format")
        .and_then(|l| l.get(1))
        .ok_or("PLY without format")?;
    let (ascii, big) = match format.as_str() {
        "ascii" => (true, false),
        "binary_little_endian" => (false, false),
        "binary_big_endian" => (false, true),
        f => return Err(format!("unsupported PLY format {f}")),
    };
    // only the vertex element is read, so it has to come first
    let el = head.iter().position(|l| l[0] == "element").ok_or("PLY without elements")?;
    if head[el].get(1).map(String::as_str) != Some("vertex") {
        return Err("PLY whose first element isn't vertex".into());
    }
    let n: usize = head[el].get(2).and_then(|s| s.parse().ok()).ok_or("bad vertex count")?;
    let mut fields = vec![];
    let (mut off, mut col) = (0, 0);
    let props: Vec<&Vec<String>> = head[el + 1..].iter().take_while(|l| l[0] == "property").collect();
    for l in &props {
        if l.get(1).map(String::as_str) == Some("list") {
            return Err("PLY with a list property in vertex".into());
        }
        let (ty, size) = ply_type(l.get(1).map_or("", String::as_str))?;
        let name = l.get(2).cloned().unwrap_or_default();
        fields.push(Field {
            name,
            ty,
            size,
            count: 1,
            off,
            col,
        });
        off += size;
        col += 1;
    }
    let field = |names: &[&str]| fields.iter().find(|f| names.contains(&f.name.as_str()));
    let (fx, fy, fz) = match (field(&["x"]), field(&["y"]), field(&["z"])) {
        (Some(x), Some(y), Some(z)) => (x, y, z),
        _ => return Err("PLY without x y z".into()),
    };
    let rgb = match (
        field(&["red", "r", "diffuse_red"]),
        field(&["green", "g", "diffuse_green"]),
        field(&["blue", "b", "diffuse_blue"]),
    ) {
        (Some(r), Some(g), Some(b)) => Some([r, g, b]),
        _ => None,
    };
    // uchar colors are 0..255, float ones 0..1
    let unit = |f: &Field, v: f64| if f.ty == b'F' { v as f32 } else { v as f32 / 255.0 };

    // capped by the file's size, as in read_pcd
    let cap = n.min(body.len());
    let mut points = Vec::with_capacity(cap);
    let mut colors = rgb.map(|_| Vec::with_capacity(cap));
    let mut push = |v: &dyn Fn(&Field) -> f64| {
        points.push(vec3(v(fx) as f32, v(fy) as f32, v(fz) as f32));
        if let (Some([r, g, b]), Some(cols)) = (rgb, &mut colors) {
            cols.push(vec3(unit(r, v(r)), unit(g, v(g)), unit(b, v(b))));
        }
    };
    // faces: an optional `element face M` right after vertex, whose only
    // property is the index list
    let fel = el + 1 + props.len();
    let m: usize = match head.get(fel) {
        Some(l) if l[0] == "element" && l.get(1).map(String::as_str) == Some("face") => {
            l.get(2).and_then(|s| s.parse().ok()).ok_or("bad face count")?
        }
        _ => 0,
    };
    let list = match head.get(fel + 1) {
        Some(l) if m > 0 && l.get(1).map(String::as_str) == Some("list") && l.len() >= 5 => Some((ply_type(&l[2])?, ply_type(&l[3])?)),
        _ if m > 0 => return Err("PLY face element without an index list".into()),
        _ => None,
    };
    if m > 0 && !ascii && head.get(fel + 2).is_some_and(|l| l[0] == "property") {
        return Err("binary PLY faces with extra properties".into());
    }
    let mut faces = list.map(|_| Vec::with_capacity(m.min(body.len()) * 3));
    let mut add_face = |idx: &[u32]| -> Result<(), String> {
        if idx.iter().any(|&i| i as usize >= n) {
            return Err("PLY face index out of range".into());
        }
        let f = faces.as_mut().unwrap();
        for k in 1..idx.len().saturating_sub(1) {
            f.extend([idx[0], idx[k], idx[k + 1]]); // fan: polygons become triangles
        }
        Ok(())
    };

    if ascii {
        let text = String::from_utf8_lossy(body);
        let mut lines = text.lines().filter(|l| !l.trim().is_empty());
        for line in lines.by_ref().take(n) {
            let t: Vec<f64> = line.split_whitespace().map(|s| s.parse().unwrap_or(f64::NAN)).collect();
            if t.len() < col {
                return Err("PLY vertex line too short".into());
            }
            push(&|f: &Field| t[f.col]);
        }
        for line in lines.take(m) {
            let t: Vec<u32> = line.split_whitespace().map_while(|s| s.parse().ok()).collect();
            let k = *t.first().ok_or("bad PLY face line")? as usize;
            add_face(t.get(1..1 + k).ok_or("PLY face line too short")?)?;
        }
    } else {
        let stride = off;
        let len = n.checked_mul(stride).ok_or("bad vertex count")?;
        let data = body.get(..len).ok_or("truncated file")?;
        for row in data.chunks_exact(stride) {
            push(&|f: &Field| scalar(f.ty, &row[f.off..f.off + f.size], big));
        }
        if let Some(((cty, csize), (ity, isize))) = list {
            let mut rest = &body[len..];
            let mut take = |size: usize| -> Result<&[u8], String> {
                let (a, b) = rest.split_at_checked(size).ok_or("truncated file")?;
                rest = b;
                Ok(a)
            };
            let mut idx = vec![];
            for _ in 0..m {
                let k = scalar(cty, take(csize)?, big) as usize;
                idx.clear();
                for _ in 0..k {
                    idx.push(scalar(ity, take(isize)?, big) as u32);
                }
                add_face(&idx)?;
            }
        }
    }
    Ok(Cloud { points, colors, faces })
}

fn ply_type(t: &str) -> Result<(u8, usize), String> {
    Ok(match t {
        "char" | "int8" => (b'I', 1),
        "uchar" | "uint8" => (b'U', 1),
        "short" | "int16" => (b'I', 2),
        "ushort" | "uint16" => (b'U', 2),
        "int" | "int32" => (b'I', 4),
        "uint" | "uint32" => (b'U', 4),
        "float" | "float32" => (b'F', 4),
        "double" | "float64" => (b'F', 8),
        t => return Err(format!("unsupported PLY type {t}")),
    })
}

#[cfg(test)]
mod tests {
    use crate::{
        formats::load_cloud,
        test_support::{close, tmp},
    };

    // 9. .ply without faces -> a cloud, with its colors
    #[test]
    fn ply_cloud() {
        let ascii = b"ply\nformat ascii 1.0\ncomment x\nelement vertex 2\nproperty float x\nproperty float y\nproperty float z\nproperty uchar red\nproperty uchar green\nproperty uchar blue\nend_header\n0 0 0 255 0 0\n1 2 3 0 0 255\n";
        let c = load_cloud(&tmp("a.ply", ascii)).unwrap();
        assert_eq!(c.points.len(), 2);
        assert!(close(c.points[1], [1.0, 2.0, 3.0]));
        assert!(close(c.colors.unwrap()[1], [0.0, 0.0, 1.0]));

        let mut bin = b"ply\nformat binary_little_endian 1.0\nelement vertex 2\nproperty double x\nproperty double y\nproperty double z\nend_header\n".to_vec();
        for v in [0.0f64, 0.0, 0.0, 4.0, 5.0, 6.0] {
            bin.extend(v.to_le_bytes());
        }
        let c = load_cloud(&tmp("b.ply", &bin)).unwrap();
        assert!(close(c.points[1], [4.0, 5.0, 6.0]) && c.colors.is_none());
    }
}
