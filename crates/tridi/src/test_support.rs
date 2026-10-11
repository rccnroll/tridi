//! Fixtures shared by the tests of several modules.

// ========================================== Imports ========================================== {{{

use std::{env, fmt::Write, fs, path::PathBuf, process};
use three_d::{InnerSpace, Vec3};

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

pub fn close(a: Vec3, b: [f32; 3]) -> bool {
    (a - Vec3::from(b)).magnitude() < 1e-3
}

// }}}
