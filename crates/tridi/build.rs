//! Builds the STEP child's OpenCASCADE side (`src/step/occt.cpp`) against the
//! system's OpenCASCADE: headers in `OCCT_INCLUDE_DIR`, libraries in
//! `OCCT_LIB_DIR`, or where the distributions put them.

use std::{env, path::PathBuf};

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

fn main() {
    println!("cargo:rerun-if-changed=src/step/occt.cpp");
    println!("cargo:rerun-if-env-changed=OCCT_INCLUDE_DIR");
    println!("cargo:rerun-if-env-changed=OCCT_LIB_DIR");
    let include = env::var_os("OCCT_INCLUDE_DIR").map_or_else(|| PathBuf::from("/usr/include/opencascade"), PathBuf::from);
    let lib = env::var_os("OCCT_LIB_DIR").map(PathBuf::from).or_else(|| {
        ["/usr/lib", "/usr/lib64", "/usr/lib/x86_64-linux-gnu", "/usr/lib/aarch64-linux-gnu"]
            .into_iter()
            .map(PathBuf::from)
            .find(|d| d.join("libTKernel.so").exists())
    });
    cc::Build::new()
        .cpp(true)
        .std("c++17")
        .include(&include)
        // OpenCASCADE's own headers warn by the hundred
        .warnings(false)
        .file("src/step/occt.cpp")
        .compile("tridi_occt");
    if let Some(dir) = &lib {
        println!("cargo:rustc-link-search=native={}", dir.display());
    }
    let new = lib.as_ref().is_none_or(|d| d.join("libTKDESTEP.so").exists());
    for l in LIBS.iter().chain(if new { STEP_NEW } else { STEP_OLD }) {
        println!("cargo:rustc-link-lib=dylib={l}");
    }
}
