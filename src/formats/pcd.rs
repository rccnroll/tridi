//! PCD: ascii, binary and `binary_compressed` (LZF), as PCL and `Open3D` write it.

// ========================================== Imports ========================================== {{{

use three_d::vec3;

use crate::formats::{Cloud, Field, FormatError, FormatResult, Scalar, key, narrow, split_header, unpack_rgb};

// }}}

// ============================================ PCD ============================================ {{{

pub(crate) fn read(raw: &[u8]) -> FormatResult<Cloud> {
    parse(raw).map_err(|reason| FormatError::Invalid { format: "PCD", reason })
}

/// The reader proper; the error is the reason the file is invalid.
fn parse(raw: &[u8]) -> Result<Cloud, String> {
    let (head, body) = split_header(raw, "DATA")?;
    let get = |k: &str| head.iter().find(|l| key(l) == k).and_then(|l| l.get(1..));
    let names = get("FIELDS").ok_or("PCD without FIELDS")?;
    let sizes = get("SIZE").ok_or("PCD without SIZE")?;
    let types = get("TYPE").ok_or("PCD without TYPE")?;
    let ones = vec!["1".to_owned(); names.len()];
    let counts = get("COUNT").unwrap_or(&ones);
    if sizes.len() != names.len() || types.len() != names.len() || counts.len() != names.len() {
        return Err("FIELDS, SIZE, TYPE and COUNT have different lengths".into());
    }
    let (fields, stride, col) = header_fields(names, sizes, types, counts)?;
    let n: usize = if let Some(v) = get("POINTS") {
        v.first().and_then(|s| s.parse().ok()).ok_or("bad POINTS")?
    } else {
        let dim = |k| get(k).and_then(|v| v.first()?.parse::<usize>().ok()).unwrap_or(1);
        dim("WIDTH").checked_mul(dim("HEIGHT")).ok_or("bad WIDTH or HEIGHT")?
    };
    let field = |name: &str| fields.iter().find(|f| f.name == name);
    let (Some(fx), Some(fy), Some(fz)) = (field("x"), field("y"), field("z")) else {
        return Err("PCD without x y z fields".into());
    };
    let frgb = field("rgb").or_else(|| field("rgba"));
    let mode = get("DATA").and_then(|v| v.first()).map_or("", String::as_str);

    // every point takes at least a byte: a header can't make us allocate
    // more than the file could hold
    let cap = n.min(body.len());
    let mut points = Vec::with_capacity(cap);
    let mut colors = frgb.map(|_| Vec::with_capacity(cap));
    match mode {
        "ascii" => {
            let text = String::from_utf8_lossy(body);
            for line in text.lines() {
                let t: Vec<&str> = line.split_whitespace().collect();
                if t.len() < col {
                    continue;
                }
                let v = |f: &Field| t.get(f.col).and_then(|s| s.parse::<f32>().ok()).unwrap_or(f32::NAN);
                points.push(vec3(v(fx), v(fy), v(fz)));
                if let (Some(f), Some(cols)) = (frgb, &mut colors) {
                    let word = t.get(f.col).copied().unwrap_or("");
                    let bits = if f.kind.is_float() {
                        word.parse::<f32>().map_or(0, f32::to_bits)
                    } else {
                        word.parse::<u32>().unwrap_or(0)
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
                let word = |i: usize| -> Option<usize> {
                    let b = body.get(i..i.checked_add(4)?)?;
                    usize::try_from(u32::from_le_bytes(b.try_into().ok()?)).ok()
                };
                let (packed, unpacked) = (word(0).ok_or("truncated file")?, word(4).ok_or("truncated file")?);
                if unpacked != total {
                    return Err("binary_compressed size doesn't match POINTS".into());
                }
                let packed = 8_usize
                    .checked_add(packed)
                    .and_then(|end| body.get(8..end))
                    .ok_or("truncated file")?;
                lzf(packed, total)?
            };
            // binary is one point after the other; binary_compressed is one
            // field after the other (all x, then all y, ...). Both stay
            // inside `data`: n * stride bytes, checked above
            let at = |i: usize, f: &Field| -> &[u8] {
                let o = if mode == "binary" {
                    i * stride + f.off
                } else {
                    n * f.off + i * f.kind.size() * f.count
                };
                data.get(o..).unwrap_or_default()
            };
            for i in 0..n {
                let v = |f: &Field| f.kind.read(at(i, f), false).map_or(f32::NAN, narrow);
                points.push(vec3(v(fx), v(fy), v(fz)));
                if let (Some(f), Some(cols)) = (frgb, &mut colors) {
                    let b = at(i, f);
                    // a 4-byte rgb is the packed bits, whatever its TYPE says
                    let bits = if f.kind.size() == 4 {
                        b.get(..4).and_then(|w| w.try_into().ok()).map_or(0, u32::from_le_bytes)
                    } else {
                        f.kind.read_int(b, false).and_then(|v| u32::try_from(v).ok()).unwrap_or(0)
                    };
                    cols.push(unpack_rgb(bits));
                }
            }
        }
        _ => return Err(format!("unsupported DATA {mode}")),
    }
    Ok(Cloud {
        points,
        colors,
        faces: None,
    })
}

// }}}

// ========================================== Helpers ========================================== {{{

/// The FIELDS, SIZE, TYPE and COUNT lines as fields, with the bytes per
/// point (binary) and the columns per line (ascii).
fn header_fields(names: &[String], sizes: &[String], types: &[String], counts: &[String]) -> Result<(Vec<Field>, usize, usize), String> {
    let mut fields = vec![];
    let (mut off, mut col) = (0_usize, 0_usize);
    for (((name, size), ty), count) in names.iter().zip(sizes).zip(types).zip(counts) {
        let size: usize = size.parse().map_err(|_err| "bad SIZE")?;
        let count: usize = count.parse().ok().filter(|&c| c > 0).ok_or("bad COUNT")?;
        let kind = ty
            .chars()
            .next()
            .and_then(|t| Scalar::from_pcd(t, size))
            .ok_or_else(|| format!("unsupported TYPE {ty} SIZE {size}"))?;
        fields.push(Field {
            name: name.clone(),
            kind,
            count,
            off,
            col,
        });
        off = size.checked_mul(count).and_then(|b| off.checked_add(b)).ok_or("bad COUNT")?;
        col = col.checked_add(count).ok_or("bad COUNT")?;
    }
    Ok((fields, off, col))
}

/// LZF, as PCL uses it for `binary_compressed`.
fn lzf(inp: &[u8], out_len: usize) -> Result<Vec<u8>, String> {
    let bad = || "corrupt binary_compressed data".to_owned();
    let mut out = Vec::with_capacity(out_len.min(inp.len().saturating_mul(64)));
    let mut rest = inp;
    while let Some((&ctrl, tail)) = rest.split_first() {
        rest = tail;
        let ctrl = usize::from(ctrl);
        if ctrl < 32 {
            let (lit, tail) = rest.split_at_checked(ctrl + 1).ok_or_else(bad)?;
            out.extend_from_slice(lit);
            rest = tail;
        } else {
            let mut len = ctrl >> 5;
            if len == 7 {
                let (&more, tail) = rest.split_first().ok_or_else(bad)?;
                len += usize::from(more);
                rest = tail;
            }
            len += 2;
            let (&low, tail) = rest.split_first().ok_or_else(bad)?;
            rest = tail;
            let back = ((ctrl & 0x1f) << 8) + usize::from(low) + 1;
            let start = out.len().checked_sub(back).ok_or_else(bad)?;
            for k in start..start + len {
                // byte by byte: the copy may overlap itself
                let b = *out.get(k).ok_or_else(bad)?;
                out.push(b);
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

// }}}

// =========================================== Tests =========================================== {{{

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        formats::load_cloud,
        test_support::{close, tmp},
    };

    #[test]
    fn pcd_binary_mixed_fields_and_nan() {
        // the motherboard scans: FIELDS Coord._Z x y z _ with a padding field of COUNT 4
        let mut b = b"FIELDS Coord._Z x y z _\nSIZE 4 4 4 4 1\nTYPE F F F F U\nCOUNT 1 1 1 1 4\nWIDTH 2\nHEIGHT 1\nPOINTS 2\nDATA binary\n"
            .to_vec();
        for p in [[9.0_f32, 1.0, 2.0, 3.0], [9.0, f32::NAN, 0.0, 0.0]] {
            for v in p {
                b.extend(v.to_le_bytes());
            }
            b.extend([0_u8; 4]);
        }
        let c = load_cloud(&tmp("mb.pcd", &b)).unwrap();
        assert_eq!(c.points.len(), 1, "the NaN point is dropped");
        assert!(close(c.points[0], [1.0, 2.0, 3.0]));
    }

    #[test]
    fn pcd_pcl_float_rgb() {
        let packed = f32::from_bits(0x00ff_8000);
        let s = format!("FIELDS x y z rgb\nSIZE 4 4 4 4\nTYPE F F F F\nPOINTS 1\nDATA ascii\n1 2 3 {packed:e}\n");
        let c = load_cloud(&tmp("pcl.pcd", s.as_bytes())).unwrap();
        assert!(close(c.colors.unwrap()[0], [1.0, 128.0 / 255.0, 0.0]));
    }

    #[test]
    fn lzf_back_reference() {
        // "abcabcabc": literal "abc", then copy 6 bytes from 3 back (overlapping)
        let packed = [2, b'a', b'b', b'c', 4 << 5, 2]; // ctrl: length 4 (+2), offset high bits 0
        assert_eq!(lzf(&packed, 9).unwrap(), b"abcabcabc");
    }
}

// }}}
