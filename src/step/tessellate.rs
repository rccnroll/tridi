//! The child's work, done by OpenCASCADE (`occt.cpp`): the STEP file's parts
//! healed and meshed in parallel, each handed on placed where the assembly
//! puts it, as FLOATS f32 per vertex.

// ========================================== Imports ========================================== {{{

use std::{
    ffi::{CString, c_char, c_int, c_void},
    fs, io,
    os::unix::ffi::OsStrExt,
    path::Path,
    slice,
    sync::{Mutex, PoisonError},
};

use crate::step::{Quality, StepError, StepResult};

// }}}

// ============================================ FFI ============================================ {{{

/// What `tridi_step_tessellate` returns.
const OK: c_int = 0;
const NOT_STEP: c_int = 1;
const NO_SOLID: c_int = 2;

/// OpenCASCADE's document application is one for the process, and not for
/// two readers at once (colors came out mixed): one file at a time. The
/// child reads one anyway; the tests read several.
static OCCT: Mutex<()> = Mutex::new(());

type IdsFn = unsafe extern "C" fn(*mut c_void, *const u64, usize) -> c_int;
type PartFn = unsafe extern "C" fn(*mut c_void, u64, *const f32, usize) -> c_int;

unsafe extern "C" {
    fn tridi_step_tessellate(
        path: *const c_char,
        coarse: c_int,
        only: *const u64,
        n_only: usize,
        ctx: *mut c_void,
        ids: IdsFn,
        part: PartFn,
    ) -> c_int;
}

/// The two callbacks behind the `ctx` pointer, and the first error one of
/// them returned. `part` runs on several threads at once.
struct Ctx<I, P> {
    ids: Mutex<Option<I>>,
    part: P,
    err: Mutex<Option<io::Error>>,
}

impl<I, P> Ctx<I, P> {
    /// 0 to go on; else the error kept, and nonzero to stop.
    fn keep(&self, r: io::Result<()>) -> c_int {
        let Err(e) = r else { return 0 };
        if let Ok(mut slot) = self.err.lock() {
            slot.get_or_insert(e);
        }
        1
    }
}

/// `n` values at `p`; C++ hands a null pointer for none.
///
/// # Safety
/// Unless `n` is 0, `p` points at `n` values alive for `'a`.
unsafe fn values<'a, T>(p: *const T, n: usize) -> &'a [T] {
    if n == 0 || p.is_null() {
        return &[];
    }
    // SAFETY: the caller's promise
    unsafe { slice::from_raw_parts(p, n) }
}

unsafe extern "C" fn on_ids<I, P>(ctx: *mut c_void, ids: *const u64, n: usize) -> c_int
where
    I: FnOnce(&[u64]) -> io::Result<()>,
{
    // SAFETY: `ctx` is the Ctx<I, P> tessellate passed, alive for the call
    let ctx = unsafe { &*ctx.cast::<Ctx<I, P>>() };
    // SAFETY: C++ hands its id vector, alive for the call
    let ids = unsafe { values(ids, n) };
    let f = ctx.ids.lock().ok().and_then(|mut f| f.take());
    f.map_or(1, |f| ctx.keep(f(ids)))
}

unsafe extern "C" fn on_part<I, P>(ctx: *mut c_void, id: u64, v: *const f32, n: usize) -> c_int
where
    P: Fn(u64, &[f32]) -> io::Result<()> + Sync,
{
    // SAFETY: as in on_ids; only `part` and `err` are shared between the
    // threads, the one Sync, the other a Mutex
    let ctx = unsafe { &*ctx.cast::<Ctx<I, P>>() };
    // SAFETY: C++ hands its float vector, alive for the call
    let v = unsafe { values(v, n) };
    ctx.keep((ctx.part)(id, v))
}

// }}}

// ======================================= Tessellation ======================================== {{{

/// `ids` first gets the parts to do: the file's, or those of them in `only`
/// if it isn't empty; then `part` gets each one's id and triangles, at every
/// place the assembly uses it, as soon as it's done (empty if OpenCASCADE
/// can't mesh it), from several threads. A part's ids are its rank in the
/// file's assembly, the same on every run.
pub(crate) fn tessellate<I, P>(path: &Path, quality: Quality, only: &[u64], ids: I, part: P) -> StepResult<()>
where
    I: FnOnce(&[u64]) -> io::Result<()>,
    P: Fn(u64, &[f32]) -> io::Result<()> + Sync,
{
    // OpenCASCADE tells a missing file from a bad one only on its console
    fs::File::open(path).map_err(StepError::Read)?;
    let c_path = CString::new(path.as_os_str().as_bytes()).map_err(|e| StepError::Read(io::Error::other(e)))?;
    let ctx = Ctx {
        ids: Mutex::new(Some(ids)),
        part,
        err: Mutex::new(None),
    };
    let _one = OCCT.lock().unwrap_or_else(PoisonError::into_inner);
    // SAFETY: every pointer outlives the call, which returns only once its
    // threads are done; the callbacks are the ones for this Ctx's types
    let status = unsafe {
        tridi_step_tessellate(
            c_path.as_ptr(),
            c_int::from(quality == Quality::Coarse),
            only.as_ptr(),
            only.len(),
            (&raw const ctx).cast_mut().cast(),
            on_ids::<I, P>,
            on_part::<I, P>,
        )
    };
    match status {
        OK => Ok(()),
        NOT_STEP => Err(StepError::NotStep),
        NO_SOLID => Err(StepError::NoSolid),
        _ => Err(StepError::Write(
            ctx.err
                .into_inner()
                .ok()
                .flatten()
                .unwrap_or_else(|| io::Error::other("a callback failed")),
        )),
    }
}

// }}}
