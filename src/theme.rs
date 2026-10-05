//! Themes (Nord light and Nord dark) and `tridi theme`, which switches the
//! viewer and the thumbnails together.
//!
//! The thumbnailer runs in Nautilus's sandbox, which sees neither `$HOME` nor
//! the session bus and clears the environment: the only way in is the `Exec=`
//! line of a `.thumbnailer` file, read outside the sandbox. So the theme is a
//! copy of the package's entry in `~/.local/share/thumbnailers/`, with
//! `--theme dark` for the dark one. The copy is written for light too: next
//! to f3d's entries in /usr/share, which also claim glb, stl and obj, which
//! one wins is down to directory order; the user's directory comes first.

use std::path::{Path, PathBuf};

/// The colors of a look: background, height ramp, one tint per file when
/// several are open, and the color of meshes without a material.
pub struct Theme {
    pub name: &'static str,
    pub bg: [f32; 3],
    pub ramp: [[f32; 3]; 2],
    pub palette: [[f32; 3]; 6],
    pub mesh: [f32; 3],
}

/// The default: Nord snow storm background, everything else dark.
pub const LIGHT: Theme = Theme {
    name: "light",
    bg: [0.925, 0.937, 0.957],                      // nord6
    ramp: [[0.14, 0.16, 0.21], [0.37, 0.51, 0.67]], // nord0 -> nord9
    // Nord aurora, darkened just enough to read on a light background
    palette: [
        [0.63, 0.25, 0.28], // red
        [0.25, 0.40, 0.58], // blue
        [0.35, 0.52, 0.31], // green
        [0.55, 0.36, 0.52], // purple
        [0.72, 0.42, 0.26], // orange
        [0.20, 0.51, 0.55], // teal
    ],
    mesh: [0.25, 0.30, 0.38], // 1.0's CAD_GREY
};

/// Nord polar night background (1.0's thumbnails), everything else light.
pub const DARK: Theme = Theme {
    name: "dark",
    bg: [0.18, 0.20, 0.25],                         // nord0
    ramp: [[0.37, 0.51, 0.67], [0.53, 0.75, 0.82]], // nord10 -> nord8
    palette: [
        [0.75, 0.38, 0.42], // nord11 red
        [0.51, 0.63, 0.76], // nord9 blue
        [0.64, 0.75, 0.55], // nord14 green
        [0.71, 0.56, 0.68], // nord15 purple
        [0.82, 0.53, 0.44], // nord12 orange
        [0.56, 0.74, 0.73], // nord7 teal
    ],
    mesh: [0.85, 0.87, 0.91], // nord4
};

pub const THEMES: [&Theme; 2] = [&LIGHT, &DARK];

pub fn by_name(name: &str) -> Result<&'static Theme, String> {
    THEMES.into_iter().find(|t| t.name == name).ok_or(format!("unknown theme {name}: light or dark"))
}

/// The package's thumbnailer entry, the one installed under /usr/share.
const ENTRY: &str = include_str!("../share/thumbnailers/tridi.thumbnailer");
/// The package's desktop entry: its MimeType= line is what we open.
const DESKTOP: &str = include_str!("../share/applications/tridi.desktop");
/// Extensions whose cached thumbnails a theme switch (or an install) throws
/// away: everything we draw, plus STEP, whose 1.0 thumbnails would otherwise
/// stay forever now that nothing redraws them.
const EXTS: [&str; 12] = ["pcd", "ply", "xyz", "xyzrgb", "pts", "glb", "gltf", "obj", "stl", "off", "step", "stp"];

fn xdg(var: &str, fallback: &str) -> Option<PathBuf> {
    std::env::var_os(var).map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(fallback)))
}

fn config_file() -> Option<PathBuf> {
    Some(xdg("XDG_CONFIG_HOME", ".config")?.join("tridi/theme"))
}

fn override_file() -> Option<PathBuf> {
    Some(xdg("XDG_DATA_HOME", ".local/share")?.join("thumbnailers/tridi.thumbnailer"))
}

/// The viewer's theme: the one `tridi theme` saved, else light.
pub fn current() -> &'static Theme {
    config_file()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| by_name(s.trim()).ok())
        .unwrap_or(&LIGHT)
}

/// The package's entry, with `--theme dark` added for the dark theme.
fn entry(theme: &Theme) -> String {
    if theme.name == "dark" { ENTRY.replace(" thumb %i", " thumb --theme dark %i") } else { ENTRY.to_owned() }
}

/// `tridi theme [light|dark]`: without a name, prints the current one.
pub fn command(name: Option<&str>) -> Result<(), String> {
    let Some(name) = name else {
        println!("{}", current().name);
        return Ok(());
    };
    let theme = by_name(name)?;
    let cfg = config_file().ok_or("no $HOME")?;
    std::fs::create_dir_all(cfg.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&cfg, format!("{}\n", theme.name)).map_err(|e| e.to_string())?;
    install(theme)?;
    let cache = xdg("XDG_CACHE_HOME", ".cache").ok_or("no $HOME")?.join("thumbnails");
    let n = clear_thumbnails(&cache);
    println!("theme {}: viewer and thumbnails; {n} cached thumbnails cleared, Nautilus redraws them", theme.name);
    Ok(())
}

/// The per-user half of the install, which the package can't do: our copy of
/// the thumbnailer entry, and tridi as the default app for our types.
fn install(theme: &Theme) -> Result<(), String> {
    let ovr = override_file().ok_or("no $HOME")?;
    std::fs::create_dir_all(ovr.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&ovr, entry(theme)).map_err(|e| e.to_string())?;
    let types = DESKTOP.lines().find_map(|l| l.strip_prefix("MimeType=")).unwrap_or("");
    let ok = std::process::Command::new("xdg-mime")
        .args(["default", "tridi.desktop"])
        .args(types.split(';').filter(|t| !t.is_empty()))
        .status()
        .is_ok_and(|s| s.success());
    if !ok {
        eprintln!("[tridi] xdg-mime failed: tridi isn't the default app for our types");
    }
    Ok(())
}

/// After a package upgrade that changed our types, the user's copy of the
/// entry is stale: the viewer redoes the install when it sees that. Only for
/// users who ran `tridi theme`; errors are ignored, the viewer comes first.
/// The cached thumbnails go too: until now the new types were drawn by the
/// package's entry, which is the light theme.
pub fn refresh() {
    let theme = current();
    if let Some(ovr) = override_file()
        && std::fs::read_to_string(&ovr).is_ok_and(|s| s != entry(theme))
    {
        let _ = install(theme);
        if let Some(cache) = xdg("XDG_CACHE_HOME", ".cache") {
            clear_thumbnails(&cache.join("thumbnails"));
        }
    }
}

/// Deletes the cached thumbnails of our formats under `dir` (a
/// `~/.cache/thumbnails`), failed attempts included; returns how many.
pub fn clear_thumbnails(dir: &Path) -> usize {
    let mut n = 0;
    let mut dirs = vec![dir.to_path_buf()];
    while let Some(d) = dirs.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                dirs.push(p);
            } else if p.extension().is_some_and(|x| x == "png")
                && std::fs::read(&p).ok().and_then(|b| thumb_uri(&b)).is_some_and(|u| ours(&u))
                && std::fs::remove_file(&p).is_ok()
            {
                n += 1;
            }
        }
    }
    n
}

fn ours(uri: &str) -> bool {
    let ext = uri.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    EXTS.contains(&ext.as_str())
}

/// The `Thumb::URI` text chunk of a thumbnail PNG (freedesktop thumbnail spec).
fn thumb_uri(png: &[u8]) -> Option<String> {
    let mut i = 8;
    while i + 8 <= png.len() {
        let len = u32::from_be_bytes(png[i..i + 4].try_into().ok()?) as usize;
        let kind = &png[i + 4..i + 8];
        let data = png.get(i + 8..i + 8 + len)?;
        if kind == b"tEXt"
            && let Some(uri) = data.strip_prefix(b"Thumb::URI\0")
        {
            return Some(String::from_utf8_lossy(uri).into_owned());
        }
        if kind == b"IEND" {
            break;
        }
        i += 12 + len;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(uri: &str) -> Vec<u8> {
        let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
        let text = [b"Thumb::URI\0".as_slice(), uri.as_bytes()].concat();
        b.extend((text.len() as u32).to_be_bytes());
        b.extend(b"tEXt");
        b.extend(&text);
        b.extend([0u8; 4]); // CRC, not checked
        b.extend([0, 0, 0, 0]);
        b.extend(b"IEND");
        b.extend([0u8; 4]);
        b
    }

    #[test]
    fn clears_only_our_thumbnails() {
        let dir = std::env::temp_dir().join(format!("tridi-thumbs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (sub, name, uri) in [
            ("normal", "a.png", "file:///home/x/scan.pcd"),
            ("large", "b.png", "file:///home/x/car%20body.GLB"),
            ("fail/gnome-thumbnail-factory", "c.png", "file:///home/x/part.step"),
            ("normal", "d.png", "file:///home/x/photo.jpg"),
        ] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
            std::fs::write(dir.join(sub).join(name), png(uri)).unwrap();
        }
        assert_eq!(clear_thumbnails(&dir), 3);
        assert!(dir.join("normal/d.png").exists(), "a photo's thumbnail must stay");
    }

    #[test]
    fn dark_entry_only_adds_the_theme() {
        assert_eq!(entry(&LIGHT), ENTRY);
        let d = entry(&DARK);
        assert!(d.contains("Exec=/usr/bin/tridi thumb --theme dark %i %o %s"), "{d}");
        let mime = |s: &str| s.lines().find(|l| l.starts_with("MimeType=")).map(str::to_owned);
        assert_eq!(mime(&d), mime(ENTRY));
    }
}
