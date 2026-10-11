#![no_main]
// the readers alone: the viewer's stack does not build with the sanitizers

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let _ = tridi_formats::parse_cloud(data, "ply");
});
