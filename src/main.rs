//! pcdview - minimal viewer for point clouds (pcd, ply, xyz, xyzrgb, pts) and
//! meshes (glb, gltf, obj, stl, off, ply with faces).
//!
//!     pcdview [--theme T] FILE [FILE ...]        window
//!     pcdview thumb [--theme T] IN OUT SIZE      PNG thumbnail, no window (for Nautilus)
//!     pcdview theme [light|dark]                 show or switch the theme (viewer and thumbnails)
//!     pcdview clear-thumbnails [DIR ...]         drop cached thumbnails of our formats
//!
//! T is `light` or `dark`, both Nord. The viewer uses the one `pcdview theme`
//! saved (light if none); thumbnails get theirs from the thumbnailer entry.
//!
//! In the window: left drag orbits, right or middle drag (or shift + left)
//! pans, the wheel zooms; R resets the view, + and - change the point size,
//! 1-9 turn the N-th file off and on, Q or Esc quits.
//!
//! With several files each cloud gets its own color, to tell them apart at a
//! glance; with a single file the cloud is colored by Z height. In both cases
//! only if the cloud doesn't already carry its own colors. Meshes keep their
//! materials.

mod cloud;
mod mesh;
mod render;
mod theme;

use mesh::Item;
use render::Input;
use std::path::Path;
use std::time::Instant;
use three_d::*;

/// The Wayland app_id the dock groups the window by, matching the .desktop.
const APP_ID: &str = "pcdview";
const USAGE: &str = "usage: pcdview [--theme light|dark] FILE [FILE ...]
       pcdview thumb [--theme light|dark] IN OUT SIZE
       pcdview theme [light|dark]
       pcdview clear-thumbnails [DIR ...]";

fn is_gltf(p: &str) -> bool {
    let ext = Path::new(p).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    ext == "glb" || ext == "gltf"
}

fn extent(pts: impl Iterator<Item = Vec3>) -> Vec3 {
    let mut bb = AxisAlignedBoundingBox::EMPTY;
    bb.expand(&pts.collect::<Vec<_>>());
    bb.max() - bb.min()
}

/// Loads every file; one that fails is reported and skipped. None if nothing
/// could be read. Returns the scene inputs, the names of the files read (in
/// the same order) and the up axis: +Y when every file is glTF, +Z otherwise.
fn load_all(paths: &[String], theme: &theme::Theme) -> Option<(Vec<Input>, Vec<String>, Vec3)> {
    let (mut inputs, mut names, mut all_gltf) = (vec![], vec![], true);
    for (i, p) in paths.iter().enumerate() {
        let path = Path::new(p);
        let name = path.file_name().map_or(p.clone(), |n| n.to_string_lossy().into_owned());
        let item = match mesh::load(path, theme.mesh) {
            Ok(it) => it,
            Err(e) => {
                eprintln!("[pcdview] {name}: {e}");
                continue;
            }
        };
        all_gltf &= is_gltf(p);
        names.push(name.clone());
        match item {
            Item::Cloud(c) => {
                let tint = (paths.len() > 1).then(|| theme.palette[i % theme.palette.len()]);
                let e = extent(c.points.iter().copied());
                let tag = match tint {
                    Some(t) if c.colors.is_none() => format!("  color [{:.2}, {:.2}, {:.2}]", t[0], t[1], t[2]),
                    _ => String::new(),
                };
                println!("{name}: {} points, extent [{:.3}, {:.3}, {:.3}]{tag}", c.points.len(), e.x, e.y, e.z);
                let colors = cloud::colorize(&c, tint, theme);
                inputs.push(Input::Points(c.points, colors));
            }
            Item::Mesh(m) => {
                // ponytail: no per-file tint on meshes (1.0 didn't have one either)
                let e = extent(m.geometries.iter().flat_map(|g| match &g.geometry {
                    three_d_asset::Geometry::Triangles(t) => {
                        t.positions.to_f32().into_iter().map(|v| (g.transformation * v.extend(1.0)).truncate()).collect()
                    }
                    _ => vec![],
                }));
                println!("{name}: {} triangles, extent [{:.3}, {:.3}, {:.3}]", mesh::triangles(&m), e.x, e.y, e.z);
                inputs.push(Input::Mesh(m));
            }
        }
    }
    (!inputs.is_empty()).then(|| (inputs, names, up_for(all_gltf)))
}

/// glTF is Y-up by its standard (decided 25/09: we follow it, even though
/// Open3D writes scan GLBs Z-up); scans and CAD are Z-up.
fn up_for(gltf: bool) -> Vec3 {
    if gltf { Vec3::unit_y() } else { Vec3::unit_z() }
}

/// Mouse navigation around the camera's target: left drag orbits, right or
/// middle drag (or shift + left) pans, the wheel zooms. three-d's
/// OrbitControl has no pan and a fixed target, so it's done here.
fn navigate(cam: &mut Camera, events: &mut [Event], dpr: f32, min: f32, max: f32) {
    for e in events.iter_mut() {
        match e {
            Event::MouseMotion { button: Some(b), delta, modifiers, handled, .. } if !*handled => {
                let pan = matches!(b, MouseButton::Right | MouseButton::Middle) || modifiers.shift;
                if pan {
                    // world units per logical pixel at the target's depth
                    let dist = cam.position().distance(cam.target());
                    let k = 2.0 * dist * (render::FOV.to_radians() / 2.0).tan() * dpr / cam.viewport().height as f32;
                    let right = cam.right_direction();
                    let up = right.cross(cam.view_direction()).normalize();
                    cam.translate((up * delta.1 - right * delta.0) * k);
                } else if *b == MouseButton::Left {
                    let t = cam.target();
                    cam.rotate_around_with_fixed_up(t, 0.008 * delta.0, 0.008 * delta.1);
                }
                *handled = true;
            }
            Event::MouseWheel { delta, handled, .. } if !*handled => {
                let (t, dist) = (cam.target(), cam.position().distance(cam.target()));
                cam.zoom_towards(t, dist * (1.0 - (-delta.1 * 0.01).exp()), min, max);
                *handled = true;
            }
            _ => {}
        }
    }
}

fn view(paths: &[String], theme: &'static theme::Theme) -> Result<(), String> {
    let (inputs, names, up) = load_all(paths, theme).ok_or("")?;
    if names.len() > 1 {
        let keys: Vec<String> = names.iter().enumerate().take(9).map(|(i, n)| format!("{} {n}", i + 1)).collect();
        println!("keys: {}", keys.join(", "));
    }
    // winit 0.28 panics instead of returning an error when there's no display
    if std::env::var_os("WAYLAND_DISPLAY").is_none() && std::env::var_os("DISPLAY").is_none() {
        return Err("no display: neither WAYLAND_DISPLAY nor DISPLAY is set".into());
    }
    // our own winit window: three-d doesn't set the Wayland app_id, and
    // without it the dock doesn't group the window with the .desktop
    use winit::platform::wayland::WindowBuilderExtWayland;
    let event_loop = winit::event_loop::EventLoop::new();
    let ww = winit::window::WindowBuilder::new()
        // the title is set once: three-d owns the window afterwards
        .with_title(format!("{APP_ID} — {}", names.join(", ")))
        .with_name(APP_ID, APP_ID)
        .with_maximized(true)
        .build(&event_loop)
        .map_err(|e| e.to_string())?;
    let window = render::quiet_stderr(|| Window::from_winit_window(ww, event_loop, SurfaceSettings::default(), true))
        .map_err(|e| e.to_string())?;
    let mut scene = render::Scene::new(&window.gl(), inputs, true, up)?;
    if std::env::var_os("PCDVIEW_DEBUG").is_some() {
        eprintln!("GL: {}", unsafe { window.gl().get_parameter_string(context::RENDERER) });
    }
    let mut cam = scene.camera(window.viewport());
    // min above the near plane (radius * 0.01), or zooming in clips everything
    let (min, max) = (scene.radius * 0.05, scene.radius * 15.0);
    let mut psize = 2.0;
    let bg = theme.bg;
    window.render_loop(move |mut fi| {
        cam.set_viewport(fi.viewport);
        let mut exit = false;
        for e in &fi.events {
            let Event::KeyPress { kind, .. } = e else { continue };
            match kind {
                Key::R => cam = scene.camera(fi.viewport),
                Key::Plus | Key::Equals => psize = f32::min(psize * 1.25, 20.0),
                Key::Minus => psize = f32::max(psize / 1.25, 1.0),
                Key::Q | Key::Escape => exit = true,
                k => {
                    let digits = [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9];
                    if let Some(i) = digits.iter().position(|d| d == k)
                        && let Some(on) = scene.toggle(i)
                    {
                        println!("{}: {}", names[i], if on { "on" } else { "off" });
                    }
                }
            }
        }
        navigate(&mut cam, &mut fi.events, fi.device_pixel_ratio, min, max);
        let screen = fi.screen();
        screen.clear(ClearState::color_and_depth(bg[0], bg[1], bg[2], 1.0, 1.0));
        scene.render(&screen, &cam, psize * fi.device_pixel_ratio);
        FrameOutput { exit, ..Default::default() }
    });
    Ok(())
}

fn thumb(inp: &str, out: &str, size: &str, theme: &theme::Theme) -> Result<(), String> {
    let t0 = Instant::now();
    let size: u32 = size.parse().ok().filter(|s| (1..=4096).contains(s)).ok_or("SIZE must be 1..4096")?;
    // Nautilus's sandbox clears the environment: without this glvnd would
    // also load NVIDIA's EGL, even for a user who limited it to Mesa
    let mesa = "/usr/share/glvnd/egl_vendor.d/50_mesa.json";
    if std::env::var_os("__EGL_VENDOR_LIBRARY_FILENAMES").is_none() && Path::new(mesa).exists() {
        // SAFETY: single-threaded, and before EGL is loaded
        unsafe { std::env::set_var("__EGL_VENDOR_LIBRARY_FILENAMES", mesa) };
    }
    let input = match mesh::load(Path::new(inp), theme.mesh)? {
        Item::Cloud(c) => {
            let colors = cloud::colorize(&c, None, theme);
            Input::Points(c.points, colors)
        }
        Item::Mesh(m) => Input::Mesh(m),
    };
    let (ctx, _keep) = render::headless()?;
    // no axis triad: at thumbnail size it's only noise
    let scene = render::Scene::new(&ctx, vec![input], false, up_for(is_gltf(inp)))?;
    let img = render::offscreen(&ctx, &scene, size, theme.bg)?;
    image::save_buffer(out, &img, size, size, image::ExtendedColorType::Rgba8).map_err(|e| e.to_string())?;
    if std::env::var_os("PCDVIEW_TIMING").is_some() {
        eprintln!("thumb {:?}", t0.elapsed());
    }
    Ok(())
}

/// Takes `--theme NAME` out of the arguments, if it's there.
fn take_theme(a: &mut Vec<String>) -> Result<Option<&'static theme::Theme>, String> {
    let Some(i) = a.iter().position(|s| s == "--theme") else { return Ok(None) };
    let name = a.get(i + 1).cloned().ok_or(USAGE)?;
    a.drain(i..i + 2);
    theme::by_name(&name).map(Some)
}

fn main() {
    let mut a: Vec<String> = std::env::args().skip(1).collect();
    let r = take_theme(&mut a).and_then(|t| match a.as_slice() {
        // thumbnails can't read the saved theme (sandbox): light unless told
        [cmd, i, o, s] if cmd == "thumb" => thumb(i, o, s, t.unwrap_or(&theme::LIGHT)),
        [cmd, ..] if cmd == "thumb" => Err(USAGE.into()),
        [cmd] if cmd == "theme" => theme::command(None),
        [cmd, name] if cmd == "theme" => theme::command(Some(name)),
        [cmd, dirs @ ..] if cmd == "clear-thumbnails" => {
            let home = std::env::var_os("HOME").map(|h| Path::new(&h).join(".cache/thumbnails"));
            let dirs: Vec<std::path::PathBuf> = if dirs.is_empty() { home.into_iter().collect() } else { dirs.iter().map(Into::into).collect() };
            let n: usize = dirs.iter().map(|d| theme::clear_thumbnails(d)).sum();
            println!("{n} cached thumbnails cleared");
            Ok(())
        }
        [] => Err(USAGE.into()),
        files => {
            theme::refresh();
            view(files, t.unwrap_or_else(theme::current))
        }
    });
    if let Err(e) = r {
        if !e.is_empty() {
            eprintln!("[pcdview] {e}");
        }
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn motion(button: MouseButton, delta: (f32, f32)) -> Event {
        Event::MouseMotion { button: Some(button), delta, position: PhysicalPoint { x: 0.0, y: 0.0 }, modifiers: Modifiers::default(), handled: false }
    }

    #[test]
    fn pan_orbit_zoom() {
        let vp = Viewport::new_at_origo(800, 600);
        let mut cam = Camera::new_perspective(vp, vec3(0.0, -10.0, 0.0), Vec3::zero(), Vec3::unit_z(), degrees(render::FOV), 0.1, 100.0);
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
            let wheel = Event::MouseWheel { delta: (0.0, 500.0), position: PhysicalPoint { x: 0.0, y: 0.0 }, modifiers: Modifiers::default(), handled: false };
            navigate(&mut cam, &mut [wheel], 1.0, 1.0, 50.0);
        }
        assert!(cam.position().distance(cam.target()) >= 1.0 - 1e-3);
    }

    // 7. the dock's app_id is a constant: if it follows the title or the file
    //    name, the dock goes back to "unknown"
    #[test]
    fn app_id_is_fixed() {
        assert_eq!(super::APP_ID, "pcdview");
        assert!(include_str!("main.rs").contains(".with_name(APP_ID, APP_ID)"));
    }
}
