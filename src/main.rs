//! tridi - minimal viewer for point clouds (pcd, ply, xyz, xyzrgb, pts) and
//! meshes (glb, gltf, obj, stl, off, ply with faces, step/stp).
//!
//!     tridi [--theme T] FILE [FILE ...]        window
//!     tridi thumb [--theme T] IN OUT SIZE      PNG thumbnail, no window (for Nautilus)
//!     tridi theme [light|dark]                 show or switch the theme (viewer and thumbnails)
//!     tridi clear-thumbnails [DIR ...]         drop cached thumbnails of our formats
//!
//! T is `light` or `dark`, both Nord. The viewer uses the one `tridi theme`
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

mod desktop;
mod thumb;
mod view;

use eyre::WrapErr;
use std::path::Path;
use std::process::ExitCode;
use tridi::{LIGHT, Theme};

#[derive(clap::Parser)]
#[command(
    version,
    disable_help_subcommand = true,
    override_usage = "tridi [OPTIONS] FILE...\n       tridi [OPTIONS] <COMMAND>",
    about = "Viewer for point clouds and meshes, and their thumbnails in Nautilus",
    after_help = AFTER_HELP
)]
struct Cli {
    /// Files to open together in one window
    #[arg(value_name = "FILE")]
    files: Vec<String>,
    /// Theme for this run, instead of the one `tridi theme` saved
    #[arg(long, global = true, value_name = "THEME", value_parser = ["light", "dark"])]
    theme: Option<String>,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(clap::Subcommand)]
enum Cmd {
    /// Write a PNG thumbnail of IN to OUT, without a window (what Nautilus runs)
    Thumb {
        /// The file to draw
        #[arg(value_name = "IN")]
        input: String,
        /// The PNG to write
        #[arg(value_name = "OUT")]
        output: String,
        /// Width and height in pixels
        #[arg(value_name = "SIZE", value_parser = clap::value_parser!(u32).range(1..=4096))]
        size: u32,
    },
    /// Show the theme, or switch viewer and thumbnails to another one
    Theme {
        /// The theme to switch to; without it, prints the current one
        #[arg(value_parser = ["light", "dark"])]
        name: Option<String>,
    },
    /// Drop the cached thumbnails of our formats (default ~/.cache/thumbnails)
    ClearThumbnails {
        /// Thumbnail cache directories
        dirs: Vec<std::path::PathBuf>,
    },
    /// Internal: step.rs runs itself as a child to tessellate
    #[command(hide = true)]
    StepMesh { input: String },
    /// Packaging: write the man page and the bash, zsh and fish completions to DIR
    #[command(hide = true)]
    Generate { dir: std::path::PathBuf },
}

const AFTER_HELP: &str = "\
Examples:
  tridi scan.pcd                   open a point cloud
  tridi a.pcd b.pcd part.step      open several files, one color each
  tridi --theme dark model.glb     dark theme for this run only
  tridi theme dark                 switch viewer and thumbnails to dark

Formats:
  point clouds  pcd, ply (no faces), xyz, xyzrgb, pts
  meshes        glb, gltf, obj, stl, off, ply (with faces)
  CAD           step, stp

Window:
  left drag             orbit
  right or middle drag  pan (or shift + left drag)
  wheel                 zoom
  R                     reset the view
  + and -               point size
  1-9                   turn the N-th file off and on
  I                     show or hide the legend (on with several files)
  H or ?                show or hide these keys
  Q or Esc              quit

Environment:
  TRIDI_DEBUG=1    print which GPU renders

Files:
  ~/.config/tridi/theme
      the theme `tridi theme` saved
  ~/.local/share/thumbnailers/tridi.thumbnailer
      our thumbnailer entry, which wins over f3d's

Bugs: https://github.com/rccnroll/tridi/issues";

/// The Window section of the help, which H shows in the viewer.
fn window_keys() -> &'static str {
    let s = &AFTER_HELP[AFTER_HELP.find("Window:").unwrap_or(0)..];
    &s[..s.find("\n\n").unwrap_or(s.len())]
}

/// `tridi generate DIR`: the man pages (tridi.1, one per subcommand), and
/// tridi.bash, _tridi and tridi.fish, which the packages install.
fn generate(dir: &Path) -> eyre::Result<()> {
    use clap_complete::Shell;
    let mut cmd = <Cli as clap::CommandFactory>::command();
    std::fs::create_dir_all(dir).wrap_err_with(|| format!("cannot create {}", dir.display()))?;
    clap_mangen::generate_to(cmd.clone(), dir).wrap_err("cannot write the man pages")?;
    for sh in [Shell::Bash, Shell::Zsh, Shell::Fish] {
        clap_complete::generate_to(sh, &mut cmd, "tridi", dir).wrap_err_with(|| format!("cannot write the {sh} completions"))?;
    }
    Ok(())
}

fn main() -> ExitCode {
    let cli = <Cli as clap::Parser>::parse();
    let t = cli.theme.as_deref().and_then(Theme::by_name);
    let r = match cli.cmd {
        Some(Cmd::StepMesh { input }) => {
            // the parent reads this and puts the file's name in front
            return tridi::step_mesh_to_stdout(&input).map_or_else(
                |e| {
                    eprintln!("{:#}", eyre::Report::new(e));
                    ExitCode::FAILURE
                },
                |()| ExitCode::SUCCESS,
            );
        }
        Some(Cmd::Generate { dir }) => generate(&dir),
        // thumbnails can't read the saved theme (sandbox): light unless told
        Some(Cmd::Thumb { input, output, size }) => thumb::thumb(&input, &output, size, t.unwrap_or(&LIGHT)),
        Some(Cmd::Theme { name }) => desktop::command(name.as_deref()),
        Some(Cmd::ClearThumbnails { dirs }) => {
            let home = std::env::var_os("HOME").map(|h| Path::new(&h).join(".cache/thumbnails"));
            let dirs = if dirs.is_empty() { home.into_iter().collect() } else { dirs };
            let n: usize = dirs.iter().map(|d| desktop::clear_thumbnails(d)).sum();
            println!("{n} cached thumbnails cleared");
            Ok(())
        }
        None if cli.files.is_empty() => {
            <Cli as clap::CommandFactory>::command().print_help().ok();
            return ExitCode::from(2);
        }
        None => {
            desktop::refresh();
            view::view(&cli.files, t.unwrap_or_else(desktop::current))
        }
    };
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            // the whole chain on one line: "a.step: gave up tessellating after 15 s"
            eprintln!("[tridi] {e:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_line() {
        use clap::{CommandFactory, Parser};
        Cli::command().debug_assert();
        // the thumbnailer entries' Exec lines, light and dark
        for a in [
            &["tridi", "thumb", "i", "o", "256"][..],
            &["tridi", "thumb", "--theme", "dark", "i", "o", "256"],
        ] {
            assert!(matches!(Cli::parse_from(a).cmd, Some(Cmd::Thumb { size: 256, .. })));
        }
        let c = Cli::parse_from(["tridi", "--theme", "dark", "a.pcd", "b.step"]);
        assert_eq!((c.files.len(), c.theme.as_deref()), (2, Some("dark")));
        assert!(Cli::try_parse_from(["tridi", "--help"]).is_err_and(|e| e.exit_code() == 0));
        assert!(Cli::try_parse_from(["tridi", "-x"]).is_err_and(|e| e.exit_code() == 2));
        // H shows the Window section, all of it and nothing after
        let k = window_keys();
        assert!(k.starts_with("Window:") && k.ends_with("quit") && k.contains("H or ?"), "{k}");
    }

    #[test]
    fn generate_writes_what_the_packages_install() {
        let dir = std::env::temp_dir().join(format!("tridi-generate-{}", std::process::id()));
        generate(&dir).unwrap();
        for f in [
            "tridi.1",
            "tridi-thumb.1",
            "tridi-theme.1",
            "tridi-clear-thumbnails.1",
            "tridi.bash",
            "_tridi",
            "tridi.fish",
        ] {
            assert!(dir.join(f).is_file(), "{f}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
