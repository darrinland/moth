//! Icons drawn from the Phosphor Regular icon font (MIT, see `assets/fonts/`).
//!
//! The font is bundled, so no icon can go missing on a machine whose system fonts
//! lack emoji. Codepoints come from the same font file.

use eframe::egui;

const FONT: &[u8] = include_bytes!("../assets/fonts/Phosphor-Regular.ttf");

pub const ARROW_LEFT: &str = "\u{E058}";
pub const ARROW_RIGHT: &str = "\u{E06C}";
pub const ARROW_U_UP_LEFT: &str = "\u{E08A}";
pub const ARROW_U_UP_RIGHT: &str = "\u{E08C}";
pub const ARROWS_LEFT_RIGHT: &str = "\u{E0A0}";
pub const CARET_LEFT: &str = "\u{E138}";
pub const CARET_RIGHT: &str = "\u{E13A}";
pub const CHECK_CIRCLE: &str = "\u{E184}";
pub const DICE_FIVE: &str = "\u{E1EE}";
pub const DNA: &str = "\u{E924}";
pub const HOURGLASS: &str = "\u{E2B2}";
pub const MINUS: &str = "\u{E32A}";
pub const PLUS: &str = "\u{E3D4}";
pub const SPARKLE: &str = "\u{E6A2}";
pub const X: &str = "\u{E4F6}";
pub const X_CIRCLE: &str = "\u{E4F8}";

/// Adds the icon font as a fallback for proportional text, so icons mix into labels.
pub fn install(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts
        .font_data
        .insert("phosphor".into(), egui::FontData::from_static(FONT).into());
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .push("phosphor".into());
    ctx.set_fonts(fonts);
}
