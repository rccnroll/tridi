//! Fixtures shared by the tests of several modules.

// ========================================== Imports ========================================== {{{

use std::{env, fs, path::PathBuf, process};

// }}}

// ========================================= Fixtures ========================================== {{{

pub fn tmp(name: &str, data: &[u8]) -> PathBuf {
    let dir = env::temp_dir().join(format!("tridi-test-{}", process::id()));
    fs::create_dir_all(&dir).unwrap();
    let p = dir.join(name);
    fs::write(&p, data).unwrap();
    p
}

pub fn close(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum::<f32>().sqrt() < 1e-3
}

// }}}
