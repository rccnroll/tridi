//! Fixtures shared by the tests of several modules.

// ========================================== Imports ========================================== {{{

#[cfg(feature = "thumbnail")]
use glutin::api::egl::device::Device;
#[cfg(feature = "thumbnail")]
use std::sync::Once;
use std::{env, fmt::Write, fs, path::PathBuf, process};

#[cfg(feature = "thumbnail")]
use crate::quiet_stderr;

// }}}

// ========================================= Fixtures ========================================== {{{

pub fn tmp(name: &str, data: &[u8]) -> PathBuf {
    let dir = env::temp_dir().join(format!("tridi-test-{}", process::id()));
    fs::create_dir_all(&dir).unwrap();
    let p = dir.join(name);
    fs::write(&p, data).unwrap();
    p
}

pub fn ascii_pcd(pts: &[[f32; 3]]) -> Vec<u8> {
    let mut s = format!(
        "# .PCD v0.7\nVERSION 0.7\nFIELDS x y z\nSIZE 4 4 4\nTYPE F F F\nCOUNT 1 1 1\nWIDTH {n}\nHEIGHT 1\nVIEWPOINT 0 0 0 1 0 0 0\nPOINTS {n}\nDATA ascii\n",
        n = pts.len()
    );
    for p in pts {
        writeln!(s, "{} {} {}", p[0], p[1], p[2]).expect("writing to a String can't fail");
    }
    s.into_bytes()
}

pub fn close(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum::<f32>().sqrt() < 1e-3
}

/// Headless GL on Mesa's llvmpipe from here on, GPU or not: the same
/// pixels everywhere, as in CI. Once, before any test opens EGL, which reads
/// the environment from C.
#[cfg(feature = "thumbnail")]
pub fn llvmpipe() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let software = quiet_stderr(Device::query_devices)
            .unwrap()
            .position(|d| d.extensions().contains("EGL_MESA_device_software"))
            .expect("no llvmpipe EGL device");
        // SAFETY: under the Once, before the tests that read it start EGL
        unsafe { env::set_var("TRIDI_EGL_DEVICE", software.to_string()) };
    });
}

// }}}
