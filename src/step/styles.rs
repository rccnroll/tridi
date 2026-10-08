//! What monstertruck drops: the DATA section scanned for styles, so faces
//! and solids get the colors the file gives them.

use std::collections::HashMap;

/// The DATA section as `id -> (NAME, arguments)`, simple entities only
/// (complex ones, `#1 = ( A() B() );`, are relationships we don't need
/// here). monstertruck drops the style entities, so colors are read here.
pub(crate) type Ents<'a> = HashMap<u64, (&'a str, &'a str)>;

pub(crate) fn entities(text: &str) -> Ents<'_> {
    let data = text.find("DATA;").map_or(text, |i| &text[i + 5..]);
    let mut out = HashMap::new();
    let (mut start, mut quoted) = (0, false);
    for (i, c) in data.char_indices() {
        match c {
            '\'' => quoted = !quoted,
            ';' if !quoted => {
                let stmt = data[start..i].trim();
                start = i + 1;
                let Some((id, rest)) = stmt.strip_prefix('#').and_then(|s| s.split_once('=')) else {
                    continue;
                };
                let (Ok(id), Some((name, args))) = (id.trim().parse(), rest.split_once('(')) else {
                    continue;
                };
                let name = name.trim();
                if !name.is_empty() {
                    out.insert(id, (name, args.trim_end().strip_suffix(')').unwrap_or(args)));
                }
            }
            _ => {}
        }
    }
    out
}

/// The `#n` references in some arguments, in order, strings skipped.
fn refs(args: &str) -> Vec<u64> {
    let (mut out, mut quoted, mut it) = (Vec::new(), false, args.chars().peekable());
    while let Some(c) = it.next() {
        match c {
            '\'' => quoted = !quoted,
            '#' if !quoted => {
                let digits: String = std::iter::from_fn(|| it.next_if(char::is_ascii_digit)).collect();
                if let Ok(n) = digits.parse() {
                    out.push(n);
                }
            }
            _ => {}
        }
    }
    out
}

/// Item id -> color, from STYLED_ITEM (and OVER_RIDING_STYLED_ITEM). An item
/// is a face, a solid, or a representation, whose color then goes to its
/// items that have none of their own.
pub(crate) fn colors(ents: &Ents) -> HashMap<u64, [f32; 3]> {
    let mut direct = HashMap::new();
    for (name, args) in ents.values() {
        let r = refs(args);
        let item = match *name {
            "STYLED_ITEM" if !r.is_empty() => r.len() - 1,
            "OVER_RIDING_STYLED_ITEM" if r.len() >= 2 => r.len() - 2,
            _ => continue,
        };
        if let Some(c) = r[..item].iter().find_map(|&s| colour(ents, s, 0)) {
            direct.insert(r[item], c);
        }
    }
    let mut all = direct.clone();
    for (id, c) in &direct {
        if ents.get(id).is_some_and(|(name, _)| name.ends_with("REPRESENTATION")) {
            for item in refs(ents[id].1) {
                all.entry(item).or_insert(*c);
            }
        }
    }
    all
}

/// The surface color a style chain ends in (PRESENTATION_STYLE_ASSIGNMENT ->
/// SURFACE_STYLE_USAGE -> … -> COLOUR_RGB), curve and point styles skipped.
fn colour(ents: &Ents, id: u64, depth: u8) -> Option<[f32; 3]> {
    let (name, args) = ents.get(&id)?;
    match *name {
        "COLOUR_RGB" => {
            // after the name, which may hold commas
            let rgb: Vec<f32> = args.rsplit('\'').next()?.split(',').filter_map(|v| v.trim().parse().ok()).collect();
            (rgb.len() == 3).then(|| [rgb[0], rgb[1], rgb[2]])
        }
        "DRAUGHTING_PRE_DEFINED_COLOUR" => Some(match args.trim().trim_matches('\'') {
            "red" => [1.0, 0.0, 0.0],
            "green" => [0.0, 1.0, 0.0],
            "blue" => [0.0, 0.0, 1.0],
            "yellow" => [1.0, 1.0, 0.0],
            "magenta" => [1.0, 0.0, 1.0],
            "cyan" => [0.0, 1.0, 1.0],
            "black" => [0.0, 0.0, 0.0],
            "white" => [1.0, 1.0, 1.0],
            _ => return None,
        }),
        "CURVE_STYLE" | "POINT_STYLE" | "TEXT_STYLE" => None,
        _ if depth < 10 => refs(args).into_iter().find_map(|r| colour(ents, r, depth + 1)),
        _ => None,
    }
}

/// The faces of a solid's outer shell, ORIENTED_FACE resolved to the face.
pub(crate) fn outer_faces(ents: &Ents, solid: u64) -> Vec<u64> {
    let Some((_, args)) = ents.get(&solid) else { return Vec::new() };
    let Some(&shell) = refs(args).first() else { return Vec::new() };
    let Some((_, shell_args)) = ents.get(&shell) else {
        return Vec::new();
    };
    refs(shell_args)
        .into_iter()
        .map(|f| match ents.get(&f) {
            Some(("ORIENTED_FACE", a)) => refs(a).first().copied().unwrap_or(f),
            _ => f,
        })
        .collect()
}
