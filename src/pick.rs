//! Picking a cloud's point with the mouse: which point is under the cursor,
//! where a pick lands on screen, the line it prints on stdout, and what the
//! picks panel shows and copies.

// ========================================== Imports ========================================== {{{

use std::ops::Range;
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

/// What the clipboard gets: each pick's stdout line, in pick order, `paths`
/// the files as given on the command line.
pub fn copy(picks: &[Pick], paths: &[String]) -> String {
    picks
        .iter()
        .map(|p| line("picked", p, paths.get(p.file).map_or("", String::as_str)) + "\n")
        .collect()
}

/// A pick as the picks panel shows it, numbered `n` like its marker: the
/// number, the index in the file, x, y and z, a column each.
pub fn row(n: usize, p: &Pick) -> [String; 5] {
    [
        n.to_string(),
        format!("#{}", p.index),
        p.at.x.to_string(),
        p.at.y.to_string(),
        p.at.z.to_string(),
    ]
}

/// The picks the panel shows out of `len`, at most `max` from `top`, or the
/// last `max` when `top` is None (it follows the newest).
pub fn shown(len: usize, top: Option<usize>, max: usize) -> Range<usize> {
    let last = len.saturating_sub(max);
    let start = top.map_or(last, |t| t.min(last));
    start..len.min(start + max)
}

/// `top` moved a row `up` (to older picks) or down; None once it's back at
/// the end, so the next pick shows.
pub fn scroll(len: usize, top: Option<usize>, max: usize, up: bool) -> Option<usize> {
    let last = len.saturating_sub(max);
    let start = shown(len, top, max).start;
    let start = if up { start.saturating_sub(1) } else { (start + 1).min(last) };
    (start < last).then_some(start)
}

/// `name` cut to `max` characters by taking out its middle, where a file's
/// name is least telling (its start and its extension stay).
pub fn short_name(name: &str, max: usize) -> String {
    let n = name.chars().count();
    if n <= max {
        return name.to_owned();
    }
    let tail = (max - 1).div_euclid(3);
    let head = max - 1 - tail;
    let start: String = name.chars().take(head).collect();
    let end: String = name.chars().skip(n - tail).collect();
    format!("{start}…{end}")
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

/// The point under `cursor` among several files' clouds, each with its
/// pick radius: the closest to the eye wins. Returns the file and the
/// point's place in its cloud.
pub fn under<'a, I: IntoIterator<Item = (usize, &'a [Vec3], f32)>>(
    clouds: I,
    mvp: Mat4,
    size: (f32, f32),
    cursor: (f32, f32),
) -> Option<(usize, usize)> {
    clouds
        .into_iter()
        .filter_map(|(file, points, radius)| nearest(points, mvp, size, cursor, radius).map(|(i, z)| (file, i, z)))
        .min_by(|a, b| a.2.total_cmp(&b.2))
        .map(|(file, i, _)| (file, i))
}

/// A press at `from` released at `to` (logical px) is a click, not a drag.
pub fn is_click(from: (f32, f32), to: (f32, f32)) -> bool {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    dx * dx + dy * dy < CLICK_SLOP * CLICK_SLOP
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

    #[test]
    fn across_files_the_front_most_wins() {
        let back = [vec3(0.0, 0.0, -1.0)];
        let front = [vec3(5.0, 0.0, 0.0), vec3(0.0, 0.0, 1.0)];
        let clouds = [(0, &back[..], PICK_RADIUS), (3, &front[..], PICK_RADIUS)];
        assert_eq!(under(clouds, mvp(), SIZE, CENTER), Some((3, 1)));
    }

    #[test]
    fn a_click_moves_less_than_the_slop() {
        assert!(is_click((10.0, 10.0), (12.0, 13.0)));
        assert!(!is_click((10.0, 10.0), (10.0, 20.0)));
    }

    #[test]
    fn copy_is_the_stdout_lines_in_pick_order() {
        let a = Pick {
            file: 1,
            index: 7,
            at: vec3(1.0, 2.0, 3.0),
        };
        let b = Pick {
            file: 0,
            index: 0,
            at: vec3(0.5, 0.0, -1.0),
        };
        let paths = ["a.pcd".to_owned(), "dir/b c.pcd".to_owned()];
        assert_eq!(copy(&[a, b], &paths), "picked #7 1 2 3 dir/b c.pcd\npicked #0 0.5 0 -1 a.pcd\n");
        assert_eq!(copy(&[], &paths), "");
    }

    #[test]
    fn a_row_shows_number_index_coordinates_and_name() {
        let p = Pick {
            file: 0,
            index: 52313,
            at: vec3(1024.087, 3681.835, 1257.451),
        };
        assert_eq!(row(2, &p), ["2", "#52313", "1024.087", "3681.835", "1257.451"]);
    }

    #[test]
    fn the_panel_shows_the_newest_until_scrolled() {
        assert_eq!(shown(27, None, 8), 19..27);
        assert_eq!(shown(27, Some(3), 8), 3..11);
        // a top past the end is the end
        assert_eq!(shown(27, Some(25), 8), 19..27);
        assert_eq!(shown(5, None, 8), 0..5);
    }

    #[test]
    fn scrolling_up_leaves_the_end_and_down_comes_back() {
        assert_eq!(scroll(27, None, 8, true), Some(18));
        assert_eq!(scroll(27, Some(18), 8, false), None);
        assert_eq!(scroll(27, Some(0), 8, true), Some(0));
        assert_eq!(scroll(5, None, 8, true), None);
    }

    #[test]
    fn long_names_lose_their_middle() {
        assert_eq!(short_name("scan.pcd", 24), "scan.pcd");
        let long = "Cappellotto_eye_in_hand_robot_World_mm_2026-11-04T16-27-17-126.pcd";
        let s = short_name(long, 24);
        assert_eq!(s.chars().count(), 24);
        assert!(s.starts_with("Cappellotto") && s.ends_with("126.pcd") && s.contains('…'), "{s}");
    }
}

// }}}
