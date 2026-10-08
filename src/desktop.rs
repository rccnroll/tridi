//! The desktop side: `tridi theme`, which switches the viewer and the
//! thumbnails together, and the thumbnail cache.
//!
//! The thumbnailer runs in Nautilus's sandbox, which sees neither `$HOME` nor
//! the session bus and clears the environment: the only way in is the `Exec=`
//! line of a `.thumbnailer` file, read outside the sandbox. So the theme is a
//! copy of the package's entry in `~/.local/share/thumbnailers/`, with
//! `--theme dark` for the dark one. The copy is written for light too: next
//! to f3d's entries in /usr/share, which also claim glb, stl and obj, which
//! one wins is down to directory order; the user's directory comes first.

use eyre::{OptionExt, WrapErr};
use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

use tridi::{LIGHT, Theme};

use crate::tell;

/// The package's thumbnailer entry, the one installed under /usr/share.
const ENTRY: &str = include_str!("../share/thumbnailers/tridi.thumbnailer");
/// The package's desktop entry: its `MimeType`= line is what we open.
const DESKTOP: &str = include_str!("../share/applications/tridi.desktop");
/// Extensions whose cached thumbnails a theme switch (or an install) throws
/// away: everything we draw, plus STEP, whose 1.0 thumbnails would otherwise
/// stay forever now that nothing redraws them.
const EXTS: [&str; 12] = [
    "pcd", "ply", "xyz", "xyzrgb", "pts", "glb", "gltf", "obj", "stl", "off", "step", "stp",
];

fn xdg(var: &str, fallback: &str) -> Option<PathBuf> {
    env::var_os(var)
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|h| Path::new(&h).join(fallback)))
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
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| Theme::by_name(s.trim()))
        .unwrap_or(&LIGHT)
}

/// The package's entry, with `--theme dark` added for the dark theme.
fn entry(theme: &Theme) -> String {
    if theme.name == "dark" {
        ENTRY.replace(" thumb %i", " thumb --theme dark %i")
    } else {
        ENTRY.to_owned()
    }
}

/// `tridi theme [light|dark]`: without a name, prints the current one to
/// `out`.
pub fn command(name: Option<&str>, out: &mut dyn Write) -> eyre::Result<()> {
    let Some(name) = name else {
        tell(out, format_args!("{}", current().name));
        return Ok(());
    };
    let theme = Theme::by_name(name).ok_or_else(|| eyre::eyre!("unknown theme {name}: light or dark"))?;
    let cfg = config_file().ok_or_eyre("no $HOME")?;
    write(&cfg, &format!("{}\n", theme.name))?;
    install(theme)?;
    let cache = xdg("XDG_CACHE_HOME", ".cache").ok_or_eyre("no $HOME")?.join("thumbnails");
    let n = clear_thumbnails(&cache);
    tell(
        out,
        format_args!(
            "theme {}: viewer and thumbnails; {n} cached thumbnails cleared, Nautilus redraws them",
            theme.name
        ),
    );
    Ok(())
}

/// The per-user half of the install, which the package can't do: our copy of
/// the thumbnailer entry, and tridi as the default app for our types.
fn install(theme: &Theme) -> eyre::Result<()> {
    let ovr = override_file().ok_or_eyre("no $HOME")?;
    write(&ovr, &entry(theme))?;
    let types = DESKTOP.lines().find_map(|l| l.strip_prefix("MimeType=")).unwrap_or("");
    let ok = Command::new("xdg-mime")
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
/// users who ran `tridi theme`; an error is reported and the viewer opens
/// anyway.
/// The cached thumbnails go too: until now the new types were drawn by the
/// package's entry, which is the light theme.
pub fn refresh() {
    let theme = current();
    if let Some(ovr) = override_file()
        && fs::read_to_string(&ovr).is_ok_and(|s| s != entry(theme))
    {
        if let Err(e) = install(theme) {
            eprintln!("[tridi] cannot update the thumbnailer entry: {e:#}");
        }
        if let Some(cache) = xdg("XDG_CACHE_HOME", ".cache") {
            clear_thumbnails(&cache.join("thumbnails"));
        }
    }
}

/// Writes `text` to `path`, making its directory first.
fn write(path: &Path, text: &str) -> eyre::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).wrap_err_with(|| format!("cannot create {}", dir.display()))?;
    }
    fs::write(path, text).wrap_err_with(|| format!("cannot write {}", path.display()))
}

/// Deletes the cached thumbnails of our formats under `dir` (a
/// `~/.cache/thumbnails`), failed attempts included; returns how many.
pub fn clear_thumbnails(dir: &Path) -> usize {
    let mut n = 0;
    let mut dirs = vec![dir.to_path_buf()];
    while let Some(d) = dirs.pop() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                dirs.push(p);
            } else if p.extension().is_some_and(|x| x == "png")
                && fs::read(&p).ok().and_then(|b| thumb_uri(&b)).is_some_and(|u| ours(&u))
                && fs::remove_file(&p).is_ok()
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
    // after the signature, chunks: length, kind, data, CRC
    let mut rest = png.get(8..)?;
    while let Some((head, tail)) = rest.split_at_checked(8) {
        let (len, kind) = head.split_at_checked(4)?;
        let len = usize::try_from(u32::from_be_bytes(len.try_into().ok()?)).ok()?;
        let (data, tail) = tail.split_at_checked(len)?;
        if kind == b"tEXt"
            && let Some(uri) = data.strip_prefix(b"Thumb::URI\0")
        {
            return Some(String::from_utf8_lossy(uri).into_owned());
        }
        if kind == b"IEND" {
            break;
        }
        rest = tail.get(4..)?;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process;
    use tridi::DARK;

    fn png(uri: &str) -> Vec<u8> {
        let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
        let text = [b"Thumb::URI\0".as_slice(), uri.as_bytes()].concat();
        b.extend(u32::try_from(text.len()).unwrap().to_be_bytes());
        b.extend(b"tEXt");
        b.extend(&text);
        b.extend([0_u8; 4]); // CRC, not checked
        b.extend([0, 0, 0, 0]);
        b.extend(b"IEND");
        b.extend([0_u8; 4]);
        b
    }

    #[test]
    fn clears_only_our_thumbnails() {
        let dir = env::temp_dir().join(format!("tridi-thumbs-{}", process::id()));
        if dir.exists() {
            fs::remove_dir_all(&dir).unwrap();
        }
        for (sub, name, uri) in [
            ("normal", "a.png", "file:///home/x/scan.pcd"),
            ("large", "b.png", "file:///home/x/car%20body.GLB"),
            ("fail/gnome-thumbnail-factory", "c.png", "file:///home/x/part.step"),
            ("normal", "d.png", "file:///home/x/photo.jpg"),
        ] {
            fs::create_dir_all(dir.join(sub)).unwrap();
            fs::write(dir.join(sub).join(name), png(uri)).unwrap();
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
