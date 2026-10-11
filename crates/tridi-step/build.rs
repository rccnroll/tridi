//! Builds tridi-step-helper (`helper/main.cpp` and `helper/occt.cpp`)
//! against the system's OpenCASCADE: headers in `OCCT_INCLUDE_DIR`,
//! libraries in `OCCT_LIB_DIR`, or where the distributions put them. The
//! crate embeds the executable (`src/helper.rs`).

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
    let include = env::var_os("OCCT_INCLUDE_DIR").map_or_else(|| PathBuf::from("/usr/include/opencascade"), PathBuf::from);
    let lib = env::var_os("OCCT_LIB_DIR").map(PathBuf::from).or_else(|| {
        ["/usr/lib", "/usr/lib64", "/usr/lib/x86_64-linux-gnu", "/usr/lib/aarch64-linux-gnu"]
            .into_iter()
            .map(PathBuf::from)
            .find(|d| d.join("libTKernel.so").exists())
    });
    let manifest = env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    // cc picks the compiler and the flags (CXX, CXXFLAGS) as for a library;
    // the link into an executable is ours
    let compiler = cc::Build::new().cpp(true).std("c++17").opt_level(2).get_compiler();
    let mut cmd = compiler.to_command();
    cmd.arg("-w") // OpenCASCADE's own headers warn by the hundred
        .arg(format!("-ffile-prefix-map={manifest}=/build")) // no build paths in the helper
        .arg("-I")
        .arg(&include)
        .arg(Path::new(&manifest).join("helper/main.cpp"))
        .arg(Path::new(&manifest).join("helper/occt.cpp"))
        .args(["-pthread", "-s", "-o"])
        .arg(&out);
    if let Some(dir) = &lib {
        cmd.arg(format!("-L{}", dir.display()));
    }
    // a non-standard OCCT_LIB_DIR is found at run time too; the distributions' dirs need no rpath
    if let Some(dir) = env::var_os("OCCT_LIB_DIR") {
        cmd.arg(format!("-Wl,-rpath,{}", Path::new(&dir).display()));
    }
    let new = lib.as_ref().is_none_or(|d| d.join("libTKDESTEP.so").exists());
    for l in LIBS.iter().chain(if new { STEP_NEW } else { STEP_OLD }) {
        cmd.arg(format!("-l{l}"));
    }
    let status = cmd.status().expect("cannot run the C++ compiler");
    assert!(status.success(), "building tridi-step-helper failed: {status}");
}
