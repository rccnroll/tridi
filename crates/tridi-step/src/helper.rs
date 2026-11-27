//! The helper program, embedded: written once per process to an in-memory
//! file and run from there, or from a copy in the cache where the system
//! forbids running from memory.

// ========================================== Imports ========================================== {{{

use std::{
    env,
    ffi::CStr,
    fs::{self, DirBuilder, File, OpenOptions},
    hash::{DefaultHasher, Hash, Hasher},
    io::{self, Write},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
    process::{self, Command},
    sync::OnceLock,
    thread,
    time::Duration,
};

// }}}

// ========================================= Constants ========================================= {{{

/// tridi-step-helper, built by build.rs.
static HELPER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/tridi-step-helper"));

/// The in-memory file's name, as /proc/PID/maps and `ls -l /proc/PID/fd` show it.
const NAME: &CStr = c"tridi-step-helper";

/// The helper's exit code without arguments (sysexits.h's `EX_USAGE`).
const USAGE: i32 = 64;

/// The exit code of the dynamic loader when a shared library is missing.
const NO_LIBRARY: i32 = 127;

/// How often a busy executable is tried: a fork in another thread can hold
/// a copy of the write fd of a file just written, until that child execs.
const BUSY_TRIES: u32 = 10;

/// The wait between those tries; the child execs within microseconds.
const BUSY_WAIT: Duration = Duration::from_millis(10);

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

/// Memory first; the cache, then our own directory in the temp dir, where
/// memory can't run. OpenCASCADE missing stops the search: it's missing
/// from every place.
fn place() -> io::Result<Place> {
    match memory() {
        Ok(fd) if runs(Path::new(&fd_path(&fd)))? => return Ok(Place::Memory(fd)),
        Ok(_) => debug!("the helper can't run from memory"),
        Err(e) => debug!("no in-memory file for the helper: {e}"),
    }
    let cache = env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|h| Path::new(&h).join(".cache")))
        .map(|c| c.join("tridi"));
    // the temp dir is everyone's: only a directory of ours alone will do
    let shared = env::temp_dir().join(format!("tridi-{}", uid()));
    let mut last = io::Error::other("no cache or temp dir");
    for (dir, private) in cache.map(|c| (c, false)).into_iter().chain([(shared, true)]) {
        let ready = if private { private_dir(&dir, uid()) } else { Ok(()) };
        match ready.and_then(|()| cache_copy(&dir)) {
            Ok(p) if runs(&p)? => {
                debug!("the helper runs from {}", p.display());
                return Ok(Place::Disk(p));
            }
            Ok(p) => last = io::Error::other(format!("{} can't run (noexec?)", p.display())),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// `run`, again while the executable is busy (ETXTBSY): a few times, then
/// the error. For every spawn of the helper from a disk path.
pub(crate) fn retry_busy<T>(mut run: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    for _ in 1..BUSY_TRIES {
        match run() {
            Err(e) if e.kind() == io::ErrorKind::ExecutableFileBusy => thread::sleep(BUSY_WAIT),
            done => return done,
        }
    }
    run()
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
    // a leftover of a run that died with our pid: a symlink is removed, not followed
    #[expect(
        clippy::let_underscore_must_use,
        reason = "mostly there is none; create_new says if it's in the way"
    )]
    let _ = fs::remove_file(&tmp);
    // O_EXCL: never through a link someone else put there
    let mut f = OpenOptions::new().write(true).create_new(true).mode(0o700).open(&tmp)?;
    f.write_all(HELPER)?;
    // closed before it runs, or exec says ETXTBSY
    drop(f);
    fs::rename(&tmp, &path)?;
    Ok(path)
}

/// `dir` as ours alone, made 0700 if missing: refused unless it's a real
/// directory (no symlink), owned by `uid`, mode 0700, since in a directory
/// every user writes to another one could have put a helper of theirs there.
fn private_dir(dir: &Path, uid: u32) -> io::Result<()> {
    if let Err(e) = DirBuilder::new().mode(0o700).create(dir)
        && e.kind() != io::ErrorKind::AlreadyExists
    {
        return Err(e);
    }
    let m = fs::symlink_metadata(dir)?;
    if !m.file_type().is_dir() || m.uid() != uid || m.mode() & 0o7777 != 0o700 {
        return Err(io::Error::other(format!("{} isn't a directory of ours alone", dir.display())));
    }
    Ok(())
}

fn uid() -> u32 {
    // SAFETY: getuid always succeeds and touches no memory
    unsafe { libc::getuid() }
}

/// Whether the helper starts from `exe`: without arguments it prints its
/// usage and exits `USAGE`. False if it can't be run from there (noexec);
/// an error if it runs but the loader can't find OpenCASCADE's libraries,
/// with the loader's message.
fn runs(exe: &Path) -> io::Result<bool> {
    let Ok(out) = retry_busy(|| Command::new(exe).output()) else {
        return Ok(false);
    };
    let stderr = String::from_utf8_lossy(&out.stderr);
    if out.status.code() == Some(NO_LIBRARY) || stderr.contains("error while loading shared libraries") {
        let line = stderr.lines().next().unwrap_or("the shared libraries are missing");
        return Err(io::Error::other(format!("cannot load OpenCASCADE: {line}")));
    }
    Ok(out.status.code() == Some(USAGE))
}

// }}}

// =========================================== Tests =========================================== {{{

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        env,
        fs::{self, Permissions},
        os::unix::fs::{PermissionsExt, symlink},
        process,
    };

    // where memfd exec is forbidden (vm.memfd_noexec = 2) the cache copy runs
    #[test]
    fn cache_copy_runs() {
        let dir = env::temp_dir().join(format!("tridi-helper-test-{}", process::id()));
        let exe = cache_copy(&dir).unwrap();
        let out = retry_busy(|| Command::new(&exe).output()).unwrap();
        // no arguments: the helper's usage, exit 64 (EX_USAGE)
        assert_eq!(out.status.code(), Some(64), "{out:?}");
        // a second call reuses the file
        assert_eq!(cache_copy(&dir).unwrap(), exe);
        fs::remove_dir_all(dir).unwrap();
    }

    // a planted directory in the temp dir is not used
    #[test]
    fn shared_dir_must_be_private() {
        let base = env::temp_dir().join(format!("tridi-private-test-{}", process::id()));
        fs::create_dir_all(&base).unwrap();
        let ours = base.join("ours");
        private_dir(&ours, uid()).unwrap();
        // again, now that it's there
        private_dir(&ours, uid()).unwrap();
        private_dir(&ours, uid() + 1).unwrap_err(); // someone else's
        let open = base.join("open");
        fs::create_dir_all(&open).unwrap();
        fs::set_permissions(&open, Permissions::from_mode(0o777)).unwrap();
        private_dir(&open, uid()).unwrap_err(); // 0777
        let link = base.join("link");
        symlink(&ours, &link).unwrap();
        private_dir(&link, uid()).unwrap_err(); // a symlink
        fs::remove_dir_all(base).unwrap();
    }

    // OpenCASCADE missing is told apart from a place that can't run
    #[test]
    fn missing_library_is_told() {
        let dir = env::temp_dir().join(format!("tridi-probe-test-{}", process::id()));
        fs::create_dir_all(&dir).unwrap();
        let script = |name: &str, body: &str| {
            let p = dir.join(name);
            fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
            fs::set_permissions(&p, Permissions::from_mode(0o700)).unwrap();
            p
        };
        let ld = "x: error while loading shared libraries: libTKernel.so.7.8: cannot open shared object file";
        let e = runs(&script("ld", &format!("echo '{ld}' >&2; exit 127"))).unwrap_err();
        assert_eq!(e.to_string(), format!("cannot load OpenCASCADE: {ld}"));
        assert!(runs(&script("ok", "exit 64")).unwrap(), "the usage exit");
        assert!(!runs(&script("other", "exit 3")).unwrap(), "any other exit");
        // not executable: can't run from there
        let noexec = script("noexec", "exit 64");
        fs::set_permissions(&noexec, Permissions::from_mode(0o600)).unwrap();
        assert!(!runs(&noexec).unwrap());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn memfd_or_cache_gives_a_command() {
        let out = command().unwrap().output().unwrap();
        assert_eq!(out.status.code(), Some(64), "{out:?}");
    }
}

// }}}
