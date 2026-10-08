//! PCD: ascii, binary and binary_compressed (LZF), as PCL and Open3D write it.

use three_d::vec3;

use crate::formats::{Cloud, Field, scalar, split_header, unpack_rgb, valid_type};

pub(crate) fn read(raw: &[u8]) -> Result<Cloud, String> {
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
        let count: usize = counts[i].parse().ok().filter(|&c| c > 0).ok_or("bad COUNT")?;
        let ty = types[i].as_bytes().first().copied().unwrap_or(0);
        if !valid_type(ty, size) {
            return Err(format!("unsupported TYPE {} SIZE {size}", types[i]));
        }
        fields.push(Field {
            name: names[i].clone(),
            ty,
            size,
            count,
            off,
            col,
        });
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
                let o = if mode == "binary" {
                    i * stride + f.off
                } else {
                    n * f.off + i * f.size * f.count
                };
                &data[o..o + f.size]
            };
            for i in 0..n {
                let v = |f: &Field| scalar(f.ty, at(i, f), false) as f32;
                points.push(vec3(v(fx), v(fy), v(fz)));
                if let (Some(f), Some(cols)) = (frgb, &mut colors) {
                    let b = at(i, f);
                    let bits = if f.size == 4 {
                        u32::from_le_bytes(b.try_into().unwrap())
                    } else {
                        scalar(f.ty, b, false) as u32
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
        for p in [[9.0f32, 1.0, 2.0, 3.0], [9.0, f32::NAN, 0.0, 0.0]] {
            p.iter().for_each(|v| b.extend(v.to_le_bytes()));
            b.extend([0u8; 4]);
        }
        let c = load_cloud(&tmp("mb.pcd", &b)).unwrap();
        assert_eq!(c.points.len(), 1, "the NaN point is dropped");
        assert!(close(c.points[0], [1.0, 2.0, 3.0]));
    }

    #[test]
    fn pcd_pcl_float_rgb() {
        let packed = f32::from_bits(0x00ff8000);
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
