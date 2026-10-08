//! OFF and COFF meshes, read as a cloud with faces.

use three_d::vec3;

use crate::formats::{Cloud, FormatError, FormatResult};

/// OFF / COFF: counts, then vertices (COFF adds r g b a, 0..255 or 0..1),
/// then faces as `k i0 i1 ...`.
pub(crate) fn read(raw: &[u8]) -> FormatResult<Cloud> {
    parse(raw).map_err(|reason| FormatError::Invalid { format: "OFF", reason })
}

/// The reader proper; the error is the reason the file is invalid.
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
    let rest: String = first[kw.len()..].trim().to_owned();
    let counts = if rest.is_empty() {
        lines.next().ok_or("OFF without counts")?.to_owned()
    } else {
        rest
    };
    let c: Vec<usize> = counts
        .split_whitespace()
        .map(|s| s.parse().map_err(|_| "bad OFF counts"))
        .collect::<Result<_, _>>()?;
    let (n, m) = (*c.first().ok_or("bad OFF counts")?, *c.get(1).ok_or("bad OFF counts")?);
    // capped by the file's size, as in read_pcd
    let cap = n.min(raw.len());
    let mut points = Vec::with_capacity(cap);
    let mut colors = colored.then(|| Vec::with_capacity(cap));
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
    let mut faces = Vec::with_capacity(m.min(raw.len()) * 3);
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
    Ok(Cloud {
        points,
        colors,
        faces: Some(faces),
    })
}
