//! The helper program, embedded: written once per process to an in-memory
//! file and run from there, or from a copy in the cache where the system
//! forbids running from memory.

// ========================================== Imports ========================================== {{{

use std::{
    env,
    ffi::CStr,
    fs::{self, File, Permissions},
    hash::{DefaultHasher, Hash, Hasher},
    io::{self, Write},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::fs::PermissionsExt,
    },
    path::{Path, PathBuf},
    process::{self, Command},
    sync::OnceLock,
};

// }}}

// ========================================= Constants ========================================= {{{

/// tridi-step-helper, built by build.rs.
static HELPER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/tridi-step-helper"));

/// The in-memory file's name, as /proc/PID/maps and `ls -l /proc/PID/fd` show it.
const NAME: &CStr = c"tridi-step-helper";

/// The helper's exit code without arguments (sysexits.h's `EX_USAGE`).
const USAGE: i32 = 64;

// }}}

// ========================================== Helper =========================================== {{{

/// Where the helper runs from, decided on first use.
enum Place {
    /// an in-memory file, run as /proc/self/fd/N
    Memory(OwnedFd),
    /// a file in the cache
    Disk(PathBuf),
}

/// A command that runs the helper; its arguments are the caller's. The
/// place is found once per process; a failure to find one is kept too, as
/// the next call would fail the same way.
pub(crate) fn command() -> io::Result<Command> {
    static PLACE: OnceLock<Result<Place, String>> = OnceLock::new();
    let place = PLACE.get_or_init(|| place().map_err(|e| e.to_string()));
    match place {
        Ok(Place::Memory(fd)) => Ok(Command::new(fd_path(fd))),
        Ok(Place::Disk(p)) => Ok(Command::new(p)),
        Err(e) => Err(io::Error::other(e.clone())),
    }
}

/// Memory first; the cache, then the temp dir, where memory can't run.
fn place() -> io::Result<Place> {
    match memory() {
        Ok(fd) if runs(Path::new(&fd_path(&fd))) => return Ok(Place::Memory(fd)),
        Ok(_) => debug!("the helper can't run from memory"),
        Err(e) => debug!("no in-memory file for the helper: {e}"),
    }
    let cache = env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|h| Path::new(&h).join(".cache")))
        .map(|c| c.join("tridi"));
    let mut last = io::Error::other("no cache or temp dir");
    for dir in cache.into_iter().chain([env::temp_dir().join("tridi")]) {
        match cache_copy(&dir) {
            Ok(p) if runs(&p) => {
                debug!("the helper runs from {}", p.display());
                return Ok(Place::Disk(p));
            }
            Ok(p) => last = io::Error::other(format!("{} can't run (noexec?)", p.display())),
            Err(e) => last = e,
        }
    }
    Err(last)
}

fn fd_path(fd: &OwnedFd) -> String {
    format!("/proc/self/fd/{}", fd.as_raw_fd())
}

/// The helper in an in-memory file. `MFD_EXEC` asks for an executable one
/// where `vm.memfd_noexec` = 1 makes them non-executable by default;
/// kernels before 6.3 don't know the flag and get the call without it.
fn memory() -> io::Result<OwnedFd> {
    let create = |flags| {
        // SAFETY: a C string that outlives the call, and plain flags
        let fd = unsafe { libc::memfd_create(NAME.as_ptr(), flags) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: a new descriptor, ours alone
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    };
    let fd = create(libc::MFD_CLOEXEC | libc::MFD_EXEC).or_else(|_e| create(libc::MFD_CLOEXEC))?;
    File::from(fd.try_clone()?).write_all(HELPER)?;
    Ok(fd)
}

/// The helper in `dir`, named by its content so versions don't collide:
/// written once (to a temp name, then renamed, so a half-written file never
/// runs), reused after. `DefaultHasher` is stable within one build of std,
/// all a cache name needs: another toolchain writes another file.
fn cache_copy(dir: &Path) -> io::Result<PathBuf> {
    let mut h = DefaultHasher::new();
    HELPER.hash(&mut h);
    let path = dir.join(format!("tridi-step-helper-{:016x}", h.finish()));
    if path.is_file() {
        return Ok(path);
    }
    fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".tridi-step-helper-{}", process::id()));
    fs::write(&tmp, HELPER)?;
    fs::set_permissions(&tmp, Permissions::from_mode(0o755))?;
    fs::rename(&tmp, &path)?;
    Ok(path)
}

/// Whether the helper starts from `exe`: without arguments it prints its
/// usage and exits `USAGE`.
fn runs(exe: &Path) -> bool {
    Command::new(exe).output().is_ok_and(|o| o.status.code() == Some(USAGE))
}

// }}}

// =========================================== Tests =========================================== {{{

#[cfg(test)]
mod tests {
    use super::*;
    use std::{env, fs, process};

    // where memfd exec is forbidden (vm.memfd_noexec = 2) the cache copy runs
    #[test]
    fn cache_copy_runs() {
        let dir = env::temp_dir().join(format!("tridi-helper-test-{}", process::id()));
        let exe = cache_copy(&dir).unwrap();
        let out = Command::new(&exe).output().unwrap();
        // no arguments: the helper's usage, exit 64 (EX_USAGE)
        assert_eq!(out.status.code(), Some(64), "{out:?}");
        // a second call reuses the file
        assert_eq!(cache_copy(&dir).unwrap(), exe);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn memfd_or_cache_gives_a_command() {
        let out = command().unwrap().output().unwrap();
        assert_eq!(out.status.code(), Some(64), "{out:?}");
    }
}

// }}}
