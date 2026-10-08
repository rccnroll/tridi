//! Point cloud readers for pcd, ply, off and xyz/xyzrgb/pts: what Nautilus
//! hands the thumbnailer, so every byte is untrusted. The leaf of the crate:
//! nothing here depends on the rest of tridi.

mod off;
mod pcd;
mod ply;
mod text;

use std::path::Path;
use three_d::{Vec3, vec3};

/// A cloud read from a file, before any coloring.
pub struct Cloud {
    pub points: Vec<Vec3>,
    /// 0..1, one per point, only if the file has them
    pub colors: Option<Vec<Vec3>>,
    /// triangles as index triples, for ply/off files that have faces
    pub faces: Option<Vec<u32>>,
}

/// Reads `path`, picking the reader from its extension (any case).
pub fn load_cloud(path: &Path) -> Result<Cloud, String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    parse_cloud(&std::fs::read(path).map_err(|e| e.to_string())?, &ext)
}

/// A cloud from a file's bytes, `ext` lowercase. What the fuzz targets call.
pub fn parse_cloud(raw: &[u8], ext: &str) -> Result<Cloud, String> {
    let mut c = match ext {
        "pcd" => pcd::read(raw)?,
        "ply" => ply::read(raw)?,
        "off" => off::read(raw)?,
        "xyz" | "xyzrgb" | "pts" => text::read(raw, ext),
        _ => return Err(format!("unsupported format: .{ext}")),
    };
    // NaNs: PCL organized clouds mark missing points that way (a mesh keeps
    // them, or its indices would shift)
    let keep: Vec<bool> = c
        .points
        .iter()
        .map(|p| p.x.is_finite() && p.y.is_finite() && p.z.is_finite())
        .collect();
    if c.faces.is_none() && keep.contains(&false) {
        let mut k = keep.iter();
        c.points.retain(|_| *k.next().unwrap());
        if let Some(cols) = &mut c.colors {
            let mut k = keep.iter();
            cols.retain(|_| *k.next().unwrap());
        }
    }
    if c.points.is_empty() {
        return Err("no points read".into());
    }
    Ok(c)
}

/// Little- or big-endian scalar of PCD/PLY type `ty` (F, U, I) and `size` bytes.
pub(crate) fn scalar(ty: u8, b: &[u8], big: bool) -> f64 {
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

pub(crate) fn valid_type(ty: u8, size: usize) -> bool {
    matches!((ty, size), (b'F', 4 | 8) | (b'U' | b'I', 1 | 2 | 4 | 8))
}

/// rgb packed in 32 bits (PCL writes it as a float, Open3D as U4) -> 0..1
pub(crate) fn unpack_rgb(bits: u32) -> Vec3 {
    let c = |s: u32| ((bits >> s) & 255) as f32 / 255.0;
    vec3(c(16), c(8), c(0))
}

/// Splits `raw` after the header line that starts with `last`; returns the
/// header lines (comments dropped) and the body.
pub(crate) fn split_header<'a>(raw: &'a [u8], last: &str) -> Result<(Vec<Vec<String>>, &'a [u8]), String> {
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

pub(crate) struct Field {
    pub(crate) name: String,
    pub(crate) ty: u8,
    pub(crate) size: usize,
    pub(crate) count: usize,
    /// byte offset within a point (binary) and first column (ascii)
    pub(crate) off: usize,
    pub(crate) col: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::tmp;

    // found by `cargo fuzz` (fuzz/): each panicked or asked for exabytes
    #[test]
    fn lying_headers_are_errors() {
        let pcd = b"FIELDS x y z rgb\nSIZE 4 4 4 4\nTYPE F F F U\nCOUNT 1 1 1 0\nPOINTS 1\nDATA ascii\n0 0 0\n";
        assert!(parse_cloud(pcd, "pcd").is_err());
        let pcd = b"FIELDS x y z\nSIZE 4 4 4\nTYPE F F F\nPOINTS 999999999999999999\nDATA ascii\n0 0 0\n";
        assert_eq!(parse_cloud(pcd, "pcd").unwrap().points.len(), 1);
        let ply = b"ply\nformat ascii 1.0\nelement vertex 999999999999999999\nproperty float x\nproperty float y\nproperty float z\n\
                    element face 999999999999999999\nproperty list uchar int vertex_indices\nend_header\n0 0 0\n";
        assert!(parse_cloud(ply, "ply").is_ok_and(|c| c.points.len() == 1));
        assert!(parse_cloud(b"OFF\n999999999999999999 999999999999999999 0\n0 0 0\n", "off").is_err());
    }

    // 6. unreadable file -> an error, not an empty window
    #[test]
    fn unreadable_is_error() {
        assert!(load_cloud(&tmp("bad.pcd", b"not a point cloud\n")).is_err());
        assert!(load_cloud(&tmp("bad.ply", b"not a point cloud\n")).is_err());
        assert!(load_cloud(&tmp("empty.xyz", b"hello\n")).is_err());
        assert!(load_cloud(&tmp("x.foo", b"1 2 3\n")).is_err());
        // truncated binary and garbage LZF must not panic
        let mut b = b"FIELDS x y z\nSIZE 4 4 4\nTYPE F F F\nPOINTS 10\nDATA binary\n".to_vec();
        b.extend([0u8; 20]);
        assert!(load_cloud(&tmp("short.pcd", &b)).is_err());
        let mut z = b"FIELDS x y z\nSIZE 4 4 4\nTYPE F F F\nPOINTS 2\nDATA binary_compressed\n".to_vec();
        z.extend(8u32.to_le_bytes());
        z.extend(24u32.to_le_bytes());
        z.extend([0xff; 8]);
        assert!(load_cloud(&tmp("lzf.pcd", &z)).is_err());
    }
}
