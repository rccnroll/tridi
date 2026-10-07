#![no_main]
// tridi is a binary, not a library: its parser comes in by path
#[allow(dead_code)]
#[path = "../../src/cloud.rs"]
mod cloud;
#[allow(dead_code)]
#[path = "../../src/theme.rs"]
mod theme;

// the first byte picks the extension, the rest is the file
libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    if let Some((&k, rest)) = data.split_first() {
        let _ = cloud::parse(rest, ["xyz", "xyzrgb", "pts"][k as usize % 3]);
    }
});
