//! OFF and COFF meshes, read as a cloud with faces.

// ========================================== Imports ========================================== {{{

use three_d::vec3;

use crate::formats::{Cloud, FormatError, FormatResult};

// }}}

// ============================================ OFF ============================================ {{{

/// OFF / COFF: counts, then vertices (COFF adds r g b a, 0..255 or 0..1),
/// then faces as `k i0 i1 ...`.
pub(crate) fn read(raw: &[u8]) -> FormatResult<Cloud> {
    parse(raw).map_err(|reason| FormatError::Invalid { format: "OFF", reason })
}

/// The reader proper; the error is the reason the file is invalid.
#[expect(
    clippy::many_single_char_names,
    reason = "x y z, r g b, and the format's n vertices, m faces, k sides"
)]
fn parse(raw: &[u8]) -> Result<Cloud, String> {
    let text = String::from_utf8_lossy(raw);
    let mut lines = text
        .lines()
        .map(|l| l.split('#').next().unwrap_or("").trim())
        .filter(|l| !l.is_empty());
    let first = lines.next().ok_or("empty OFF")?;
    let kw = first.split_whitespace().next().unwrap_or("");
    if !kw.ends_with("OFF") {
        return Err("not an OFF file".into());
    }
    let colored = kw.starts_with('C');
    // the counts can be on the OFF line itself
    let rest = first.strip_prefix(kw).unwrap_or_default().trim().to_owned();
    let counts = if rest.is_empty() {
        lines.next().ok_or("OFF without counts")?.to_owned()
    } else {
        rest
    };
    let nums: Vec<usize> = counts
        .split_whitespace()
        .map(|s| s.parse().map_err(|_err| "bad OFF counts"))
        .collect::<Result<_, _>>()?;
    let [n, m, ..] = *nums.as_slice() else {
        return Err("bad OFF counts".into());
    };
    // capped by the file's size, as in pcd.rs
    let cap = n.min(raw.len());
    let mut points = Vec::with_capacity(cap);
    let mut colors = colored.then(|| Vec::with_capacity(cap));
    for line in lines.by_ref().take(n) {
        let t: Vec<f32> = line.split_whitespace().map(|s| s.parse().unwrap_or(f32::NAN)).collect();
        let [x, y, z, ref rgb @ ..] = *t.as_slice() else {
            return Err("OFF vertex line too short".into());
        };
        points.push(vec3(x, y, z));
        if let Some(cols) = &mut colors {
            let [r, g, b, ..] = *rgb else {
                return Err("COFF vertex without color".into());
            };
            let k = if [r, g, b].iter().any(|&v| v > 1.0) { 255.0 } else { 1.0 };
            cols.push(vec3(r, g, b) / k);
        }
    }
    if points.len() < n {
        return Err("truncated OFF".into());
    }
    let mut faces = Vec::with_capacity(m.min(raw.len()) * 3);
    for line in lines.take(m) {
        let t: Vec<u32> = line.split_whitespace().map_while(|s| s.parse().ok()).collect();
        let (&k, rest) = t.split_first().ok_or("bad OFF face line")?;
        let k = usize::try_from(k).map_err(|_err| "bad OFF face line")?;
        let idx = rest.get(..k).ok_or("OFF face line too short")?;
        if idx.iter().any(|&i| usize::try_from(i).map_or(true, |i| i >= n)) {
            return Err("OFF face index out of range".into());
        }
        // fan: polygons become triangles
        if let Some((&first, others)) = idx.split_first() {
            for w in others.windows(2) {
                if let [a, b] = *w {
                    faces.extend([first, a, b]);
                }
            }
        }
    }
    Ok(Cloud {
        points,
        colors,
        faces: Some(faces),
    })
}

// }}}
