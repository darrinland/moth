//! Stage 1: look at a batch of flames and pick one. The randomness controls live in a
//! drawer that is closed until asked for.

use eframe::egui::{self, Color32, CornerRadius, Rect, Sense, StrokeKind, pos2, vec2};
use moth_core::flame::{Variation, io};
use moth_core::library;

use crate::app::{MothApp, Settings, Stage};
use crate::history::STAR_GOLD;
use crate::icons;

pub fn ui(app: &mut MothApp, ui: &mut egui::Ui) {
    if app.show_randomness {
        egui::Panel::right("randomness")
            .default_size(260.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| settings_ui(ui, &mut app.settings));
            });
    }
    egui::CentralPanel::default().show(ui, |ui| gallery(app, ui));
}

fn gallery(app: &mut MothApp, ui: &mut egui::Ui) {
    let (mut clicked, mut opened, mut starred) = (None, None, None);
    egui::ScrollArea::vertical().show(ui, |ui| {
        let spacing = 12.0;
        let avail = ui.available_width();
        let cols = (((avail + spacing) / (250.0 + spacing)).floor() as usize).max(1);
        let w = (avail - spacing * (cols as f32 - 1.0)) / cols as f32;
        let h = w * 0.75;
        egui::Grid::new("gallery")
            .spacing([spacing, spacing])
            .show(ui, |ui| {
                for (i, card) in app.cards.iter_mut().enumerate() {
                    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
                    match &card.texture {
                        Some(tex) => {
                            ui.painter().image(
                                tex.id(),
                                rect,
                                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                                Color32::WHITE,
                            );
                        }
                        None => {
                            ui.painter().rect_filled(
                                rect,
                                CornerRadius::same(4),
                                Color32::from_gray(24),
                            );
                            ui.put(
                                Rect::from_center_size(rect.center(), vec2(20.0, 20.0)),
                                egui::Spinner::new(),
                            );
                        }
                    }
                    let stroke = if i == app.selected {
                        egui::Stroke::new(3.0, Color32::from_rgb(255, 190, 70))
                    } else if resp.hovered() {
                        egui::Stroke::new(1.5, Color32::from_gray(160))
                    } else {
                        egui::Stroke::NONE
                    };
                    ui.painter().rect_stroke(
                        rect,
                        CornerRadius::same(4),
                        stroke,
                        StrokeKind::Inside,
                    );
                    if let Some(genome) = &card.genome {
                        let hash = card
                            .hash
                            .get_or_insert_with(|| library::content_hash(&io::to_xml(genome)));
                        let on = app.favorite_hashes.contains(hash.as_str());
                        let spot = Rect::from_min_size(
                            rect.right_top() + vec2(-30.0, 6.0),
                            vec2(24.0, 24.0),
                        );
                        let star = ui
                            .interact(spot, resp.id.with("star"), Sense::click())
                            .on_hover_text(if on { "Unstar" } else { "Star" });
                        if on || resp.hovered() || star.hovered() {
                            ui.painter().circle_filled(
                                spot.center(),
                                12.0,
                                Color32::from_black_alpha(150),
                            );
                            ui.painter().text(
                                spot.center(),
                                egui::Align2::CENTER_CENTER,
                                icons::STAR,
                                egui::FontId::proportional(16.0),
                                if on {
                                    STAR_GOLD
                                } else {
                                    Color32::from_gray(200)
                                },
                            );
                        }
                        if star.clicked() {
                            starred = Some(i);
                        }
                    }
                    // The star is drawn after the card, so it wins clicks over its own corner.
                    if resp.clicked() {
                        clicked = Some(i);
                    }
                    if resp.double_clicked() && card.genome.is_some() {
                        opened = Some(i);
                    }
                    if (i + 1) % cols == 0 {
                        ui.end_row();
                    }
                }
            });
        if app.cards.is_empty() {
            ui.weak("Nothing here yet. Generate a batch, or open a file.");
        }
    });
    if let Some(i) = starred {
        app.toggle_card_star(i);
    }
    if let Some(i) = clicked {
        app.select(i);
    }
    if let Some(i) = opened {
        app.select(i);
        app.go(Stage::Refine);
    }
}

fn settings_ui(ui: &mut egui::Ui, s: &mut Settings) {
    ui.heading("Randomness");
    ui.weak("Steers new batches and evolving.");
    ui.add_space(4.0);
    ui.label("Transforms");
    range_ui(ui, &mut s.xforms, 1..=8);
    ui.label("Variations per transform");
    range_ui(ui, &mut s.variations_per_xform, 1..=5);
    ui.add(egui::Slider::new(&mut s.wildness, 0.0..=1.0).text("wildness"));
    ui.add(egui::Slider::new(&mut s.final_chance, 0.0..=1.0).text("final transform"));
    ui.add_space(8.0);
    ui.separator();
    ui.heading("Allowed variations");
    ui.horizontal(|ui| {
        if ui.small_button("All").clicked() {
            s.allowed.fill(true);
        }
        if ui.small_button("None").clicked() {
            s.allowed.fill(false);
        }
        if ui.small_button("Invert").clicked() {
            s.allowed.iter_mut().for_each(|a| *a = !*a);
        }
    });
    let on = s.allowed.iter().filter(|a| **a).count();
    ui.weak(format!("{on} of {} enabled", Variation::ALL.len()));
    if on == 0 {
        ui.colored_label(
            Color32::LIGHT_RED,
            "none enabled: only `linear` will be used",
        );
    }
    for (v, allowed) in Variation::ALL.iter().zip(&mut s.allowed) {
        ui.checkbox(allowed, v.name());
    }
}

fn range_ui(
    ui: &mut egui::Ui,
    range: &mut (usize, usize),
    bounds: std::ops::RangeInclusive<usize>,
) {
    ui.horizontal(|ui| {
        ui.add(egui::DragValue::new(&mut range.0).range(*bounds.start()..=*bounds.end()));
        ui.label("to");
        ui.add(egui::DragValue::new(&mut range.1).range(*bounds.start()..=*bounds.end()));
    });
    if range.0 > range.1 {
        range.1 = range.0;
    }
}
