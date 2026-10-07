#![no_main]
// tridi is a binary, not a library: its parser comes in by path
#[allow(dead_code)]
#[path = "../../src/cloud.rs"]
mod cloud;
#[allow(dead_code)]
#[path = "../../src/theme.rs"]
mod theme;

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let _ = cloud::parse(data, "off");
});
