//! Builds tridi-step-helper (`helper/main.cpp` and `helper/occt.cpp`)
//! against the system's OpenCASCADE: headers in `OCCT_INCLUDE_DIR`,
//! libraries in `OCCT_LIB_DIR`, or where the distributions put them. With
//! none there, or with `TRIDI_OCCT=bundled`, against OpenCASCADE built from
//! the source in occt-sys and linked statically: slow the first time, but
//! nothing to install, and a helper that needs no library at run time (the
//! Python wheel's). The crate embeds the executable (`src/helper.rs`).

use std::{
    env, fs,
    path::{Path, PathBuf},
};

/// The libraries every version needs.
const LIBS: &[&str] = &[
    "TKernel",
    "TKMath",
    "TKG2d",
    "TKG3d",
    "TKGeomBase",
    "TKGeomAlgo",
    "TKBRep",
    "TKTopAlgo",
    "TKShHealing",
    "TKMesh",
    "TKXSBase",
    "TKLCAF",
    "TKCAF",
    "TKCDF",
    "TKXCAF",
];

/// STEP from 7.8 on, and before.
const STEP_NEW: &[&str] = &["TKDESTEP", "TKDE"];
const STEP_OLD: &[&str] = &["TKSTEP", "TKSTEPBase", "TKSTEPAttr", "TKSTEP209", "TKXDESTEP"];

/// What a static TKXCAF pulls in that the shared one brings by itself.
const STATIC_EXTRA: &[&str] = &["TKVCAF", "TKV3d", "TKService", "TKHLR", "TKBool", "TKBO", "TKPrim"];

/// The OpenCASCADE the helper is built against: its headers, and the
/// linker arguments for its libraries.
struct Occt {
    include: PathBuf,
    link: Vec<String>,
}

#[expect(clippy::expect_used, reason = "a build script reports by panicking")]
fn main() {
    println!("cargo:rerun-if-changed=helper/main.cpp");
    println!("cargo:rerun-if-changed=helper/occt.cpp");
    println!("cargo:rerun-if-env-changed=OCCT_INCLUDE_DIR");
    println!("cargo:rerun-if-env-changed=OCCT_LIB_DIR");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR")).join("tridi-step-helper");
    // docs.rs has no OpenCASCADE, and documentation runs nothing
    if env::var_os("DOCS_RS").is_some() {
        fs::write(&out, b"").expect("OUT_DIR is writable");
        return;
    }
    println!("cargo:rerun-if-env-changed=TRIDI_OCCT");
    let bundled = env::var_os("TRIDI_OCCT").is_some_and(|v| v == "bundled");
    let occt = if bundled { None } else { system() }.unwrap_or_else(bundled_occt);
    let manifest = env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    // cc picks the compiler and the flags (CXX, CXXFLAGS) as for a library;
    // the link into an executable is ours
    let compiler = cc::Build::new().cpp(true).std("c++17").opt_level(2).get_compiler();
    let mut cmd = compiler.to_command();
    cmd.arg("-w") // OpenCASCADE's own headers warn by the hundred
        .arg(format!("-ffile-prefix-map={manifest}=/build")) // no build paths in the helper
        .arg("-I")
        .arg(&occt.include)
        .arg(Path::new(&manifest).join("helper/main.cpp"))
        .arg(Path::new(&manifest).join("helper/occt.cpp"))
        .args(["-pthread", "-s", "-o"])
        .arg(&out)
        .args(&occt.link);
    let status = cmd.status().expect("cannot run the C++ compiler");
    assert!(status.success(), "building tridi-step-helper failed: {status}");
}

/// The system's OpenCASCADE, if its headers are there.
fn system() -> Option<Occt> {
    let include = env::var_os("OCCT_INCLUDE_DIR").map_or_else(|| PathBuf::from("/usr/include/opencascade"), PathBuf::from);
    let version = include.join("Standard_Version.hxx");
    if !version.exists() {
        return None;
    }
    // another OpenCASCADE in the same place (a system upgrade): the helper
    // is built again against it
    println!("cargo:rerun-if-changed={}", version.display());
    let lib = env::var_os("OCCT_LIB_DIR").map(PathBuf::from).or_else(|| {
        ["/usr/lib", "/usr/lib64", "/usr/lib/x86_64-linux-gnu", "/usr/lib/aarch64-linux-gnu"]
            .into_iter()
            .map(PathBuf::from)
            .find(|d| d.join("libTKernel.so").exists())
    });
    let mut link = Vec::new();
    if let Some(dir) = &lib {
        link.push(format!("-L{}", dir.display()));
    }
    // a non-standard OCCT_LIB_DIR is found at run time too; the distributions' dirs need no rpath
    if let Some(dir) = env::var_os("OCCT_LIB_DIR") {
        link.push(format!("-Wl,-rpath,{}", Path::new(&dir).display()));
    }
    let new = lib.as_ref().is_none_or(|d| d.join("libTKDESTEP.so").exists());
    link.extend(LIBS.iter().chain(if new { STEP_NEW } else { STEP_OLD }).map(|l| format!("-l{l}")));
    Some(Occt { include, link })
}

/// OpenCASCADE built by occt-sys into the target dir (once: later builds
/// find it there), linked statically.
fn bundled_occt() -> Occt {
    println!("cargo:warning=building OpenCASCADE from source for STEP: about 10 minutes, the first time only");
    // occt-sys's CMakeLists predates CMake 4, which refuses it without this
    if env::var_os("CMAKE_POLICY_VERSION_MINIMUM").is_none() {
        // SAFETY: a build script is one thread, and nothing reads the
        // environment while it's set
        unsafe { env::set_var("CMAKE_POLICY_VERSION_MINIMUM", "3.5") };
    }
    occt_sys::build_occt();
    let root = occt_sys::occt_path();
    let mut link = vec![format!("-L{}", root.join("lib").display()), "-Wl,--start-group".to_owned()];
    link.extend(LIBS.iter().chain(STEP_NEW).chain(STATIC_EXTRA).map(|l| format!("-l:lib{l}.a")));
    // TKernel loads plugins; libdl is part of libc only from glibc 2.34 (manylinux_2_28 has 2.28)
    link.extend(["-Wl,--end-group", "-ldl"].map(str::to_owned));
    Occt {
        include: root.join("include"),
        link,
    }
}
