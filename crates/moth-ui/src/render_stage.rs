//! Stage 3: choose how big and in what format, and queue the render. A framing preview
//! shows exactly what will be cropped for the chosen shape.

use eframe::egui::{self, Color32, CornerRadius, Rect, Sense, pos2, vec2};

use crate::app::{MothApp, Preview};
use crate::exporter::{Exporter, State};
use crate::icons;
use crate::worker::PreviewSpec;

#[derive(Clone, Copy, PartialEq)]
pub enum Format {
    Png8,
    Png16,
    Jpeg,
}

pub struct RenderStage {
    pub width: u32,
    pub height: u32,
    pub quality: f32,
    pub oversample: u32,
    pub denoise: f32,
    pub format: Format,
    pub jpeg_quality: u8,
    pub transparent: bool,
    pub seed: u64,
    /// The shape the framing preview was last rendered for.
    preview_for: Option<(u32, u32)>,
}

impl RenderStage {
    pub fn new() -> Self {
        Self {
            width: 3840,
            height: 2160,
            quality: 300.0,
            oversample: 2,
            denoise: 1.0,
            format: Format::Png8,
            jpeg_quality: 92,
            transparent: false,
            seed: 0,
            preview_for: None,
        }
    }

    /// Forces the framing preview to be redrawn (the flame may have changed).
    pub fn invalidate(&mut self) {
        self.preview_for = None;
    }
}

pub fn ui(app: &mut MothApp, ui: &mut egui::Ui) {
    // Keep the framing preview in step with the chosen shape.
    let shape = (app.render.width, app.render.height);
    if app.render.preview_for != Some(shape) {
        if let Some(g) = app.cards.get(app.selected).and_then(|c| c.genome.clone()) {
            let aspect = shape.1 as f32 / shape.0.max(1) as f32;
            app.preview
                .request(&app.worker, &g, PreviewSpec::fine(720, aspect));
            app.render.preview_for = Some(shape);
        }
    }

    egui::Panel::right("render-options")
        .default_size(380.0)
        .show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                options(ui, &mut app.render);
                ui.add_space(10.0);
                ui.separator();
                queue(ui, &app.exporter);
            });
        });

    egui::CentralPanel::default().show(ui, |ui| frame_preview(ui, &app.preview, &app.render));
}

fn frame_preview(ui: &mut egui::Ui, preview: &Preview, r: &RenderStage) {
    let aspect = r.height as f32 / r.width.max(1) as f32;
    let avail = ui.available_size();
    let (mut w, mut h) = (avail.x, avail.x * aspect);
    if h > avail.y - 24.0 {
        (h, w) = (
            (avail.y - 24.0).max(1.0),
            (avail.y - 24.0).max(1.0) / aspect,
        );
    }
    let (rect, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    match &preview.texture {
        Some(tex) => {
            ui.painter().image(
                tex.id(),
                rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        None => {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(4), Color32::from_gray(24));
        }
    }
    if preview.pending {
        ui.put(
            Rect::from_min_size(rect.left_top() + vec2(8.0, 8.0), vec2(18.0, 18.0)),
            egui::Spinner::new(),
        );
    }
    ui.add_space(4.0);
    ui.weak(format!(
        "{} × {} px · {:.1} megapixels",
        r.width,
        r.height,
        r.width as f64 * r.height as f64 / 1e6
    ));
}

fn options(ui: &mut egui::Ui, e: &mut RenderStage) {
    ui.heading("Output");
    ui.horizontal_wrapped(|ui| {
        for (label, w, h) in [
            ("1080p", 1920, 1080),
            ("4K", 3840, 2160),
            ("Square", 4000, 4000),
            ("Poster", 8000, 6000),
            ("Phone", 1290, 2796),
        ] {
            if ui.small_button(label).clicked() {
                (e.width, e.height) = (w, h);
            }
        }
    });
    egui::Grid::new("render-grid")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            ui.label("Size");
            ui.horizontal(|ui| {
                ui.add(
                    egui::DragValue::new(&mut e.width)
                        .range(16..=30000)
                        .suffix(" px"),
                );
                ui.label("×");
                ui.add(
                    egui::DragValue::new(&mut e.height)
                        .range(16..=30000)
                        .suffix(" px"),
                );
            });
            ui.end_row();
            ui.label("Format");
            egui::ComboBox::from_id_salt("format")
                .selected_text(match e.format {
                    Format::Png8 => "PNG (8-bit)",
                    Format::Png16 => "PNG (16-bit)",
                    Format::Jpeg => "JPEG",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut e.format, Format::Png8, "PNG (8-bit)");
                    ui.selectable_value(&mut e.format, Format::Png16, "PNG (16-bit)");
                    ui.selectable_value(&mut e.format, Format::Jpeg, "JPEG");
                });
            ui.end_row();
            if e.format == Format::Jpeg {
                ui.label("JPEG quality");
                ui.add(egui::Slider::new(&mut e.jpeg_quality, 1..=100));
            } else {
                ui.label("Background");
                ui.checkbox(&mut e.transparent, "transparent");
            }
            ui.end_row();
        });

    ui.add_space(8.0);
    egui::CollapsingHeader::new("Detail")
        .default_open(true)
        .show(ui, |ui| {
            egui::Grid::new("detail-grid")
                .num_columns(2)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Quality");
                    ui.add(
                        egui::DragValue::new(&mut e.quality)
                            .range(1.0..=20000.0)
                            .speed(5.0)
                            .suffix(" samples/px"),
                    );
                    ui.end_row();
                    ui.label("Oversample");
                    ui.add(egui::Slider::new(&mut e.oversample, 1..=3));
                    ui.end_row();
                    ui.label("Noise filter");
                    ui.add(egui::Slider::new(&mut e.denoise, 0.0..=2.0).fixed_decimals(2));
                    ui.end_row();
                    ui.label("Render seed");
                    ui.add(egui::DragValue::new(&mut e.seed));
                    ui.end_row();
                });
        });
    let samples = e.quality as f64 * e.width as f64 * e.height as f64;
    let hi_px = e.width as u64 * e.height as u64 * (e.oversample * e.oversample) as u64;
    let bands = hi_px.div_ceil(16_000_000).max(1);
    ui.weak(format!(
        "{:.1} billion samples{}",
        samples / 1e9,
        if bands > 1 {
            format!(" · {bands} bands to save memory (slower)")
        } else {
            String::new()
        }
    ));
}

fn queue(ui: &mut egui::Ui, exporter: &Exporter) {
    ui.heading("Renders");
    if exporter.items.is_empty() {
        ui.weak("Finished and running renders show up here.");
    }
    for item in exporter.items.iter().rev() {
        ui.horizontal(|ui| {
            match &item.state {
                State::Queued => {
                    ui.label(icons::HOURGLASS);
                }
                State::Rendering => {
                    ui.spinner();
                }
                State::Done => {
                    ui.label(icons::CHECK_CIRCLE);
                }
                State::Failed(_) => {
                    ui.label(icons::X_CIRCLE);
                }
            }
            ui.vertical(|ui| {
                ui.label(
                    item.path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                );
                let detail = match &item.state {
                    State::Queued => format!("{} × {} · waiting", item.size.0, item.size.1),
                    State::Rendering => format!(
                        "{} × {} · {:.0}s so far",
                        item.size.0,
                        item.size.1,
                        item.elapsed().unwrap_or(0.0)
                    ),
                    State::Done => format!(
                        "{} × {} · took {:.0}s",
                        item.size.0,
                        item.size.1,
                        item.seconds.unwrap_or(0.0)
                    ),
                    State::Failed(e) => e.clone(),
                };
                ui.weak(detail);
            });
            if item.state == State::Done && ui.small_button("Show").clicked() {
                reveal(&item.path);
            }
        });
    }
}

fn reveal(path: &std::path::Path) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer")
        .arg("/select,")
        .arg(path)
        .spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    if let Some(dir) = path.parent() {
        let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
    }
}
