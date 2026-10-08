//! PLY: ascii and binary, vertices with optional colors, and an optional
//! face element (polygons fan-triangulated).

use three_d::vec3;

use crate::formats::{Cloud, Field, FormatError, FormatResult, Scalar, key, narrow, split_header};

pub(crate) fn read(raw: &[u8]) -> FormatResult<Cloud> {
    parse(raw).map_err(|reason| FormatError::Invalid { format: "PLY", reason })
}

/// The reader proper; the error is the reason the file is invalid.
fn parse(raw: &[u8]) -> Result<Cloud, String> {
    if !raw.starts_with(b"ply") {
        return Err("not a PLY file".into());
    }
    let (head, body) = split_header(raw, "end_header")?;
    let format = head.iter().find(|l| key(l) == "format").ok_or("PLY without format")?;
    let (ascii, big) = match word(format, 1) {
        "ascii" => (true, false),
        "binary_little_endian" => (false, false),
        "binary_big_endian" => (false, true),
        f => return Err(format!("unsupported PLY format {f}")),
    };
    // only the vertex element is read, so it has to come first
    let el = head.iter().position(|l| key(l) == "element").ok_or("PLY without elements")?;
    let vertex = head.get(el).map_or(&[][..], Vec::as_slice);
    if word(vertex, 1) != "vertex" {
        return Err("PLY whose first element isn't vertex".into());
    }
    let n: usize = word(vertex, 2).parse().map_err(|_err| "bad vertex count")?;
    let after = head.get(el + 1..).unwrap_or_default();
    let props: Vec<&Vec<String>> = after.iter().take_while(|l| key(l) == "property").collect();
    let (fields, stride, col) = vertex_fields(&props)?;
    let field = |names: &[&str]| fields.iter().find(|f| names.contains(&f.name.as_str()));
    let (Some(fx), Some(fy), Some(fz)) = (field(&["x"]), field(&["y"]), field(&["z"])) else {
        return Err("PLY without x y z".into());
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
    let unit = |f: &Field, v: f64| if f.kind.is_float() { narrow(v) } else { narrow(v) / 255.0 };

    // capped by the file's size, as in pcd.rs
    let cap = n.min(body.len());
    let mut points = Vec::with_capacity(cap);
    let mut colors = rgb.map(|_| Vec::with_capacity(cap));
    let mut push = |v: &dyn Fn(&Field) -> f64| {
        points.push(vec3(narrow(v(fx)), narrow(v(fy)), narrow(v(fz))));
        if let (Some([r, g, b]), Some(cols)) = (rgb, &mut colors) {
            cols.push(vec3(unit(r, v(r)), unit(g, v(g)), unit(b, v(b))));
        }
    };
    // faces: an optional `element face M` right after vertex, whose only
    // property is the index list
    let fel = el + 1 + props.len();
    let m: usize = match head.get(fel) {
        Some(l) if key(l) == "element" && word(l, 1) == "face" => word(l, 2).parse().map_err(|_err| "bad face count")?,
        _ => 0,
    };
    let list = face_list(&head, fel, m, ascii)?;
    let mut faces = list.map(|_| Vec::with_capacity(m.min(body.len()) * 3));
    let mut add_face = |idx: &[u32]| -> Result<(), String> {
        if idx.iter().any(|&i| usize::try_from(i).map_or(true, |i| i >= n)) {
            return Err("PLY face index out of range".into());
        }
        if let (Some(f), Some((&first, rest))) = (faces.as_mut(), idx.split_first()) {
            // fan: polygons become triangles
            for w in rest.windows(2) {
                if let [a, b] = *w {
                    f.extend([first, a, b]);
                }
            }
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
            push(&|f: &Field| t.get(f.col).copied().unwrap_or(f64::NAN));
        }
        for line in lines.take(m) {
            let t: Vec<u32> = line.split_whitespace().map_while(|s| s.parse().ok()).collect();
            let (&k, rest) = t.split_first().ok_or("bad PLY face line")?;
            let k = usize::try_from(k).map_err(|_err| "bad PLY face line")?;
            add_face(rest.get(..k).ok_or("PLY face line too short")?)?;
        }
    } else {
        let len = n.checked_mul(stride).ok_or("bad vertex count")?;
        let data = body.get(..len).ok_or("truncated file")?;
        for row in data.chunks_exact(stride) {
            push(&|f: &Field| row.get(f.off..).and_then(|b| f.kind.read(b, big)).unwrap_or(f64::NAN));
        }
        if let Some(list) = list {
            binary_faces(body.get(len..).unwrap_or_default(), list, m, big, &mut add_face)?;
        }
    }
    Ok(Cloud { points, colors, faces })
}

/// The vertex element's properties as fields, with the bytes per vertex
/// (binary) and the columns per line (ascii).
fn vertex_fields(props: &[&Vec<String>]) -> Result<(Vec<Field>, usize, usize), String> {
    let mut fields = vec![];
    let (mut off, mut col) = (0, 0);
    for l in props {
        if word(l, 1) == "list" {
            return Err("PLY with a list property in vertex".into());
        }
        let kind = ply_type(word(l, 1))?;
        fields.push(Field {
            name: word(l, 2).to_owned(),
            kind,
            count: 1,
            off,
            col,
        });
        off += kind.size();
        col += 1;
    }
    Ok((fields, off, col))
}

/// The face element's index list, `(count type, index type)`: None without
/// faces. Its only property must be that list.
fn face_list(head: &[Vec<String>], fel: usize, m: usize, ascii: bool) -> Result<Option<(Scalar, Scalar)>, String> {
    if m == 0 {
        return Ok(None);
    }
    let Some(l) = head.get(fel + 1).filter(|l| word(l, 1) == "list" && l.len() >= 5) else {
        return Err("PLY face element without an index list".into());
    };
    if !ascii && head.get(fel + 2).is_some_and(|l| key(l) == "property") {
        return Err("binary PLY faces with extra properties".into());
    }
    Ok(Some((ply_type(word(l, 2))?, ply_type(word(l, 3))?)))
}

/// `m` binary faces from `body`, each a count then that many indices.
fn binary_faces(
    mut body: &[u8],
    (count, index): (Scalar, Scalar),
    m: usize,
    big: bool,
    add_face: &mut dyn FnMut(&[u32]) -> Result<(), String>,
) -> Result<(), String> {
    let mut take = |kind: Scalar| -> Result<u64, String> {
        let (a, b) = body.split_at_checked(kind.size()).ok_or("truncated file")?;
        body = b;
        kind.read_int(a, big).ok_or_else(|| "bad PLY face index".to_owned())
    };
    let mut idx = vec![];
    for _ in 0..m {
        let k = take(count)?;
        idx.clear();
        for _ in 0..k {
            idx.push(u32::try_from(take(index)?).map_err(|_err| "PLY face index out of range")?);
        }
        add_face(&idx)?;
    }
    Ok(())
}

/// The `i`-th word of a header line, empty if there is none.
fn word(line: &[String], i: usize) -> &str {
    line.get(i).map_or("", String::as_str)
}

fn ply_type(t: &str) -> Result<Scalar, String> {
    Scalar::from_ply(t).ok_or_else(|| format!("unsupported PLY type {t}"))
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
        for v in [0.0_f64, 0.0, 0.0, 4.0, 5.0, 6.0] {
            bin.extend(v.to_le_bytes());
        }
        let c = load_cloud(&tmp("b.ply", &bin)).unwrap();
        assert!(close(c.points[1], [4.0, 5.0, 6.0]) && c.colors.is_none());
    }
}
