//! `cargo run -p tridi --example show -- FILE...`: the files, and a helix
//! computed here, in one window.

use std::{env, error::Error, f32::consts::TAU};

fn main() -> Result<(), Box<dyn Error>> {
    let helix: Vec<[f32; 3]> = (0..2000_u16)
        .map(|i| {
            let t = f32::from(i) / 2000.0;
            [(6.0 * TAU * t).cos(), (6.0 * TAU * t).sin(), 3.0 * t]
        })
        .collect();
    let shown = tridi::Viewer::new()
        .files(env::args().skip(1))
        .cloud("helix", &helix, None)
        .show()?;
    for (name, e) in &shown.failed {
        eprintln!("{name}: {e}");
    }
    Ok(())
}
