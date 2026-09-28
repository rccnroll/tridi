//! Point clouds: readers for pcd, ply, xyz/xyzrgb/pts, and the coloring rules
//! of 1.0 (the file's own colors, else the Z ramp, else one tint per file).

use std::path::Path;
use three_d::{Vec3, vec3};

use crate::theme::Theme;

pub struct Cloud {
    pub points: Vec<Vec3>,
    /// 0..1, one per point, only if the file has them
    pub colors: Option<Vec<Vec3>>,
    /// triangles as index triples, for ply/off files that have faces
    pub faces: Option<Vec<u32>>,
}

pub fn load(path: &Path) -> Result<Cloud, String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let raw = std::fs::read(path).map_err(|e| e.to_string())?;
    let mut c = match ext.as_str() {
        "pcd" => read_pcd(&raw)?,
        "ply" => read_ply(&raw)?,
        "off" => read_off(&raw)?,
        "xyz" | "xyzrgb" | "pts" => read_text(&raw, &ext),
        _ => return Err(format!("unsupported format: .{ext}")),
    };
    // NaNs: PCL organized clouds mark missing points that way (a mesh keeps
    // them, or its indices would shift)
    let keep: Vec<bool> = c.points.iter().map(|p| p.x.is_finite() && p.y.is_finite() && p.z.is_finite()).collect();
    if c.faces.is_none() && keep.contains(&false) {
        let mut k = keep.iter();
        c.points.retain(|_| *k.next().unwrap());
        if let Some(cols) = &mut c.colors {
            let mut k = keep.iter();
            cols.retain(|_| *k.next().unwrap());
        }
    }
    if c.points.is_empty() {
        return Err(format!("no points read from {}", path.display()));
    }
    // one color for every point is a placeholder, not a color (our scan
    // pipeline writes grey rgb): treat it as none
    if c.colors.as_ref().is_some_and(|v| v.len() > 1 && v.windows(2).all(|w| w[0] == w[1])) {
        c.colors = None;
    }
    Ok(c)
}

/// The file's colors if it has them, else `tint` flat, else the Z ramp.
pub fn colorize(c: &Cloud, tint: Option<[f32; 3]>, theme: &Theme) -> Vec<Vec3> {
    if let Some(cols) = &c.colors {
        return cols.clone();
    }
    if let Some(t) = tint {
        return vec![Vec3::from(t); c.points.len()];
    }
    let (lo, hi) = c.points.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.z), hi.max(p.z)));
    let span = if hi > lo { hi - lo } else { 1.0 };
    let (a, b) = (Vec3::from(theme.ramp[0]), Vec3::from(theme.ramp[1]));
    c.points.iter().map(|p| a + (b - a) * ((p.z - lo) / span)).collect()
}

/// Little- or big-endian scalar of PCD/PLY type `ty` (F, U, I) and `size` bytes.
fn scalar(ty: u8, b: &[u8], big: bool) -> f64 {
    macro_rules! rd {
        ($t:ty) => {{
            let a = b[..std::mem::size_of::<$t>()].try_into().unwrap();
            (if big { <$t>::from_be_bytes(a) } else { <$t>::from_le_bytes(a) }) as f64
        }};
    }
    match (ty, b.len()) {
        (b'F', 4) => rd!(f32),
        (b'F', 8) => rd!(f64),
        (b'U', 1) => rd!(u8),
        (b'U', 2) => rd!(u16),
        (b'U', 4) => rd!(u32),
        (b'U', 8) => rd!(u64),
        (b'I', 1) => rd!(i8),
        (b'I', 2) => rd!(i16),
        (b'I', 4) => rd!(i32),
        (b'I', 8) => rd!(i64),
        _ => unreachable!("checked when the header is read"),
    }
}

fn valid_type(ty: u8, size: usize) -> bool {
    matches!((ty, size), (b'F', 4 | 8) | (b'U' | b'I', 1 | 2 | 4 | 8))
}

/// rgb packed in 32 bits (PCL writes it as a float, Open3D as U4) -> 0..1
fn unpack_rgb(bits: u32) -> Vec3 {
    let c = |s: u32| ((bits >> s) & 255) as f32 / 255.0;
    vec3(c(16), c(8), c(0))
}

/// Splits `raw` after the header line that starts with `last`; returns the
/// header lines (comments dropped) and the body.
fn split_header<'a>(raw: &'a [u8], last: &str) -> Result<(Vec<Vec<String>>, &'a [u8]), String> {
    let mut lines = vec![];
    let mut off = 0;
    for line in raw.split_inclusive(|&b| b == b'\n') {
        off += line.len();
        let words: Vec<String> = String::from_utf8_lossy(line).split_whitespace().map(str::to_owned).collect();
        if words.is_empty() || words[0].starts_with('#') || words[0] == "comment" || words[0] == "obj_info" {
            continue;
        }
        let done = words[0] == last;
        lines.push(words);
        if done {
            return Ok((lines, &raw[off..]));
        }
        if off > 64 * 1024 {
            break;
        }
    }
    Err(format!("no {last} line: not a valid header"))
}

struct Field {
    name: String,
    ty: u8,
    size: usize,
    count: usize,
    /// byte offset within a point (binary) and first column (ascii)
    off: usize,
    col: usize,
}

fn read_pcd(raw: &[u8]) -> Result<Cloud, String> {
    let (head, body) = split_header(raw, "DATA")?;
    let get = |k: &str| head.iter().find(|l| l[0] == k).map(|l| &l[1..]);
    let names = get("FIELDS").ok_or("PCD without FIELDS")?;
    let sizes = get("SIZE").ok_or("PCD without SIZE")?;
    let types = get("TYPE").ok_or("PCD without TYPE")?;
    let ones = vec!["1".to_owned(); names.len()];
    let counts = get("COUNT").unwrap_or(&ones);
    if sizes.len() != names.len() || types.len() != names.len() || counts.len() != names.len() {
        return Err("FIELDS, SIZE, TYPE and COUNT have different lengths".into());
    }
    let mut fields = vec![];
    let (mut off, mut col) = (0, 0);
    for i in 0..names.len() {
        let size: usize = sizes[i].parse().map_err(|_| "bad SIZE")?;
        let count: usize = counts[i].parse().map_err(|_| "bad COUNT")?;
        let ty = types[i].as_bytes().first().copied().unwrap_or(0);
        if !valid_type(ty, size) {
            return Err(format!("unsupported TYPE {} SIZE {size}", types[i]));
        }
        fields.push(Field { name: names[i].clone(), ty, size, count, off, col });
        off += size * count;
        col += count;
    }
    let stride = off;
    let n: usize = match get("POINTS") {
        Some(v) => v.first().and_then(|s| s.parse().ok()).ok_or("bad POINTS")?,
        None => {
            let dim = |k| get(k).and_then(|v| v.first()?.parse::<usize>().ok()).unwrap_or(1);
            dim("WIDTH") * dim("HEIGHT")
        }
    };
    let field = |name: &str| fields.iter().find(|f| f.name == name);
    let (fx, fy, fz) = match (field("x"), field("y"), field("z")) {
        (Some(x), Some(y), Some(z)) => (x, y, z),
        _ => return Err("PCD without x y z fields".into()),
    };
    let frgb = field("rgb").or(field("rgba"));
    let mode = get("DATA").and_then(|v| v.first()).map(String::as_str).unwrap_or("");

    let mut points = Vec::with_capacity(n);
    let mut colors = frgb.map(|_| Vec::with_capacity(n));
    match mode {
        "ascii" => {
            let text = String::from_utf8_lossy(body);
            for line in text.lines() {
                let t: Vec<&str> = line.split_whitespace().collect();
                if t.len() < col {
                    continue;
                }
                let v = |f: &Field| t[f.col].parse::<f32>().unwrap_or(f32::NAN);
                points.push(vec3(v(fx), v(fy), v(fz)));
                if let (Some(f), Some(cols)) = (frgb, &mut colors) {
                    let bits = if f.ty == b'F' {
                        t[f.col].parse::<f32>().map(f32::to_bits).unwrap_or(0)
                    } else {
                        t[f.col].parse::<f64>().map(|v| v as u32).unwrap_or(0)
                    };
                    cols.push(unpack_rgb(bits));
                }
                if points.len() == n {
                    break;
                }
            }
        }
        "binary" | "binary_compressed" => {
            let total = n.checked_mul(stride).ok_or("POINTS too large")?;
            let data = if mode == "binary" {
                body.get(..total).ok_or("truncated file")?.to_vec()
            } else {
                let word = |i: usize| body.get(i..i + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize);
                let (packed, unpacked) = (word(0).ok_or("truncated file")?, word(4).ok_or("truncated file")?);
                if unpacked != total {
                    return Err("binary_compressed size doesn't match POINTS".into());
                }
                lzf(body.get(8..8 + packed).ok_or("truncated file")?, total)?
            };
            // binary is one point after the other; binary_compressed is one
            // field after the other (all x, then all y, ...)
            let at = |i: usize, f: &Field| -> &[u8] {
                let o = if mode == "binary" { i * stride + f.off } else { n * f.off + i * f.size * f.count };
                &data[o..o + f.size]
            };
            for i in 0..n {
                let v = |f: &Field| scalar(f.ty, at(i, f), false) as f32;
                points.push(vec3(v(fx), v(fy), v(fz)));
                if let (Some(f), Some(cols)) = (frgb, &mut colors) {
                    let b = at(i, f);
                    let bits = if f.size == 4 { u32::from_le_bytes(b.try_into().unwrap()) } else { scalar(f.ty, b, false) as u32 };
                    cols.push(unpack_rgb(bits));
                }
            }
        }
        _ => return Err(format!("unsupported DATA {mode}")),
    }
    Ok(Cloud { points, colors, faces: None })
}

/// LZF, as PCL uses it for binary_compressed.
fn lzf(inp: &[u8], out_len: usize) -> Result<Vec<u8>, String> {
    let bad = || "corrupt binary_compressed data".to_string();
    let mut out = Vec::with_capacity(out_len.min(inp.len().saturating_mul(64)));
    let mut i = 0;
    while i < inp.len() {
        let ctrl = inp[i] as usize;
        i += 1;
        if ctrl < 32 {
            let lit = inp.get(i..i + ctrl + 1).ok_or_else(bad)?;
            out.extend_from_slice(lit);
            i += ctrl + 1;
        } else {
            let mut len = ctrl >> 5;
            if len == 7 {
                len += *inp.get(i).ok_or_else(bad)? as usize;
                i += 1;
            }
            len += 2;
            let back = ((ctrl & 0x1f) << 8) + *inp.get(i).ok_or_else(bad)? as usize + 1;
            i += 1;
            let start = out.len().checked_sub(back).ok_or_else(bad)?;
            for k in 0..len {
                out.push(out[start + k]); // byte by byte: the copy may overlap itself
            }
        }
        if out.len() > out_len {
            return Err(bad());
        }
    }
    if out.len() != out_len {
        return Err(bad());
    }
    Ok(out)
}

fn read_ply(raw: &[u8]) -> Result<Cloud, String> {
    if !raw.starts_with(b"ply") {
        return Err("not a PLY file".into());
    }
    let (head, body) = split_header(raw, "end_header")?;
    let format = head.iter().find(|l| l[0] == "format").and_then(|l| l.get(1)).ok_or("PLY without format")?;
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
        fields.push(Field { name, ty, size, count: 1, off, col });
        off += size;
        col += 1;
    }
    let field = |names: &[&str]| fields.iter().find(|f| names.contains(&f.name.as_str()));
    let (fx, fy, fz) = match (field(&["x"]), field(&["y"]), field(&["z"])) {
        (Some(x), Some(y), Some(z)) => (x, y, z),
        _ => return Err("PLY without x y z".into()),
    };
    let rgb = match (field(&["red", "r", "diffuse_red"]), field(&["green", "g", "diffuse_green"]), field(&["blue", "b", "diffuse_blue"])) {
        (Some(r), Some(g), Some(b)) => Some([r, g, b]),
        _ => None,
    };
    // uchar colors are 0..255, float ones 0..1
    let unit = |f: &Field, v: f64| if f.ty == b'F' { v as f32 } else { v as f32 / 255.0 };

    let mut points = Vec::with_capacity(n);
    let mut colors = rgb.map(|_| Vec::with_capacity(n));
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
        Some(l) if m > 0 && l.get(1).map(String::as_str) == Some("list") && l.len() >= 5 => {
            Some((ply_type(&l[2])?, ply_type(&l[3])?))
        }
        _ if m > 0 => return Err("PLY face element without an index list".into()),
        _ => None,
    };
    if m > 0 && !ascii && head.get(fel + 2).is_some_and(|l| l[0] == "property") {
        return Err("binary PLY faces with extra properties".into());
    }
    let mut faces = list.map(|_| Vec::with_capacity(m * 3));
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

/// OFF / COFF: counts, then vertices (COFF adds r g b a, 0..255 or 0..1),
/// then faces as `k i0 i1 ...`.
fn read_off(raw: &[u8]) -> Result<Cloud, String> {
    let text = String::from_utf8_lossy(raw);
    let mut lines = text.lines().map(|l| l.split('#').next().unwrap_or("").trim()).filter(|l| !l.is_empty());
    let first = lines.next().ok_or("empty OFF")?;
    let kw = first.split_whitespace().next().unwrap_or("");
    if !kw.ends_with("OFF") {
        return Err("not an OFF file".into());
    }
    let colored = kw.starts_with('C');
    // the counts can be on the OFF line itself
    let rest: String = first[kw.len()..].trim().to_owned();
    let counts = if rest.is_empty() { lines.next().ok_or("OFF without counts")?.to_owned() } else { rest };
    let c: Vec<usize> = counts.split_whitespace().map(|s| s.parse().map_err(|_| "bad OFF counts")).collect::<Result<_, _>>()?;
    let (n, m) = (*c.first().ok_or("bad OFF counts")?, *c.get(1).ok_or("bad OFF counts")?);
    let mut points = Vec::with_capacity(n);
    let mut colors = colored.then(|| Vec::with_capacity(n));
    for line in lines.by_ref().take(n) {
        let t: Vec<f32> = line.split_whitespace().map(|s| s.parse().unwrap_or(f32::NAN)).collect();
        if t.len() < 3 {
            return Err("OFF vertex line too short".into());
        }
        points.push(vec3(t[0], t[1], t[2]));
        if let Some(cols) = &mut colors {
            let rgb = t.get(3..6).ok_or("COFF vertex without color")?;
            let k = if rgb.iter().any(|&v| v > 1.0) { 255.0 } else { 1.0 };
            cols.push(vec3(rgb[0], rgb[1], rgb[2]) / k);
        }
    }
    if points.len() < n {
        return Err("truncated OFF".into());
    }
    let mut faces = Vec::with_capacity(m * 3);
    for line in lines.take(m) {
        let t: Vec<u32> = line.split_whitespace().map_while(|s| s.parse().ok()).collect();
        let k = *t.first().ok_or("bad OFF face line")? as usize;
        let idx = t.get(1..1 + k).ok_or("OFF face line too short")?;
        if idx.iter().any(|&i| i as usize >= n) {
            return Err("OFF face index out of range".into());
        }
        for j in 1..k.saturating_sub(1) {
            faces.extend([idx[0], idx[j], idx[j + 1]]);
        }
    }
    Ok(Cloud { points, colors, faces: Some(faces) })
}

/// xyz: x y z; xyzrgb: x y z r g b (0..1); pts: an optional count line, then
/// x y z [intensity] [r g b] (0..255). Spaces or commas; other lines skipped.
fn read_text(raw: &[u8], ext: &str) -> Cloud {
    let mut points = vec![];
    let mut colors = vec![];
    let mut all_colored = true;
    for line in String::from_utf8_lossy(raw).lines() {
        let t: Vec<f32> = match line.split(|c: char| c.is_whitespace() || c == ',').filter(|s| !s.is_empty()).map(str::parse).collect() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if t.len() < 3 {
            continue;
        }
        points.push(vec3(t[0], t[1], t[2]));
        let c = match (ext, t.len()) {
            ("xyzrgb", 6..) => Some(vec3(t[3], t[4], t[5])),
            ("pts", 7..) => Some(vec3(t[4], t[5], t[6]) / 255.0),
            ("pts", 6) => Some(vec3(t[3], t[4], t[5]) / 255.0),
            _ => None,
        };
        match c {
            Some(c) => colors.push(c),
            None => all_colored = false,
        }
    }
    let colors = (all_colored && !points.is_empty()).then_some(colors);
    Cloud { points, colors, faces: None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{LIGHT, THEMES};
    use std::path::PathBuf;

    fn tmp(name: &str, data: &[u8]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pcdview-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, data).unwrap();
        p
    }

    fn ascii_pcd(pts: &[[f32; 3]]) -> Vec<u8> {
        let mut s = format!(
            "# .PCD v0.7\nVERSION 0.7\nFIELDS x y z\nSIZE 4 4 4\nTYPE F F F\nCOUNT 1 1 1\nWIDTH {n}\nHEIGHT 1\nVIEWPOINT 0 0 0 1 0 0 0\nPOINTS {n}\nDATA ascii\n",
            n = pts.len()
        );
        for p in pts {
            s += &format!("{} {} {}\n", p[0], p[1], p[2]);
        }
        s.into_bytes()
    }

    fn close(a: Vec3, b: [f32; 3]) -> bool {
        (a - Vec3::from(b)).magnitude() < 1e-3
    }

    fn mean(c: [f32; 3]) -> f32 {
        c.iter().sum::<f32>() / 3.0
    }

    use three_d::InnerSpace;

    // 1. a single file, no colors -> height ramp, bottom to top
    #[test]
    fn ramp_bottom_to_top() {
        let pts: Vec<[f32; 3]> = (0..500).map(|i| [0.3, 0.7, i as f32 / 50.0]).collect();
        let c = load(&tmp("plain.pcd", &ascii_pcd(&pts))).unwrap();
        assert_eq!(c.points.len(), 500);
        let col = colorize(&c, None, &LIGHT);
        assert!(close(col[0], LIGHT.ramp[0]) && close(col[499], LIGHT.ramp[1]), "{:?} {:?}", col[0], col[499]);
    }

    // 2. the ramp, the tints and the mesh color must read on the background,
    //    in every theme
    #[test]
    fn readable_on_the_background() {
        assert!(mean(LIGHT.bg) > 0.9, "the default is no longer a light background");
        for t in THEMES {
            let far = |c: [f32; 3]| (mean(c) - mean(t.bg)).abs() > 0.2;
            assert!(t.ramp.iter().all(|&c| far(c)), "{}: ramp too close to the background", t.name);
            assert!(t.palette.iter().all(|&c| far(c)), "{}: tint too close to the background", t.name);
            assert!(far(t.mesh), "{}: mesh color too close to the background", t.name);
        }
    }

    // 3. several files -> a different flat tint for each
    #[test]
    fn one_tint_per_file() {
        let mut t = LIGHT.palette.to_vec();
        t.dedup();
        assert_eq!(t.len(), LIGHT.palette.len(), "two files would get the same color");
        let c = load(&tmp("multi.pcd", &ascii_pcd(&[[0.0, 0.0, 0.0], [1.0, 2.0, 3.0]]))).unwrap();
        assert!(colorize(&c, Some(LIGHT.palette[1]), &LIGHT).iter().all(|&v| close(v, LIGHT.palette[1])));
    }

    // 4. colors already present -> untouched, even when a tint is requested
    //    (fixtures written by Open3D: rgb as U4, one ascii and one binary_compressed)
    #[test]
    fn own_colors_untouched() {
        for f in ["rgb_ascii.pcd", "rgb_compressed.pcd"] {
            let c = load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(f)).unwrap();
            assert_eq!(c.points.len(), 40, "{f}");
            assert!(close(c.points[1], [0.5, -1.0, 0.1]) && close(c.points[39], [19.5, -39.0, 152.1]), "{f}");
            let col = colorize(&c, Some(LIGHT.palette[0]), &LIGHT);
            assert!(close(col[0], [0.0, 1.0, 0.2]) && close(col[39], [1.0, 0.0, 0.2]), "{f}: {:?}", col[0]);
        }
    }

    // 4b. one color for every point is a placeholder (the scan pipeline
    //     writes grey rgb): height ramp, and tints with several files
    #[test]
    fn placeholder_color_is_no_color() {
        let c = load(&tmp("grey.xyzrgb", b"0 0 0 0.5 0.5 0.5\n0 0 1 0.5 0.5 0.5\n")).unwrap();
        assert!(c.colors.is_none());
        let c = load(&tmp("two.xyzrgb", b"0 0 0 0.5 0.5 0.5\n0 0 1 0.1 0.5 0.5\n")).unwrap();
        assert!(c.colors.is_some());
    }

    // 5. flat cloud -> no division by zero
    #[test]
    fn flat_cloud() {
        let c = load(&tmp("flat.pcd", &ascii_pcd(&[[0.0; 3]; 10]))).unwrap();
        assert!(colorize(&c, None, &LIGHT).iter().all(|v| v.x.is_finite() && v.y.is_finite() && v.z.is_finite()));
    }

    // 6. unreadable file -> an error, not an empty window
    #[test]
    fn unreadable_is_error() {
        assert!(load(&tmp("bad.pcd", b"not a point cloud\n")).is_err());
        assert!(load(&tmp("bad.ply", b"not a point cloud\n")).is_err());
        assert!(load(&tmp("empty.xyz", b"hello\n")).is_err());
        assert!(load(&tmp("x.foo", b"1 2 3\n")).is_err());
        // truncated binary and garbage LZF must not panic
        let mut b = b"FIELDS x y z\nSIZE 4 4 4\nTYPE F F F\nPOINTS 10\nDATA binary\n".to_vec();
        b.extend([0u8; 20]);
        assert!(load(&tmp("short.pcd", &b)).is_err());
        let mut z = b"FIELDS x y z\nSIZE 4 4 4\nTYPE F F F\nPOINTS 2\nDATA binary_compressed\n".to_vec();
        z.extend(8u32.to_le_bytes());
        z.extend(24u32.to_le_bytes());
        z.extend([0xff; 8]);
        assert!(load(&tmp("lzf.pcd", &z)).is_err());
    }

    // 9. .ply without faces -> a cloud, with its colors
    #[test]
    fn ply_cloud() {
        let ascii = b"ply\nformat ascii 1.0\ncomment x\nelement vertex 2\nproperty float x\nproperty float y\nproperty float z\nproperty uchar red\nproperty uchar green\nproperty uchar blue\nend_header\n0 0 0 255 0 0\n1 2 3 0 0 255\n";
        let c = load(&tmp("a.ply", ascii)).unwrap();
        assert_eq!(c.points.len(), 2);
        assert!(close(c.points[1], [1.0, 2.0, 3.0]));
        assert!(close(c.colors.unwrap()[1], [0.0, 0.0, 1.0]));

        let mut bin = b"ply\nformat binary_little_endian 1.0\nelement vertex 2\nproperty double x\nproperty double y\nproperty double z\nend_header\n".to_vec();
        for v in [0.0f64, 0.0, 0.0, 4.0, 5.0, 6.0] {
            bin.extend(v.to_le_bytes());
        }
        let c = load(&tmp("b.ply", &bin)).unwrap();
        assert!(close(c.points[1], [4.0, 5.0, 6.0]) && c.colors.is_none());
    }

    #[test]
    fn pcd_binary_mixed_fields_and_nan() {
        // the motherboard scans: FIELDS Coord._Z x y z _ with a padding field of COUNT 4
        let mut b = b"FIELDS Coord._Z x y z _\nSIZE 4 4 4 4 1\nTYPE F F F F U\nCOUNT 1 1 1 1 4\nWIDTH 2\nHEIGHT 1\nPOINTS 2\nDATA binary\n".to_vec();
        for p in [[9.0f32, 1.0, 2.0, 3.0], [9.0, f32::NAN, 0.0, 0.0]] {
            p.iter().for_each(|v| b.extend(v.to_le_bytes()));
            b.extend([0u8; 4]);
        }
        let c = load(&tmp("mb.pcd", &b)).unwrap();
        assert_eq!(c.points.len(), 1, "the NaN point is dropped");
        assert!(close(c.points[0], [1.0, 2.0, 3.0]));
    }

    #[test]
    fn pcd_pcl_float_rgb() {
        let packed = f32::from_bits(0x00ff8000);
        let s = format!("FIELDS x y z rgb\nSIZE 4 4 4 4\nTYPE F F F F\nPOINTS 1\nDATA ascii\n1 2 3 {packed:e}\n");
        let c = load(&tmp("pcl.pcd", s.as_bytes())).unwrap();
        assert!(close(c.colors.unwrap()[0], [1.0, 128.0 / 255.0, 0.0]));
    }

    #[test]
    fn text_formats() {
        let c = load(&tmp("a.xyz", b"1 2 3\n4,5,6\n")).unwrap();
        assert_eq!(c.points.len(), 2);
        assert!(c.colors.is_none());
        let c = load(&tmp("a.xyzrgb", b"1 2 3 1 0 0\n")).unwrap();
        assert!(close(c.colors.unwrap()[0], [1.0, 0.0, 0.0]));
        let c = load(&tmp("a.pts", b"2\n1 2 3 -500 255 0 0\n4 5 6 -500 0 255 0\n")).unwrap();
        assert_eq!(c.points.len(), 2);
        assert!(close(c.colors.unwrap()[1], [0.0, 1.0, 0.0]));
    }

    #[test]
    fn lzf_back_reference() {
        // "abcabcabc": literal "abc", then copy 6 bytes from 3 back (overlapping)
        let packed = [2, b'a', b'b', b'c', 4 << 5, 2]; // ctrl: length 4 (+2), offset high bits 0
        assert_eq!(lzf(&packed, 9).unwrap(), b"abcabcabc");
    }
}
