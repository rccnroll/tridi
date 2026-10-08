//! Text clouds: xyz, xyzrgb and pts, one point per line.

use three_d::vec3;

use crate::formats::Cloud;

/// xyz: x y z; xyzrgb: x y z r g b (0..1); pts: an optional count line, then
/// x y z [intensity] [r g b] (0..255). Spaces or commas; other lines skipped.
pub(crate) fn read(raw: &[u8], ext: &str) -> Cloud {
    let mut points = vec![];
    let mut colors = vec![];
    let mut all_colored = true;
    for line in String::from_utf8_lossy(raw).lines() {
        let t: Vec<f32> = match line
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|s| !s.is_empty())
            .map(str::parse)
            .collect()
        {
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
    Cloud {
        points,
        colors,
        faces: None,
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        formats::load_cloud,
        test_support::{close, tmp},
    };

    #[test]
    fn text_formats() {
        let c = load_cloud(&tmp("a.xyz", b"1 2 3\n4,5,6\n")).unwrap();
        assert_eq!(c.points.len(), 2);
        assert!(c.colors.is_none());
        let c = load_cloud(&tmp("a.xyzrgb", b"1 2 3 1 0 0\n")).unwrap();
        assert!(close(c.colors.unwrap()[0], [1.0, 0.0, 0.0]));
        let c = load_cloud(&tmp("a.pts", b"2\n1 2 3 -500 255 0 0\n4 5 6 -500 0 255 0\n")).unwrap();
        assert_eq!(c.points.len(), 2);
        assert!(close(c.colors.unwrap()[1], [0.0, 1.0, 0.0]));
    }
}
