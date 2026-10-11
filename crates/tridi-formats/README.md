# tridi-formats

Readers for point clouds: PCD (ascii, binary, binary_compressed), PLY with or without faces, OFF/COFF, and the text formats xyz, xyzrgb and pts.

`Cloud` holds what a file has: `points` as `[f32; 3]`, `colors` (0..1, one per point) only if the file has them, and `faces` as triangle index triples for PLY and OFF files that carry them. `load_cloud` picks the reader from the extension; `parse_cloud` takes the bytes and the extension.

```rust
let c = tridi_formats::load_cloud(Path::new("scan.pcd"))?;
println!("{} points", c.points.len());
```

The crate depends on nothing but `thiserror`, and its readers are fuzzed: every byte is untrusted, nothing panics. It pairs with any viewer, rerun included:

```rust
let c = tridi_formats::load_cloud(Path::new("scan.pcd"))?;
rec.log("scan", &rerun::Points3D::new(c.points))?;
```
