#![no_main]
// only the readers, the leaf of the crate: the library would pull in the
// whole window stack, which does not build with the sanitizers
#[allow(dead_code)]
#[path = "../../src/formats/mod.rs"]
mod formats;

// the first byte picks the extension, the rest is the file
libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    if let Some((&k, rest)) = data.split_first() {
        let _ = formats::parse_cloud(rest, ["xyz", "xyzrgb", "pts"][k as usize % 3]);
    }
});
