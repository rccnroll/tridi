//! tridi - minimal viewer for point clouds (pcd, ply, xyz, xyzrgb, pts) and
//! meshes (glb, gltf, obj, stl, off, ply with faces, step/stp).
//!
//!     tridi [--theme T] FILE [FILE ...]        window
//!     tridi thumb [--theme T] IN OUT SIZE      PNG thumbnail, no window (for the file manager)
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

// ======================================= Lint Settings ======================================= {{{

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::indexing_slicing,
        reason = "a test may panic: that is its failure"
    )
)]

// }}}

// ======================================== Sub-modules ======================================== {{{

mod desktop;
mod thumb;

// }}}

// ========================================== Imports ========================================== {{{

#[macro_use]
extern crate tracing;

use clap::{CommandFactory, Parser};
use clap_complete::Shell;
use eyre::WrapErr;
use std::{
    env, fmt, fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};
use tracing_subscriber::EnvFilter;
use tridi::{LIGHT, Theme};

// }}}

// ========================================= Constants ========================================= {{{

const EXAMPLES: &str = "\
Examples:
  tridi scan.pcd                   open a point cloud
  tridi a.pcd b.pcd part.step      open several files, one color each
  tridi --theme dark model.glb     dark theme for this run only
  tridi theme dark                 switch viewer and thumbnails to dark";

const FORMATS: &str = "\
Formats:
  point clouds  pcd, ply (no faces), xyz, xyzrgb, pts
  meshes        glb, gltf, obj, stl, off, ply (with faces)
  CAD           step, stp";

/// Environment, Files and Bugs: the end of the help.
const ENVIRONMENT: &str = "\
Environment:
  TRIDI_DEBUG=1    debug output: which GPU renders, how long things take
  TRIDI_LOG=info   a log filter instead (warnings only by default)

Files:
  ~/.config/tridi/theme
      the theme `tridi theme` saved
  ~/.local/share/thumbnailers/tridi.thumbnailer
      our thumbnailer entry, which wins over f3d's

Bugs: https://github.com/rccnroll/tridi/issues";

// }}}

// ======================================= Command line ======================================== {{{

#[derive(Parser)]
#[command(
    // the binary, not the package (tridi-cli): man pages, completions, --help
    name = "tridi",
    version,
    disable_help_subcommand = true,
    override_usage = "tridi [OPTIONS] FILE...\n       tridi [OPTIONS] <COMMAND>",
    about = "Viewer for point clouds and meshes, and their thumbnails in the file manager",
    after_help = after_help()
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
    /// Write a PNG thumbnail of IN to OUT, without a window (what the file manager runs)
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
        dirs: Vec<PathBuf>,
    },
    /// Packaging: write the man page and the bash, zsh and fish completions to DIR
    #[command(hide = true)]
    Generate { dir: PathBuf },
}

// }}}

// =========================================== Main ============================================ {{{

fn main() -> ExitCode {
    let cli = Cli::parse();
    init_logging();
    let mut out = io::stdout();
    let t = cli.theme.as_deref().and_then(Theme::by_name);
    let r = match cli.cmd {
        Some(Cmd::Generate { dir }) => generate(&dir),
        // thumbnails can't read the saved theme (sandbox): light unless told
        Some(Cmd::Thumb { input, output, size }) => thumb::thumb(&input, &output, size, t.unwrap_or(&LIGHT)),
        Some(Cmd::Theme { name }) => desktop::command(name.as_deref(), &mut out),
        Some(Cmd::ClearThumbnails { dirs }) => {
            let home = env::var_os("HOME").map(|h| Path::new(&h).join(".cache/thumbnails"));
            let dirs = if dirs.is_empty() { home.into_iter().collect() } else { dirs };
            let n: usize = dirs.iter().map(|d| desktop::clear_thumbnails(d)).sum();
            tell(&mut out, format_args!("{n} cached thumbnails cleared"));
            Ok(())
        }
        None if cli.files.is_empty() => {
            tell(&mut out, format_args!("{}", Cli::command().render_help()));
            return ExitCode::from(2);
        }
        None => {
            desktop::refresh();
            // each file that failed was logged by the viewer
            tridi::Viewer::new()
                .files(&cli.files)
                .theme(t.unwrap_or_else(desktop::current))
                .output(Box::new(io::stdout()))
                .show()
                .map(|_shown| ())
                .map_err(eyre::Report::new)
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

/// Logs to stderr, stdout being the user's: warnings by default,
/// `TRIDI_DEBUG=1` for debug, or a filter in `TRIDI_LOG` (`info`,
/// `tridi=debug`, ...).
fn init_logging() {
    let default = if env::var_os("TRIDI_DEBUG").is_some() { "debug" } else { "warn" };
    let filter = EnvFilter::try_from_env("TRIDI_LOG").unwrap_or_else(|_err| EnvFilter::new(default));
    tracing_subscriber::fmt()
        .with_writer(io::stderr)
        .with_env_filter(filter)
        .without_time()
        .with_target(false)
        .init();
}

// }}}

// ========================================== Helpers ========================================== {{{

/// The help's end: examples, formats, the window's keys, the environment.
fn after_help() -> String {
    format!("{EXAMPLES}\n\n{FORMATS}\n\n{}\n\n{ENVIRONMENT}", tridi::KEYS)
}

/// A line for the user on `out` (stdout): what tridi read, what it did.
pub fn tell(out: &mut dyn Write, line: fmt::Arguments) {
    #[expect(clippy::let_underscore_must_use, reason = "a closed stdout (a pipe to head) is no reason to stop")]
    let _ = writeln!(out, "{line}");
}

/// `tridi generate DIR`: the man pages (tridi.1, one per subcommand), and
/// tridi.bash, _tridi and tridi.fish, which the packages install.
fn generate(dir: &Path) -> eyre::Result<()> {
    let mut cmd = Cli::command();
    fs::create_dir_all(dir).wrap_err_with(|| format!("cannot create {}", dir.display()))?;
    clap_mangen::generate_to(cmd.clone(), dir).wrap_err("cannot write the man pages")?;
    for sh in [Shell::Bash, Shell::Zsh, Shell::Fish] {
        clap_complete::generate_to(sh, &mut cmd, "tridi", dir).wrap_err_with(|| format!("cannot write the {sh} completions"))?;
    }
    Ok(())
}

// }}}

// =========================================== Tests =========================================== {{{

#[cfg(test)]
mod tests {
    use super::*;
    use std::process;

    #[test]
    fn command_line() {
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
        // the help lists the keys H shows in the window
        assert!(
            Cli::command().render_long_help().to_string().contains(tridi::KEYS),
            "the keys in the help"
        );
    }

    #[test]
    fn generate_writes_what_the_packages_install() {
        let dir = env::temp_dir().join(format!("tridi-generate-{}", process::id()));
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
        fs::remove_dir_all(dir).unwrap();
    }
}

// }}}
