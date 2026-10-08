//! Point cloud readers for pcd, ply, off and xyz/xyzrgb/pts: what Nautilus
//! hands the thumbnailer, so every byte is untrusted. The leaf of the crate:
//! nothing here depends on the rest of tridi.

// ======================================== Sub-modules ======================================== {{{

mod off;
mod pcd;
mod ply;
mod text;

// }}}

// ========================================== Imports ========================================== {{{

use std::{fs, io, path::Path};
use three_d::{Vec3, vec3};

// }}}

// ========================================== Errors =========================================== {{{

/// Why a file isn't a cloud we can read.
#[derive(Debug, thiserror::Error)]
pub enum FormatError {
    #[error("cannot read the file")]
    Io(#[source] io::Error),
    #[error("unsupported format: .{ext}")]
    Unsupported { ext: String },
    /// The file is the format its extension says, but broken or using a
    /// part of the format we don't read.
    #[error("not a valid {format} file: {reason}")]
    Invalid { format: &'static str, reason: String },
    #[error("no points read")]
    Empty,
}

pub type FormatResult<T> = Result<T, FormatError>;

// }}}

// =========================================== Cloud =========================================== {{{

/// A cloud read from a file, before any coloring.
pub struct Cloud {
    pub points: Vec<Vec3>,
    /// 0..1, one per point, only if the file has them
    pub colors: Option<Vec<Vec3>>,
    /// triangles as index triples, for ply/off files that have faces
    pub faces: Option<Vec<u32>>,
}

/// Reads `path`, picking the reader from its extension (any case).
pub fn load_cloud(path: &Path) -> FormatResult<Cloud> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    parse_cloud(&fs::read(path).map_err(FormatError::Io)?, &ext)
}

/// A cloud from a file's bytes, `ext` lowercase. What the fuzz targets call.
pub fn parse_cloud(raw: &[u8], ext: &str) -> FormatResult<Cloud> {
    let mut c = match ext {
        "pcd" => pcd::read(raw)?,
        "ply" => ply::read(raw)?,
        "off" => off::read(raw)?,
        "xyz" | "xyzrgb" | "pts" => text::read(raw, ext),
        _ => return Err(FormatError::Unsupported { ext: ext.to_owned() }),
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
        c.points.retain(|_| k.next().is_some_and(|&kept| kept));
        if let Some(cols) = &mut c.colors {
            let mut k = keep.iter();
            cols.retain(|_| k.next().is_some_and(|&kept| kept));
        }
    }
    if c.points.is_empty() {
        return Err(FormatError::Empty);
    }
    Ok(c)
}

// }}}

// ========================================== Scalar =========================================== {{{

/// A binary scalar type of PCD and PLY, checked when the header is read: a
/// reader never meets a type or size it doesn't know.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Scalar {
    F32,
    F64,
    U8,
    U16,
    U32,
    U64,
    I8,
    I16,
    I32,
    I64,
}

impl Scalar {
    /// PCD's TYPE letter (F, U, I) and SIZE in bytes.
    pub(crate) fn from_pcd(ty: char, size: usize) -> Option<Self> {
        Some(match (ty, size) {
            ('F', 4) => Self::F32,
            ('F', 8) => Self::F64,
            ('U', 1) => Self::U8,
            ('U', 2) => Self::U16,
            ('U', 4) => Self::U32,
            ('U', 8) => Self::U64,
            ('I', 1) => Self::I8,
            ('I', 2) => Self::I16,
            ('I', 4) => Self::I32,
            ('I', 8) => Self::I64,
            _ => return None,
        })
    }

    /// PLY's type name, either spelling.
    pub(crate) fn from_ply(name: &str) -> Option<Self> {
        Some(match name {
            "char" | "int8" => Self::I8,
            "uchar" | "uint8" => Self::U8,
            "short" | "int16" => Self::I16,
            "ushort" | "uint16" => Self::U16,
            "int" | "int32" => Self::I32,
            "uint" | "uint32" => Self::U32,
            "float" | "float32" => Self::F32,
            "double" | "float64" => Self::F64,
            _ => return None,
        })
    }

    /// Bytes per value.
    pub(crate) fn size(self) -> usize {
        match self {
            Self::U8 | Self::I8 => 1,
            Self::U16 | Self::I16 => 2,
            Self::F32 | Self::U32 | Self::I32 => 4,
            Self::F64 | Self::U64 | Self::I64 => 8,
        }
    }

    pub(crate) fn is_float(self) -> bool {
        matches!(self, Self::F32 | Self::F64)
    }

    /// The value at the start of `b`, little- or big-endian; None if `b` is
    /// shorter than the type.
    pub(crate) fn read(self, b: &[u8], big: bool) -> Option<f64> {
        macro_rules! rd {
            ($t:ty) => {{
                let a = b.get(..size_of::<$t>())?.try_into().ok()?;
                if big { <$t>::from_be_bytes(a) } else { <$t>::from_le_bytes(a) }
            }};
        }
        Some(match self {
            Self::F32 => f64::from(rd!(f32)),
            Self::F64 => rd!(f64),
            Self::U8 => f64::from(rd!(u8)),
            Self::U16 => f64::from(rd!(u16)),
            Self::U32 => f64::from(rd!(u32)),
            Self::I8 => f64::from(rd!(i8)),
            Self::I16 => f64::from(rd!(i16)),
            Self::I32 => f64::from(rd!(i32)),
            #[expect(
                clippy::as_conversions,
                clippy::cast_precision_loss,
                reason = "a coordinate or a color: past 2^53 it was never exact"
            )]
            Self::U64 => rd!(u64) as f64,
            #[expect(
                clippy::as_conversions,
                clippy::cast_precision_loss,
                reason = "a coordinate or a color: past 2^53 it was never exact"
            )]
            Self::I64 => rd!(i64) as f64,
        })
    }

    /// The integer at the start of `b` (a PLY list count or index); None if
    /// `b` is too short, the type is a float, or the value is negative.
    pub(crate) fn read_int(self, b: &[u8], big: bool) -> Option<u64> {
        macro_rules! rd {
            ($t:ty) => {{
                let a = b.get(..size_of::<$t>())?.try_into().ok()?;
                if big { <$t>::from_be_bytes(a) } else { <$t>::from_le_bytes(a) }
            }};
        }
        match self {
            Self::F32 | Self::F64 => None,
            Self::U8 => Some(u64::from(rd!(u8))),
            Self::U16 => Some(u64::from(rd!(u16))),
            Self::U32 => Some(u64::from(rd!(u32))),
            Self::U64 => Some(rd!(u64)),
            Self::I8 => u64::try_from(rd!(i8)).ok(),
            Self::I16 => u64::try_from(rd!(i16)).ok(),
            Self::I32 => u64::try_from(rd!(i32)).ok(),
            Self::I64 => u64::try_from(rd!(i64)).ok(),
        }
    }
}

// }}}

// ========================================== Header =========================================== {{{

/// One field of a point, as the header declares it.
pub(crate) struct Field {
    pub(crate) name: String,
    pub(crate) kind: Scalar,
    pub(crate) count: usize,
    /// byte offset within a point (binary) and first column (ascii)
    pub(crate) off: usize,
    pub(crate) col: usize,
}

/// The first word of a header line.
pub(crate) fn key(line: &[String]) -> &str {
    line.first().map_or("", String::as_str)
}

/// Splits `raw` after the header line that starts with `last`; returns the
/// header lines (comments dropped) and the body. The error is the reason,
/// for the reader's `FormatError::Invalid`.
pub(crate) fn split_header<'a>(raw: &'a [u8], last: &str) -> Result<(Vec<Vec<String>>, &'a [u8]), String> {
    const MAX_HEADER: usize = 64 * 1024;
    let mut lines = vec![];
    let mut off = 0;
    for line in raw.split_inclusive(|&b| b == b'\n') {
        off += line.len();
        let words: Vec<String> = String::from_utf8_lossy(line).split_whitespace().map(str::to_owned).collect();
        let k = key(&words);
        if k.is_empty() || k.starts_with('#') || k == "comment" || k == "obj_info" {
            continue;
        }
        let done = k == last;
        lines.push(words);
        if done {
            return Ok((lines, raw.get(off..).unwrap_or_default()));
        }
        if off > MAX_HEADER {
            break;
        }
    }
    Err(format!("no {last} line: not a valid header"))
}

// }}}

// ========================================== Helpers ========================================== {{{

/// f64 to f32, rounding: what the GPU gets.
pub(crate) fn narrow(v: f64) -> f32 {
    #[expect(clippy::as_conversions, clippy::cast_possible_truncation, reason = "rounding to f32 is the point")]
    let v = v as f32;
    v
}

/// rgb packed in 32 bits (PCL writes it as a float, `Open3D` as U4) -> 0..1
pub(crate) fn unpack_rgb(bits: u32) -> Vec3 {
    let c = |s: u32| f32::from(u8::try_from((bits >> s) & 255).unwrap_or(0)) / 255.0;
    vec3(c(16), c(8), c(0))
}

// }}}

// =========================================== Tests =========================================== {{{

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::tmp;
    use std::env;

    #[test]
    fn errors_say_what_failed() {
        assert!(matches!(parse_cloud(b"1 2 3", "md"), Err(FormatError::Unsupported { ext }) if ext == "md"));
        assert!(matches!(parse_cloud(b"hello\n", "xyz"), Err(FormatError::Empty)));
        assert!(matches!(
            parse_cloud(b"ply\n", "ply"),
            Err(FormatError::Invalid { format: "PLY", .. })
        ));
        let missing = env::temp_dir().join("tridi-no-such-file.pcd");
        assert!(matches!(load_cloud(&missing), Err(FormatError::Io(_))));
    }

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
        b.extend([0_u8; 20]);
        assert!(load_cloud(&tmp("short.pcd", &b)).is_err());
        let mut z = b"FIELDS x y z\nSIZE 4 4 4\nTYPE F F F\nPOINTS 2\nDATA binary_compressed\n".to_vec();
        z.extend(8_u32.to_le_bytes());
        z.extend(24_u32.to_le_bytes());
        z.extend([0xff; 8]);
        assert!(load_cloud(&tmp("lzf.pcd", &z)).is_err());
    }
}

// }}}
