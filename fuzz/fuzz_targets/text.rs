#![no_main]
// the readers alone: the viewer's stack does not build with the sanitizers

// the first byte picks the extension, the rest is the file
libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    if let Some((&k, rest)) = data.split_first() {
        let _ = tridi_formats::parse_cloud(rest, ["xyz", "xyzrgb", "pts"][k as usize % 3]);
    }
});
