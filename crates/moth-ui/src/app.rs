use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};

use eframe::egui::{
    self, Color32, CornerRadius, Key, Rect, Sense, StrokeKind, TextureHandle, TextureOptions, pos2,
    vec2,
};
use moth_core::export::{ExportOptions, export, load_flames};
use moth_core::flame::{Genome, RandomSettings, Variation, io};
use moth_core::output::BitDepth;
use moth_core::{Image, RenderSpec};

use crate::worker::{Done, Job, Work, Worker};

const GALLERY_SIZE: usize = 10;
const PREVIEW_SIZE: (u32, u32) = (800, 600);

#[derive(Default)]
struct Card {
    genome: Option<Genome>,
    texture: Option<TextureHandle>,
    /// Bumped whenever the genome changes, so the preview knows to re-render.
    version: u64,
}

/// What the user can steer in the randomizer.
struct Settings {
    allowed: Vec<bool>,
    xforms: (usize, usize),
    variations_per_xform: (usize, usize),
    wildness: f64,
    final_chance: f64,
    /// How far "Evolve" drifts from the selected flame.
    drift: f32,
}

impl Settings {
    fn new() -> Self {
        let d = RandomSettings::default();
        Self {
            allowed: vec![true; Variation::ALL.len()],
            xforms: d.xforms,
            variations_per_xform: d.variations_per_xform,
            wildness: d.wildness,
            final_chance: d.final_xform_chance,
            drift: 0.3,
        }
    }

    fn to_random(&self) -> RandomSettings {
        RandomSettings {
            allowed: Variation::ALL
                .iter()
                .zip(&self.allowed)
                .filter(|(_, on)| **on)
                .map(|(v, _)| *v)
                .collect(),
            xforms: self.xforms,
            variations_per_xform: self.variations_per_xform,
            wildness: self.wildness,
            final_xform_chance: self.final_chance,
            size: (1024, 768),
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Format {
    Png8,
    Png16,
    Jpeg,
}

struct ExportDialog {
    open: bool,
    width: u32,
    height: u32,
    quality: f32,
    oversample: u32,
    denoise: f32,
    format: Format,
    jpeg_quality: u8,
    transparent: bool,
    seed: u64,
}

impl Default for ExportDialog {
    fn default() -> Self {
        Self {
            open: false,
            width: 3840,
            height: 2160,
            quality: 300.0,
            oversample: 2,
            denoise: 1.0,
            format: Format::Png8,
            jpeg_quality: 92,
            transparent: false,
            seed: 0,
        }
    }
}

pub struct MothApp {
    worker: Worker,
    cards: Vec<Card>,
    selected: usize,
    settings: Settings,
    next_seed: u64,
    preview: Option<TextureHandle>,
    /// The (slot, version) the current preview texture, or pending request, is for.
    preview_key: Option<(usize, u64)>,
    export: ExportDialog,
    status: String,
    messages_tx: Sender<String>,
    messages_rx: Receiver<String>,
    exports_running: usize,
}

impl MothApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let (messages_tx, messages_rx) = channel();
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() % 1_000_000)
            .unwrap_or(1);
        let mut app = Self {
            worker: Worker::spawn(cc.egui_ctx.clone()),
            cards: Vec::new(),
            selected: 0,
            settings: Settings::new(),
            next_seed: seed * 100,
            preview: None,
            preview_key: None,
            export: ExportDialog::default(),
            status: String::new(),
            messages_tx,
            messages_rx,
            exports_running: 0,
        };
        app.generate();
        app
    }

    fn take_seed(&mut self) -> u64 {
        self.next_seed += 1;
        self.next_seed
    }

    fn generate(&mut self) {
        let generation = self.worker.next_gallery_generation();
        let settings = self.settings.to_random();
        self.cards = (0..GALLERY_SIZE).map(|_| Card::default()).collect();
        for slot in 0..GALLERY_SIZE {
            let seed = self.take_seed();
            self.worker.submit(Job {
                slot,
                generation,
                work: Work::Fresh {
                    seed,
                    settings: settings.clone(),
                },
            });
        }
        self.selected = 0;
        self.preview_key = None;
        self.preview = None;
    }

    /// Slot 0 keeps the selected flame; the rest are mutations of it.
    fn evolve(&mut self) {
        let Some(parent) = self.cards.get(self.selected).and_then(|c| c.genome.clone()) else {
            return;
        };
        let keep_texture = self.cards[self.selected].texture.clone();
        let generation = self.worker.next_gallery_generation();
        let settings = self.settings.to_random();
        self.cards = (0..GALLERY_SIZE).map(|_| Card::default()).collect();
        self.cards[0] = Card {
            genome: Some(parent.clone()),
            texture: keep_texture,
            version: 1,
        };
        for slot in 1..GALLERY_SIZE {
            let seed = self.take_seed();
            self.worker.submit(Job {
                slot,
                generation,
                work: Work::Evolve {
                    parent: parent.clone(),
                    amount: self.settings.drift,
                    seed,
                    settings: settings.clone(),
                },
            });
        }
        self.selected = 0;
        self.preview_key = None;
    }

    fn open_file(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Flames and Moth images", &["flame", "png", "jpg", "jpeg"])
            .pick_file()
        else {
            return;
        };
        match load_flames(&path) {
            Ok(flames) => {
                let warnings: Vec<_> = flames
                    .iter()
                    .flat_map(|f| f.ignored.iter().cloned())
                    .collect();
                let genomes: Vec<Genome> = flames.into_iter().map(|f| f.genome).take(60).collect();
                let generation = self.worker.next_gallery_generation();
                self.cards = genomes
                    .iter()
                    .map(|g| Card {
                        genome: Some(g.clone()),
                        texture: None,
                        version: 1,
                    })
                    .collect();
                for (slot, genome) in genomes.into_iter().enumerate() {
                    self.worker.submit(Job {
                        slot,
                        generation,
                        work: Work::Thumb { genome },
                    });
                }
                self.selected = 0;
                self.preview_key = None;
                self.preview = None;
                self.status = if warnings.is_empty() {
                    format!("Opened {}", path.display())
                } else {
                    let mut w = warnings;
                    w.sort();
                    w.dedup();
                    format!(
                        "Opened {} (ignored unsupported: {})",
                        path.display(),
                        w.join(", ")
                    )
                };
            }
            Err(e) => self.status = format!("Could not open {}: {e}", path.display()),
        }
    }

    fn save_flame(&mut self) {
        let Some(genome) = self
            .cards
            .get(self.selected)
            .and_then(|c| c.genome.as_ref())
        else {
            return;
        };
        let name = if genome.name.is_empty() {
            "moth".to_string()
        } else {
            genome.name.clone()
        };
        let Some(path) = rfd::FileDialog::new()
            .set_file_name(format!("{name}.flame"))
            .add_filter("Flame", &["flame"])
            .save_file()
        else {
            return;
        };
        self.status = match std::fs::write(&path, io::to_xml(genome)) {
            Ok(()) => format!("Saved {}", path.display()),
            Err(e) => format!("Could not save {}: {e}", path.display()),
        };
    }

    fn start_export(&mut self) {
        let Some(genome) = self.cards.get(self.selected).and_then(|c| c.genome.clone()) else {
            return;
        };
        let e = &self.export;
        let (ext, filter) = match e.format {
            Format::Jpeg => ("jpg", "JPEG"),
            _ => ("png", "PNG"),
        };
        let name = if genome.name.is_empty() {
            "moth".to_string()
        } else {
            genome.name.clone()
        };
        let Some(path) = rfd::FileDialog::new()
            .set_file_name(format!("{name}.{ext}"))
            .add_filter(filter, &[ext])
            .save_file()
        else {
            return;
        };
        let opts = ExportOptions {
            spec: RenderSpec {
                width: e.width,
                height: e.height,
                quality: e.quality,
                transparent: e.transparent && e.format != Format::Jpeg,
                seed: e.seed,
                oversample: e.oversample,
                denoise: e.denoise,
            },
            depth: if e.format == Format::Png16 {
                BitDepth::Sixteen
            } else {
                BitDepth::Eight
            },
            jpeg_quality: e.jpeg_quality,
            ..ExportOptions::default()
        };
        self.exports_running += 1;
        self.status = format!("Rendering {}…", path.display());
        let tx = self.messages_tx.clone();
        let ctx_repaint = move || {};
        let _ = ctx_repaint;
        std::thread::spawn(move || {
            let msg = match export(&genome, &opts, &path) {
                Ok(()) => format!("Wrote {}", display_path(&path)),
                Err(e) => format!("Export failed: {e}"),
            };
            let _ = tx.send(msg);
        });
    }

    fn handle_done(&mut self, ctx: &egui::Context, done: Done) {
        let current = if done.preview {
            &self.worker.generations.preview
        } else {
            &self.worker.generations.gallery
        };
        if done.generation < current.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        let texture = ctx.load_texture(
            format!("img-{}", done.genome.name),
            to_color_image(&done.image),
            TextureOptions::LINEAR,
        );
        if done.preview {
            self.preview = Some(texture);
        } else if let Some(card) = self.cards.get_mut(done.slot) {
            card.texture = Some(texture);
            if card.genome.is_none() {
                card.genome = Some(done.genome);
                card.version += 1;
            }
        }
    }

    fn request_preview_if_needed(&mut self) {
        let Some(card) = self.cards.get(self.selected) else {
            return;
        };
        let Some(genome) = &card.genome else { return };
        let key = (self.selected, card.version);
        if self.preview_key != Some(key) {
            self.preview_key = Some(key);
            let generation = self.worker.next_preview_generation();
            self.worker.submit(Job {
                slot: self.selected,
                generation,
                work: Work::Preview {
                    genome: genome.clone(),
                    size: PREVIEW_SIZE,
                },
            });
        }
    }
}

impl eframe::App for MothApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        while let Ok(done) = self.worker.rx.try_recv() {
            self.handle_done(&ctx, done);
        }
        while let Ok(msg) = self.messages_rx.try_recv() {
            self.exports_running = self.exports_running.saturating_sub(1);
            self.status = msg;
        }
        if self.exports_running > 0 {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        self.request_preview_if_needed();

        // Keyboard: Space generates, E evolves (not while typing in a field).
        if !ctx.egui_wants_keyboard_input() {
            if ctx.input(|i| i.key_pressed(Key::Space)) {
                self.generate();
            }
            if ctx.input(|i| i.key_pressed(Key::E)) {
                self.evolve();
            }
        }

        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                if ui.button("✨ Generate new  (Space)").clicked() {
                    self.generate();
                }
                ui.separator();
                let has = self
                    .cards
                    .get(self.selected)
                    .is_some_and(|c| c.genome.is_some());
                if ui
                    .add_enabled(has, egui::Button::new("🧬 Evolve selected  (E)"))
                    .clicked()
                {
                    self.evolve();
                }
                ui.label("drift");
                ui.add(egui::Slider::new(&mut self.settings.drift, 0.0..=1.0).fixed_decimals(2));
                ui.separator();
                if ui.button("📂 Open…").clicked() {
                    self.open_file();
                }
                if ui
                    .add_enabled(has, egui::Button::new("💾 Save flame…"))
                    .clicked()
                {
                    self.save_flame();
                }
                if ui
                    .add_enabled(has, egui::Button::new("🖼 Export image…"))
                    .clicked()
                {
                    self.export.open = true;
                }
            });
        });

        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                if self.exports_running > 0 {
                    ui.spinner();
                }
                ui.label(&self.status);
            });
        });

        egui::Panel::left("settings")
            .default_size(250.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.settings_ui(ui));
            });

        egui::Panel::right("detail")
            .default_size(420.0)
            .show(ui, |ui| self.detail_ui(ui));

        egui::CentralPanel::default().show(ui, |ui| self.gallery_ui(ui));

        self.export_window(&ctx);
    }
}

impl MothApp {
    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        let s = &mut self.settings;
        ui.heading("Randomness");
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

    fn detail_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Selected");
        let Some(card) = self.cards.get(self.selected) else {
            return;
        };
        match &self.preview {
            Some(tex) if self.preview_key == Some((self.selected, card.version)) => {
                let width = ui.available_width();
                let height = width * PREVIEW_SIZE.1 as f32 / PREVIEW_SIZE.0 as f32;
                ui.add(
                    egui::Image::new(tex)
                        .fit_to_exact_size(vec2(width, height))
                        .corner_radius(4),
                );
            }
            _ => {
                let width = ui.available_width();
                let (rect, _) = ui.allocate_exact_size(vec2(width, width * 0.75), Sense::hover());
                ui.painter()
                    .rect_filled(rect, CornerRadius::same(4), Color32::from_gray(24));
                ui.put(
                    Rect::from_center_size(rect.center(), vec2(24.0, 24.0)),
                    egui::Spinner::new(),
                );
            }
        }
        if let Some(g) = &card.genome {
            ui.add_space(6.0);
            ui.label(egui::RichText::new(&g.name).strong());
            ui.weak(format!(
                "{} transforms{} · {} variations in use",
                g.xforms.len(),
                if g.final_xform.is_some() {
                    " + final"
                } else {
                    ""
                },
                g.xforms
                    .iter()
                    .chain(&g.final_xform)
                    .flat_map(|x| x.variations.iter().map(|(v, _)| *v))
                    .collect::<std::collections::HashSet<_>>()
                    .len()
            ));
        }
    }

    fn gallery_ui(&mut self, ui: &mut egui::Ui) {
        let mut clicked = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            let spacing = 10.0;
            let avail = ui.available_width();
            let cols = (((avail + spacing) / (210.0 + spacing)).floor() as usize).max(1);
            let w = (avail - spacing * (cols as f32 - 1.0)) / cols as f32;
            let h = w * 0.75;
            egui::Grid::new("gallery")
                .spacing([spacing, spacing])
                .show(ui, |ui| {
                    for (i, card) in self.cards.iter().enumerate() {
                        let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
                        let painter = ui.painter();
                        match &card.texture {
                            Some(tex) => {
                                painter.image(
                                    tex.id(),
                                    rect,
                                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                                    Color32::WHITE,
                                );
                            }
                            None => {
                                painter.rect_filled(
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
                        let stroke = if i == self.selected {
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
                        if resp.clicked() {
                            clicked = Some(i);
                        }
                        if (i + 1) % cols == 0 {
                            ui.end_row();
                        }
                    }
                });
        });
        if let Some(i) = clicked {
            self.selected = i;
        }
    }

    fn export_window(&mut self, ctx: &egui::Context) {
        let mut open = self.export.open;
        let mut go = false;
        egui::Window::new("Export image")
            .open(&mut open)
            .resizable(false)
            .show(ctx, |ui| {
                let e = &mut self.export;
                ui.horizontal(|ui| {
                    for (label, w, h) in [
                        ("1080p", 1920, 1080),
                        ("4K", 3840, 2160),
                        ("Square 4000", 4000, 4000),
                        ("Poster 8000×6000", 8000, 6000),
                    ] {
                        if ui.small_button(label).clicked() {
                            (e.width, e.height) = (w, h);
                        }
                    }
                });
                egui::Grid::new("export-grid")
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
                            ui.end_row();
                        } else {
                            ui.label("Background");
                            ui.checkbox(&mut e.transparent, "transparent");
                            ui.end_row();
                        }
                        ui.label("Render seed");
                        ui.add(egui::DragValue::new(&mut e.seed));
                        ui.end_row();
                    });
                let samples = e.quality as f64 * e.width as f64 * e.height as f64;
                let hi_px = e.width as u64 * e.height as u64 * (e.oversample * e.oversample) as u64;
                let bands = hi_px.div_ceil(16_000_000).max(1);
                ui.weak(format!(
                    "{:.1} billion samples{}",
                    samples / 1e9,
                    if bands > 1 {
                        format!(" · rendered in {bands} bands to save memory (slower)")
                    } else {
                        String::new()
                    }
                ));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Export…").clicked() {
                        go = true;
                    }
                    ui.weak(
                        "The flame is embedded in the file, so it can be reopened and re-rendered.",
                    );
                });
            });
        self.export.open = open;
        if go {
            self.export.open = false;
            self.start_export();
        }
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

fn to_color_image(img: &Image) -> egui::ColorImage {
    let bytes: Vec<u8> = img
        .rgba
        .iter()
        .map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
        .collect();
    egui::ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &bytes)
}

fn display_path(p: &PathBuf) -> String {
    p.display().to_string()
}
