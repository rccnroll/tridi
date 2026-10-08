//! The coloring rules of 1.0: the file's own colors, else one tint per file
//! when several are open, else the theme's height ramp.

use three_d::Vec3;

use crate::{formats::Cloud, theme::Theme};

/// The file's colors, if it has them and they're kept: with several files
/// open (`tint` given) one color for every point (our scans' grey rgb) says
/// nothing, and two of them overlapping would look like one cloud.
pub fn own_colors(c: &Cloud, tint: Option<[f32; 3]>) -> Option<&[Vec3]> {
    c.colors
        .as_deref()
        .filter(|v| tint.is_none() || !(v.len() > 1 && v.windows(2).all(|w| w[0] == w[1])))
}

/// The file's colors if kept (see `own_colors`), else `tint` flat, else the Z ramp.
pub fn colorize(c: &Cloud, tint: Option<[f32; 3]>, theme: &Theme) -> Vec<Vec3> {
    if let Some(cols) = own_colors(c, tint) {
        return cols.to_vec();
    }
    if let Some(t) = tint {
        return vec![Vec3::from(t); c.points.len()];
    }
    let (lo, hi) = c.points.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.z), hi.max(p.z)));
    let span = if hi > lo { hi - lo } else { 1.0 };
    let (a, b) = (Vec3::from(theme.ramp[0]), Vec3::from(theme.ramp[1]));
    c.points.iter().map(|p| a + (b - a) * ((p.z - lo) / span)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        formats::load_cloud,
        test_support::{ascii_pcd, close, tmp},
        theme::LIGHT,
    };
    use std::path::Path;

    // 1. a single file, no colors -> height ramp, bottom to top
    #[test]
    fn ramp_bottom_to_top() {
        let pts: Vec<[f32; 3]> = (0..500).map(|i| [0.3, 0.7, i as f32 / 50.0]).collect();
        let c = load_cloud(&tmp("plain.pcd", &ascii_pcd(&pts))).unwrap();
        assert_eq!(c.points.len(), 500);
        let col = colorize(&c, None, &LIGHT);
        assert!(
            close(col[0], LIGHT.ramp[0]) && close(col[499], LIGHT.ramp[1]),
            "{:?} {:?}",
            col[0],
            col[499]
        );
    }

    // 3. several files -> a different flat tint for each
    #[test]
    fn one_tint_per_file() {
        let mut t = LIGHT.palette.to_vec();
        t.dedup();
        assert_eq!(t.len(), LIGHT.palette.len(), "two files would get the same color");
        let c = load_cloud(&tmp("multi.pcd", &ascii_pcd(&[[0.0, 0.0, 0.0], [1.0, 2.0, 3.0]]))).unwrap();
        assert!(
            colorize(&c, Some(LIGHT.palette[1]), &LIGHT)
                .iter()
                .all(|&v| close(v, LIGHT.palette[1]))
        );
    }

    // 4. colors already present -> untouched, even when a tint is requested
    //    (fixtures written by Open3D: rgb as U4, one ascii and one binary_compressed)
    #[test]
    fn own_colors_untouched() {
        for f in ["rgb_ascii.pcd", "rgb_compressed.pcd"] {
            let c = load_cloud(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(f)).unwrap();
            assert_eq!(c.points.len(), 40, "{f}");
            assert!(
                close(c.points[1], [0.5, -1.0, 0.1]) && close(c.points[39], [19.5, -39.0, 152.1]),
                "{f}"
            );
            let col = colorize(&c, Some(LIGHT.palette[0]), &LIGHT);
            assert!(
                close(col[0], [0.0, 1.0, 0.2]) && close(col[39], [1.0, 0.0, 0.2]),
                "{f}: {:?}",
                col[0]
            );
        }
    }

    // 4b. one color for every point (the scan pipeline's grey rgb): kept
    //     with one file, a tint with several
    #[test]
    fn single_color_tinted_only_with_several_files() {
        let grey = load_cloud(&tmp("grey.xyzrgb", b"0 0 0 0.5 0.5 0.5\n0 0 1 0.5 0.5 0.5\n")).unwrap();
        assert!(colorize(&grey, None, &LIGHT).iter().all(|&v| close(v, [0.5; 3])));
        assert!(
            colorize(&grey, Some(LIGHT.palette[2]), &LIGHT)
                .iter()
                .all(|&v| close(v, LIGHT.palette[2]))
        );
        let two = load_cloud(&tmp("two.xyzrgb", b"0 0 0 0.5 0.5 0.5\n0 0 1 0.1 0.5 0.5\n")).unwrap();
        assert!(close(colorize(&two, Some(LIGHT.palette[2]), &LIGHT)[1], [0.1, 0.5, 0.5]));
    }

    // 5. flat cloud -> no division by zero
    #[test]
    fn flat_cloud() {
        let c = load_cloud(&tmp("flat.pcd", &ascii_pcd(&[[0.0; 3]; 10]))).unwrap();
        assert!(
            colorize(&c, None, &LIGHT)
                .iter()
                .all(|v| v.x.is_finite() && v.y.is_finite() && v.z.is_finite())
        );
    }
}
