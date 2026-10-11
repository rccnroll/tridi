//! What's drawn over the scene with egui: the legend, the keys, the hint.

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

/// What's drawn over the scene, none of it clickable (the keys toggle, the
/// mouse moves the view). Top left, with `legend`: each file with its key,
/// its tint and its size, the ones turned off, still loading or failed
/// dimmed. Top right, with
/// `help`, the keys; else a hint bottom left that H shows them.
pub(crate) fn overlay(ui: &egui::Ui, files: &[File], visible: &[bool], legend: bool, help: bool) {
    let panel = |id: &str, at: egui::Align2, off: [f32; 2], add: &dyn Fn(&mut egui::Ui)| {
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
        panel("help", egui::Align2::RIGHT_TOP, [-12.0, 12.0], &|ui| {
            // the help's lines: two spaces or more between key and what it does
            egui::Grid::new("keys").spacing([16.0, 4.0]).show(ui, |ui| {
                for (k, d) in KEYS.lines().skip(1).filter_map(|l| l.trim().split_once("  ")) {
                    ui.strong(k);
                    ui.label(d.trim());
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
    if !legend {
        return;
    }
    panel("legend", egui::Align2::LEFT_TOP, [12.0, 12.0], &|ui| {
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
                        (visible.get(*layer).copied().unwrap_or(true), *tint, format!("{count} {unit}{more}"))
                    }
                    State::Loading => (false, None, "loading…".to_owned()),
                    State::Failed => (false, None, "failed".to_owned()),
                };
                let alpha = if on { 1.0 } else { OFF_ALPHA };
                let fg = ui.visuals().text_color().gamma_multiply(alpha);
                let text = |t: String| egui::RichText::new(t).color(fg);
                ui.label(text(if i < 9 { format!("{}", i + 1) } else { String::new() }));
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
                ui.label(text(f.name.clone()));
                ui.label(text(count));
                ui.end_row();
            }
        });
    });
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
