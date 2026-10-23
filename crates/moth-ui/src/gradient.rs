//! A basic gradient editor: a preview bar, a row of editable color stops, and a few
//! one-click operations. Stops are a view of the palette, not its source of truth:
//! they are re-read whenever the palette changes some other way (undo, randomize...).

use eframe::egui::{self, Color32, CornerRadius, Sense, vec2};
use moth_core::flame::{Genome, Palette};

use crate::icons;

const DEFAULT_STOPS: usize = 8;

#[derive(Default)]
pub struct GradientState {
    stops: Vec<[f32; 3]>,
    /// The palette the stops were last in step with.
    synced_to: Option<Palette>,
}

pub fn bar(ui: &mut egui::Ui, palette: &Palette, height: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let painter = ui.painter();
    const STEPS: usize = 160;
    let w = rect.width() / STEPS as f32;
    for i in 0..STEPS {
        let c = palette.sample(i as f64 / (STEPS - 1) as f64);
        let x0 = rect.left() + w * i as f32;
        let cell = egui::Rect::from_min_max(
            egui::pos2(x0, rect.top()),
            egui::pos2(x0 + w + 0.5, rect.bottom()),
        );
        painter.rect_filled(cell, CornerRadius::ZERO, to_color32(c));
    }
}

/// Returns true if the genome was changed.
pub fn editor(
    ui: &mut egui::Ui,
    g: &mut Genome,
    state: &mut GradientState,
    seed: &mut u64,
) -> bool {
    let mut changed = false;
    if state.synced_to.as_ref() != Some(&g.palette) || state.stops.is_empty() {
        state.stops = g.palette.sample_stops(state.stops.len().max(DEFAULT_STOPS));
        state.synced_to = Some(g.palette.clone());
    }

    bar(ui, &g.palette, 34.0);
    ui.add_space(6.0);

    // Color stops, evenly spaced around the loop.
    let mut stops_changed = false;
    ui.horizontal_wrapped(|ui| {
        for stop in &mut state.stops {
            stops_changed |= egui::color_picker::color_edit_button_rgb(ui, stop).changed();
        }
    });
    ui.horizontal(|ui| {
        ui.weak("Stops");
        if ui
            .add_enabled(
                state.stops.len() > 3,
                egui::Button::new(icons::MINUS).small(),
            )
            .clicked()
        {
            state.stops = g.palette.sample_stops(state.stops.len() - 1);
            stops_changed = true;
        }
        if ui
            .add_enabled(state.stops.len() < 12, egui::Button::new(icons::PLUS).small())
            .clicked()
        {
            state.stops = g.palette.sample_stops(state.stops.len() + 1);
            stops_changed = true;
        }
    });
    if stops_changed {
        g.palette = Palette::from_stops(&state.stops);
        state.synced_to = Some(g.palette.clone());
        changed = true;
    }

    ui.add_space(8.0);
    ui.horizontal_wrapped(|ui| {
        if ui
            .button(format!("{} Randomize", icons::DICE_FIVE))
            .clicked()
        {
            *seed += 1;
            g.palette = Palette::random_seeded(*seed);
            changed = true;
        }
        if ui
            .button(format!("{} Reverse", icons::ARROWS_LEFT_RIGHT))
            .clicked()
        {
            g.palette.reverse();
            changed = true;
        }
    });
    ui.horizontal(|ui| {
        ui.label("Hue");
        if ui.button(icons::CARET_LEFT).clicked() {
            g.palette.shift_hue(-0.04);
            changed = true;
        }
        if ui.button(icons::CARET_RIGHT).clicked() {
            g.palette.shift_hue(0.04);
            changed = true;
        }
        ui.add_space(12.0);
        ui.label("Slide");
        if ui.button(icons::CARET_LEFT).clicked() {
            g.palette.rotate(-1.0 / 32.0);
            changed = true;
        }
        if ui.button(icons::CARET_RIGHT).clicked() {
            g.palette.rotate(1.0 / 32.0);
            changed = true;
        }
    });

    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label("Background");
        changed |= egui::color_picker::color_edit_button_rgb(ui, &mut g.background).changed();
    });
    changed
}

fn to_color32(c: [f32; 3]) -> Color32 {
    Color32::from_rgb(
        (c[0] * 255.0) as u8,
        (c[1] * 255.0) as u8,
        (c[2] * 255.0) as u8,
    )
}
