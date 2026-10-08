//! Themes, Nord light and Nord dark: the colors the viewer and the
//! thumbnails draw with.

/// The colors of a look: background, height ramp, one tint per file when
/// several are open, and the color of meshes without a material.
pub struct Theme {
    pub name: &'static str,
    pub bg: [f32; 3],
    pub ramp: [[f32; 3]; 2],
    pub palette: [[f32; 3]; 6],
    pub mesh: [f32; 3],
}

/// The default: Nord snow storm background, everything else dark.
pub const LIGHT: Theme = Theme {
    name: "light",
    bg: [0.925, 0.937, 0.957],                      // nord6
    ramp: [[0.14, 0.16, 0.21], [0.37, 0.51, 0.67]], // nord0 -> nord9
    // Nord aurora, darkened just enough to read on a light background
    palette: [
        [0.63, 0.25, 0.28], // red
        [0.25, 0.40, 0.58], // blue
        [0.35, 0.52, 0.31], // green
        [0.55, 0.36, 0.52], // purple
        [0.72, 0.42, 0.26], // orange
        [0.20, 0.51, 0.55], // teal
    ],
    mesh: [0.25, 0.30, 0.38], // 1.0's CAD_GREY
};

/// Nord polar night background (1.0's thumbnails), everything else light.
pub const DARK: Theme = Theme {
    name: "dark",
    bg: [0.18, 0.20, 0.25],                         // nord0
    ramp: [[0.37, 0.51, 0.67], [0.53, 0.75, 0.82]], // nord10 -> nord8
    palette: [
        [0.75, 0.38, 0.42], // nord11 red
        [0.51, 0.63, 0.76], // nord9 blue
        [0.64, 0.75, 0.55], // nord14 green
        [0.71, 0.56, 0.68], // nord15 purple
        [0.82, 0.53, 0.44], // nord12 orange
        [0.56, 0.74, 0.73], // nord7 teal
    ],
    mesh: [0.85, 0.87, 0.91], // nord4
};

pub const THEMES: [&Theme; 2] = [&LIGHT, &DARK];

impl Theme {
    /// The theme called `name`: `light` or `dark`.
    pub fn by_name(name: &str) -> Result<&'static Theme, String> {
        THEMES
            .into_iter()
            .find(|t| t.name == name)
            .ok_or(format!("unknown theme {name}: light or dark"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mean(c: [f32; 3]) -> f32 {
        c.iter().sum::<f32>() / 3.0
    }

    // 2. the ramp, the tints and the mesh color must read on the background,
    //    in every theme
    #[test]
    fn readable_on_the_background() {
        assert!(mean(LIGHT.bg) > 0.9, "the default is no longer a light background");
        for t in THEMES {
            let far = |c: [f32; 3]| (mean(c) - mean(t.bg)).abs() > 0.2;
            assert!(t.ramp.iter().all(|&c| far(c)), "{}: ramp too close to the background", t.name);
            assert!(t.palette.iter().all(|&c| far(c)), "{}: tint too close to the background", t.name);
            assert!(far(t.mesh), "{}: mesh color too close to the background", t.name);
        }
    }
}
