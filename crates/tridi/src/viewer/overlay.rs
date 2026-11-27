//! What's drawn over the scene with egui: the legend, the keys, the hint,
//! the picks panel and the picks' markers.

// ========================================== Imports ========================================== {{{

use three_d::egui;

use super::{
    KEYS,
    load::{File, State},
};

// }}}

// ========================================= Constants ========================================= {{{

/// The legend's dimming of a file turned off, and the panels' see-through.
const OFF_ALPHA: f32 = 0.35;

const PANEL_ALPHA: f32 = 0.75;

// }}}

// ========================================== Overlay ========================================== {{{

/// What's drawn over the scene; egui gets no input, the window hit-tests the
/// legend's rows itself. Top left, with `legend`: each file with its tint,
/// its size and its points' size if not the others', the ones turned off,
/// still loading or failed dimmed, the one under the cursor (`hover`) in
/// bold. Top right, with `help`, the keys; else a hint bottom left that H
/// shows them. Returns the legend's rows and their file, empty without it.
pub(crate) fn overlay(
    ui: &egui::Ui,
    files: &[File],
    visible: &[bool],
    sizes: &[f32],
    hover: Option<usize>,
    legend: bool,
    help: bool,
) -> Vec<(egui::Rect, usize)> {
    let panel = |id: &str, at: egui::Align2, off: [f32; 2], add: &mut dyn FnMut(&mut egui::Ui)| {
        egui::Area::new(egui::Id::new(id))
            .anchor(at, off)
            .interactable(false)
            .show(ui.ctx(), |ui| {
                // see-through, so the scene shows under it
                let fill = ui.visuals().window_fill.gamma_multiply(PANEL_ALPHA);
                egui::Frame::popup(ui.style()).fill(fill).show(ui, add)
            });
    };
    if help {
        panel("help", egui::Align2::RIGHT_TOP, [-12.0, 12.0], &mut |ui| {
            // the help's lines: a group's name alone, then its keys, two
            // spaces or more between a key and what it does
            egui::Grid::new("keys").spacing([16.0, 4.0]).show(ui, |ui| {
                for (i, l) in KEYS.lines().skip(1).map(str::trim).enumerate() {
                    if let Some((k, d)) = l.split_once("  ") {
                        ui.strong(k);
                        ui.label(d.trim());
                    } else {
                        let top = if i == 0 { 0.0 } else { 8.0 };
                        ui.vertical(|ui| {
                            ui.add_space(top);
                            ui.label(egui::RichText::new(l).small().weak());
                        });
                    }
                    ui.end_row();
                }
            });
        });
    } else {
        let loading = files
            .iter()
            .any(|f| matches!(f.state, State::Loading | State::Shown { loading: true, .. }));
        egui::Area::new(egui::Id::new("hint"))
            .anchor(egui::Align2::LEFT_BOTTOM, [12.0, -12.0])
            .interactable(false)
            .show(ui.ctx(), |ui| ui.weak(if loading { "loading…   H  keys" } else { "H  keys" }));
    }
    let mut rows = vec![];
    if !legend {
        return rows;
    }
    panel("legend", egui::Align2::LEFT_TOP, [12.0, 12.0], &mut |ui| {
        egui::Grid::new("files").min_col_width(0.0).spacing([10.0, 4.0]).show(ui, |ui| {
            for (i, f) in files.iter().enumerate() {
                let (on, tint, count) = match &f.state {
                    State::Shown {
                        layer,
                        tint,
                        count,
                        unit,
                        loading,
                        skipped,
                        ..
                    } => {
                        let more = match (loading, skipped) {
                            (true, _) => ", loading…".to_owned(),
                            (false, 0) => String::new(),
                            (false, n) => format!(", {n} solids skipped"),
                        };
                        let size = match sizes.get(*layer) {
                            Some(s) if (s - 1.0).abs() > 0.01 => format!(", points ×{s:.1}"),
                            _ => String::new(),
                        };
                        let on = visible.get(*layer).copied().unwrap_or(true);
                        (on, *tint, format!("{count} {unit}{size}{more}"))
                    }
                    State::Loading => (false, None, "loading…".to_owned()),
                    State::Failed => (false, None, "failed".to_owned()),
                };
                let alpha = if on { 1.0 } else { OFF_ALPHA };
                let fg = ui.visuals().text_color().gamma_multiply(alpha);
                let text = |t: String| egui::RichText::new(t).color(fg);
                let (r, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                if let Some(c) = tint {
                    // the palette is written to the screen as is: sRGB
                    let [r8, g8, b8] = c.map(byte);
                    let c = egui::Color32::from_rgb(r8, g8, b8).gamma_multiply(alpha);
                    ui.painter().rect_filled(r, 2.0, c);
                } else {
                    // own colors or a mesh: an empty square
                    ui.painter()
                        .rect_stroke(r, 2.0, egui::Stroke::new(1.0_f32, fg), egui::StrokeKind::Inside);
                }
                let name = text(f.name.clone());
                ui.label(if hover == Some(i) { name.strong().underline() } else { name });
                let last = ui.label(text(count));
                // half the grid's spacing above and below: no gap between rows
                rows.push((r.union(last.rect).expand2(egui::vec2(0.0, 2.0)), i));
                ui.end_row();
            }
        });
    });
    rows
}

/// What the picks panel shows: its rows in view (place in the picks, the
/// row's cells, the file's name if several, whether its file is shown) and
/// how many picks are out of view above and below.
#[derive(Default)]
pub(crate) struct PanelView {
    pub(crate) rows: Vec<(usize, [String; 6], Option<String>, bool)>,
    pub(crate) above: usize,
    pub(crate) below: usize,
}

/// The picks panel, bottom right, while there are picks and it's open
/// (`view` empty otherwise): each pick in view with its number, index, x y z
/// (right-aligned) and file, those of hidden files dimmed, the one under the
/// cursor (`over`) in bold, and how many are scrolled out; below, what the
/// last copy did (`status`), or how to copy. Returns the rows and their pick.
pub(crate) fn picks_panel(ui: &egui::Ui, view: &PanelView, over: Option<usize>, status: Option<&str>) -> Vec<(egui::Rect, usize)> {
    let mut rows = vec![];
    if view.rows.is_empty() {
        return rows;
    }
    egui::Area::new(egui::Id::new("picks panel"))
        .anchor(egui::Align2::RIGHT_BOTTOM, [-12.0, -12.0])
        .interactable(false)
        .show(ui.ctx(), |ui| {
            let fill = ui.visuals().window_fill.gamma_multiply(PANEL_ALPHA);
            egui::Frame::popup(ui.style()).fill(fill).show(ui, |ui| {
                if view.above > 0 {
                    ui.weak(format!("↑ {} more", view.above));
                }
                egui::Grid::new("picks").min_col_width(0.0).spacing([10.0, 4.0]).show(ui, |ui| {
                    for (n, cells, name, on) in &view.rows {
                        let fg = ui.visuals().text_color().gamma_multiply(if *on { 1.0 } else { OFF_ALPHA });
                        let text = |t: &str| {
                            let t = egui::RichText::new(t).color(fg);
                            if over == Some(*n) { t.strong().underline() } else { t }
                        };
                        let mut last = ui.label(text(&cells[0]).strong());
                        let first = last.rect;
                        last = ui.label(text(&cells[1]));
                        for c in &cells[2..] {
                            // coordinates and distance right-aligned, so their digits line up
                            last = ui
                                .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(text(c).monospace()))
                                .inner;
                        }
                        if let Some(name) = name {
                            last = ui.label(text(name));
                        }
                        rows.push((first.union(last.rect).expand2(egui::vec2(0.0, 2.0)), *n));
                        ui.end_row();
                    }
                });
                if view.below > 0 {
                    ui.weak(format!("↓ {} more", view.below));
                }
                ui.weak(status.unwrap_or("click a row or Ctrl+C to copy"));
            });
        });
    rows
}

/// The picks' markers, over the scene and under the panels: a ring and a dot, outlined so they
/// read on any color, and the pick's number; the one whose panel row is
/// under the cursor (`over`, 0-based) bigger, in the accent color.
pub(crate) fn draw_marks(ui: &egui::Ui, marks: &[(usize, (f32, f32))], over: Option<usize>) {
    let painter = ui
        .ctx()
        .layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("picks")));
    let (text, halo, accent) = (
        ui.visuals().strong_text_color(),
        ui.visuals().window_fill,
        ui.visuals().selection.bg_fill,
    );
    for &(n, (x, y)) in marks {
        let c = egui::pos2(x, y);
        let lit = over == n.checked_sub(1);
        let (fg, r, w) = if lit { (accent, 10.0, 2.5_f32) } else { (text, 6.0, 1.5_f32) };
        painter.circle_stroke(c, r, egui::Stroke::new(w + 2.0, halo));
        painter.circle_stroke(c, r, egui::Stroke::new(w, fg));
        painter.circle_filled(c, 1.5, fg);
        let at = c + egui::vec2(r + 2.0, -r);
        let font = egui::FontId::proportional(13.0);
        // the number outlined too: drawn in the halo color around, then on top
        for (dx, dy) in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)] {
            painter.text(
                at + egui::vec2(dx, dy),
                egui::Align2::LEFT_BOTTOM,
                n.to_string(),
                font.clone(),
                halo,
            );
        }
        painter.text(at, egui::Align2::LEFT_BOTTOM, n.to_string(), font, fg);
    }
}

/// The file whose legend row is under the cursor.
pub(crate) fn row(rows: &[(egui::Rect, usize)], cursor: Option<(f32, f32)>) -> Option<usize> {
    let (x, y) = cursor?;
    rows.iter().find(|(r, _)| r.contains(egui::pos2(x, y))).map(|(_, i)| *i)
}

/// A 0..1 channel as a byte.
fn byte(v: f32) -> u8 {
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to 0..255 first"
    )]
    let b = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    b
}

// }}}
