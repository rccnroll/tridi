//! Picking a cloud's point with the mouse: which point is under the cursor,
//! where a pick lands on screen, and the line it prints on stdout.

// ========================================== Imports ========================================== {{{

use three_d::{Mat4, Vec3};

// }}}

// ========================================= Constants ========================================= {{{

/// A press and release closer than this (logical px) is a click, not a drag.
pub const CLICK_SLOP: f32 = 4.0;

/// How far from the cursor (logical px) a point can be and still be picked;
/// the viewer widens it to half the drawn point size.
pub const PICK_RADIUS: f32 = 6.0;

// }}}

// =========================================== Pick ============================================ {{{

/// A picked point: its file (place on the command line), its record in
/// that file, and where it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pick {
    pub file: usize,
    pub index: usize,
    pub at: Vec3,
}

/// What a pick or a removal prints: `picked #48213 1.234 -0.45 2.1 path`,
/// the path last so spaces in it don't split it.
pub fn line(event: &str, p: &Pick, path: &str) -> String {
    format!("{event} #{} {} {} {} {path}", p.index, p.at.x, p.at.y, p.at.z)
}

/// `at` in normalized device coordinates, None if behind the eye or out of view.
fn ndc(at: Vec3, mvp: Mat4) -> Option<Vec3> {
    let c = mvp * at.extend(1.0);
    if c.w <= 0.0 {
        return None;
    }
    let n = c.truncate() / c.w;
    (n.x.abs() <= 1.0 && n.y.abs() <= 1.0 && n.z.abs() <= 1.0).then_some(n)
}

/// From normalized device coordinates to logical px, origin top left.
fn to_px(n: Vec3, size: (f32, f32)) -> (f32, f32) {
    (f32::midpoint(n.x, 1.0) * size.0, f32::midpoint(1.0, -n.y) * size.1)
}

/// Where `at` is on screen (logical px, origin top left), None if it isn't.
pub fn screen(at: Vec3, mvp: Mat4, size: (f32, f32)) -> Option<(f32, f32)> {
    ndc(at, mvp).map(|n| to_px(n, size))
}

/// The point under `cursor`: among those within `radius` of it on screen,
/// the one closest to the eye. Returns its place in `points` and its depth
/// (to compare with other files' picks; smaller is closer).
pub fn nearest(points: &[Vec3], mvp: Mat4, size: (f32, f32), cursor: (f32, f32), radius: f32) -> Option<(usize, f32)> {
    let r2 = radius * radius;
    points
        .iter()
        .enumerate()
        .filter_map(|(i, p)| {
            let n = ndc(*p, mvp)?;
            let (x, y) = to_px(n, size);
            let (dx, dy) = (x - cursor.0, y - cursor.1);
            (dx * dx + dy * dy <= r2).then_some((i, n.z))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

// }}}

// =========================================== Tests =========================================== {{{

#[cfg(test)]
mod tests {
    use super::*;
    use three_d::{Camera, Viewport, Zero, degrees, vec3};

    const SIZE: (f32, f32) = (200.0, 100.0);
    const CENTER: (f32, f32) = (100.0, 50.0);

    /// Looking down -Z at the origin from z = 10.
    fn mvp() -> Mat4 {
        let cam = Camera::new_perspective(
            Viewport::new_at_origo(200, 100),
            vec3(0.0, 0.0, 10.0),
            Vec3::zero(),
            Vec3::unit_y(),
            degrees(tridi::FOV),
            0.1,
            100.0,
        );
        cam.projection() * cam.view()
    }

    #[test]
    fn picks_the_point_under_the_cursor() {
        let pts = [vec3(0.0, 0.0, 0.0), vec3(5.0, 0.0, 0.0)];
        assert_eq!(nearest(&pts, mvp(), SIZE, CENTER, PICK_RADIUS).map(|(i, _)| i), Some(0));
    }

    #[test]
    fn front_most_wins_within_the_radius() {
        // both on the line of sight; the second is closer to the eye
        let pts = [vec3(0.0, 0.0, -1.0), vec3(0.0, 0.0, 1.0)];
        assert_eq!(nearest(&pts, mvp(), SIZE, CENTER, PICK_RADIUS).map(|(i, _)| i), Some(1));
    }

    #[test]
    fn nothing_in_range_picks_nothing() {
        assert!(nearest(&[vec3(0.0, 0.0, 0.0)], mvp(), SIZE, (10.0, 10.0), PICK_RADIUS).is_none());
    }

    #[test]
    fn behind_the_camera_is_never_picked() {
        assert!(nearest(&[vec3(0.0, 0.0, 100.0)], mvp(), SIZE, CENTER, PICK_RADIUS).is_none());
    }

    #[test]
    fn screen_is_top_left_logical_pixels() {
        let (x, y) = screen(vec3(0.0, 1.0, 0.0), mvp(), SIZE).unwrap();
        assert!((x - 100.0).abs() < 1e-3 && y < 50.0, "up in the world is up on screen: {x} {y}");
        assert!(screen(vec3(0.0, 0.0, 100.0), mvp(), SIZE).is_none());
    }

    #[test]
    fn the_line_ends_with_the_whole_path() {
        let p = Pick {
            file: 0,
            index: 48213,
            at: vec3(1.234, -0.45, 2.1),
        };
        assert_eq!(
            line("picked", &p, "scans/scan 01.pcd"),
            "picked #48213 1.234 -0.45 2.1 scans/scan 01.pcd"
        );
    }
}

// }}}
