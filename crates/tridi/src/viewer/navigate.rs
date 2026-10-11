//! Mouse navigation: orbit, pan and zoom around the camera's target.

// ========================================== Imports ========================================== {{{

use three_d::{Camera, Event, InnerSpace, MetricSpace, MouseButton};

use crate::{FOV, px_f32};

// }}}

// ========================================= Constants ========================================= {{{

/// Radians of orbit per logical pixel dragged.
const ORBIT_SPEED: f32 = 0.008;

/// Wheel zoom: the share of the distance one unit of wheel moves.
const ZOOM_SPEED: f32 = 0.01;

/// Wheel units per notch, and per logical pixel of a touchpad: the scale
/// three-d's own window gave.
pub(crate) const WHEEL_NOTCH: f32 = 24.0;

pub(crate) const WHEEL_PIXEL: f32 = 0.24;

/// Zoom bounds, in scene radii: the closest stays above the near plane
/// (radius * 0.01), or zooming in clips everything.
pub(crate) const ZOOM_MIN: f32 = 0.05;

pub(crate) const ZOOM_MAX: f32 = 15.0;

// }}}

// ======================================== Navigation ========================================= {{{

/// Mouse navigation around the camera's target: left drag orbits, right or
/// middle drag (or shift + left) pans, the wheel zooms. three-d's
/// `OrbitControl` has no pan and a fixed target, so it's done here.
pub(crate) fn navigate(cam: &mut Camera, events: &mut [Event], dpr: f32, min: f32, max: f32) {
    for e in events.iter_mut() {
        #[expect(clippy::wildcard_enum_match_arm, reason = "only the mouse moves the view")]
        match e {
            Event::MouseMotion {
                button: Some(b),
                delta,
                modifiers,
                handled,
                ..
            } if !*handled => {
                let pan = matches!(b, MouseButton::Right | MouseButton::Middle) || modifiers.shift;
                if pan {
                    // world units per logical pixel at the target's depth
                    let dist = cam.position().distance(cam.target());
                    let k = 2.0 * dist * (FOV.to_radians() / 2.0).tan() * dpr / px_f32(cam.viewport().height);
                    let right = cam.right_direction();
                    let up = right.cross(cam.view_direction()).normalize();
                    cam.translate((up * delta.1 - right * delta.0) * k);
                } else if *b == MouseButton::Left {
                    let t = cam.target();
                    cam.rotate_around_with_fixed_up(t, ORBIT_SPEED * delta.0, ORBIT_SPEED * delta.1);
                }
                *handled = true;
            }
            Event::MouseWheel { delta, handled, .. } if !*handled => {
                let (t, dist) = (cam.target(), cam.position().distance(cam.target()));
                cam.zoom_towards(t, dist * (1.0 - (-delta.1 * ZOOM_SPEED).exp()), min, max);
                *handled = true;
            }
            _ => {}
        }
    }
}

// }}}

// =========================================== Tests =========================================== {{{

#[cfg(test)]
mod tests {
    use super::*;
    use three_d::{Modifiers, PhysicalPoint, Vec3, Viewport, Zero, degrees, vec3};

    fn motion(button: MouseButton, delta: (f32, f32)) -> Event {
        Event::MouseMotion {
            button: Some(button),
            delta,
            position: PhysicalPoint { x: 0.0, y: 0.0 },
            modifiers: Modifiers::default(),
            handled: false,
        }
    }

    #[test]
    fn pan_orbit_zoom() {
        let vp = Viewport::new_at_origo(800, 600);
        let mut cam = Camera::new_perspective(vp, vec3(0.0, -10.0, 0.0), Vec3::zero(), Vec3::unit_z(), degrees(FOV), 0.1, 100.0);
        // pan: target and camera move together, the distance stays
        navigate(&mut cam, &mut [motion(MouseButton::Right, (100.0, 0.0))], 1.0, 1.0, 50.0);
        assert!(cam.target().x < -0.5, "right drag moves the view: target {:?}", cam.target());
        assert!((cam.position().distance(cam.target()) - 10.0).abs() < 1e-3);
        // orbit: around the (new) target, same distance
        let t = cam.target();
        navigate(&mut cam, &mut [motion(MouseButton::Left, (50.0, 20.0))], 1.0, 1.0, 50.0);
        assert!((cam.target() - t).magnitude() < 1e-4 && (cam.position().distance(t) - 10.0).abs() < 1e-3);
        // zoom: never closer than min
        for _ in 0..50 {
            let wheel = Event::MouseWheel {
                delta: (0.0, 500.0),
                position: PhysicalPoint { x: 0.0, y: 0.0 },
                modifiers: Modifiers::default(),
                handled: false,
            };
            navigate(&mut cam, &mut [wheel], 1.0, 1.0, 50.0);
        }
        assert!(cam.position().distance(cam.target()) >= 1.0 - 1e-3);
    }
}

// }}}
