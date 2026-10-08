#![no_main]
// only the readers, the leaf of the crate: the library would pull in the
// whole window stack, which does not build with the sanitizers
#[allow(dead_code)]
#[path = "../../src/formats/mod.rs"]
mod formats;

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let _ = formats::parse_cloud(data, "pcd");
});
