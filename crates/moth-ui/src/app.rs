//! The app shell: a header with the three stages, the active stage in the middle and a
//! footer with that stage's primary action. State shared between stages lives here.

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use eframe::egui::{self, Color32, Key, RichText, TextureHandle, TextureOptions};
use moth_core::export::{ExportOptions, load_flames};
use moth_core::flame::{Genome, RandomSettings, Variation, io};
use moth_core::library::{Library, now};
use moth_core::output::BitDepth;
use moth_core::{Image, RenderSpec};

use crate::exporter::Exporter;
use crate::history::{self, HistoryState};
use crate::icons;
use crate::refine::RefineState;
use crate::render_stage::{Format, RenderStage};
use crate::worker::{Done, Job, PreviewSpec, Work, Worker};
use crate::{explore, refine, render_stage};

// Shortcut hints, named for the platform's command key. Plain text, so every font has them.
#[cfg(target_os = "macos")]
const UNDO_HINT: &str = "Cmd+Z";
#[cfg(target_os = "macos")]
const REDO_HINT: &str = "Cmd+Shift+Z";
#[cfg(not(target_os = "macos"))]
const UNDO_HINT: &str = "Ctrl+Z";
#[cfg(not(target_os = "macos"))]
const REDO_HINT: &str = "Ctrl+Shift+Z";

const GALLERY_SIZE: usize = 10;

/// Which top-level view is showing: making flames, or the library's history.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum View {
    Create,
    History,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Explore,
    Refine,
    Render,
}

#[derive(Default)]
pub struct Card {
    pub genome: Option<Genome>,
    pub texture: Option<TextureHandle>,
    /// Content hash of the genome's `.flame` XML, worked out the first time it is needed.
    pub hash: Option<String>,
}

/// What the user can steer in the randomizer.
pub struct Settings {
    pub allowed: Vec<bool>,
    pub xforms: (usize, usize),
    pub variations_per_xform: (usize, usize),
    pub wildness: f64,
    pub final_chance: f64,
    /// How far "Evolve" drifts from the selected flame.
    pub drift: f32,
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

/// The large image shown in Refine and Render. The previous picture stays up while a
/// new one renders, so edits never flash an empty frame.
#[derive(Default)]
pub struct Preview {
    pub texture: Option<TextureHandle>,
    pub pending: bool,
}

impl Preview {
    pub fn request(&mut self, worker: &Worker, genome: &Genome, spec: PreviewSpec) {
        let generation = worker.next_preview_generation();
        worker.submit(Job {
            slot: 0,
            generation,
            work: Work::Preview {
                genome: genome.clone(),
                spec,
            },
        });
        self.pending = true;
    }
}

/// Opens the library. `MOTH_DATA` overrides the folder. A self-test (`MOTH_SMOKE`) without
/// it keeps its library in its own folder, so it never writes to the user's library.
fn open_library() -> Result<Library, String> {
    let dir = std::env::var_os("MOTH_DATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("MOTH_SMOKE").map(|dir| PathBuf::from(dir).join("library")))
        .or_else(moth_core::paths::data_dir)
        .ok_or_else(|| "no data folder".to_string())?;
    Library::open(&dir).map_err(|e| e.to_string())
}

pub struct MothApp {
    pub worker: Worker,
    pub exporter: Exporter,
    pub cards: Vec<Card>,
    pub selected: usize,
    pub settings: Settings,
    pub next_seed: u64,
    pub stage: Stage,
    pub show_randomness: bool,
    pub refine: RefineState,
    pub render: RenderStage,
    pub preview: Preview,
    pub status: String,
    /// Every flame selected or edited is stored here. `None` if the library could not open.
    pub library: Option<Library>,
    pub history: HistoryState,
    /// Content hashes of the starred flames, for the stars on Explore cards.
    pub favorite_hashes: std::collections::HashSet<String>,
    pub view: View,
    /// The library id of the flame being refined; the parent of its next edit.
    pub refine_source: Option<i64>,
    smoke: Option<crate::smoke::Smoke>,
}

impl MothApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        icons::install(&cc.egui_ctx);
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() % 1_000_000)
            .unwrap_or(1);
        let (library, library_note) = match open_library() {
            Ok(library) => (Some(library), String::new()),
            Err(e) => (None, format!("Library unavailable: {e}")),
        };
        let mut app = Self {
            worker: Worker::spawn(cc.egui_ctx.clone()),
            exporter: Exporter::spawn(cc.egui_ctx.clone()),
            cards: Vec::new(),
            selected: 0,
            settings: Settings::new(),
            next_seed: seed * 100,
            stage: Stage::Explore,
            show_randomness: false,
            refine: RefineState::new(),
            render: RenderStage::new(),
            preview: Preview::default(),
            status: library_note,
            library,
            history: HistoryState::new(cc.egui_ctx.clone()),
            favorite_hashes: Default::default(),
            view: View::Create,
            refine_source: None,
            smoke: crate::smoke::Smoke::from_env(),
        };
        app.refresh_favorites();
        app.generate();
        app
    }

    /// Selects a gallery card and stores its flame in the library.
    pub fn select(&mut self, index: usize) {
        self.selected = index;
        if let Some(genome) = self.cards.get(index).and_then(|c| c.genome.clone()) {
            self.remember(&genome, None);
        }
    }

    /// Reads which flames are starred again. Call after any change to the stars.
    pub fn refresh_favorites(&mut self) {
        if let Some(library) = self.library.as_ref() {
            self.favorite_hashes = library.favorite_hashes().unwrap_or_default();
        }
    }

    /// Stars or unstars card `index`, storing its flame first if it is not in the library yet.
    pub fn toggle_card_star(&mut self, index: usize) {
        let Some(genome) = self.cards.get(index).and_then(|c| c.genome.clone()) else {
            return;
        };
        let xml = moth_core::flame::io::to_xml(&genome);
        let starred = self
            .favorite_hashes
            .contains(&moth_core::library::content_hash(&xml));
        if let Some(id) = self.remember(&genome, None)
            && let Some(library) = self.library.as_mut()
            && let Err(e) = library.set_favorite(id, !starred)
        {
            self.status = format!("Could not star the flame: {e}");
        }
        self.refresh_favorites();
        self.history.refresh();
    }

    /// Stores `genome` in the library (a repeat only bumps its last-selected time) and returns
    /// its id. A failure is reported in the status line and does not stop the app.
    fn remember(&mut self, genome: &Genome, parent: Option<i64>) -> Option<i64> {
        let library = self.library.as_mut()?;
        let xml = moth_core::flame::io::to_xml(genome);
        match library.record(&xml, &genome.name, parent, now()) {
            Ok(id) => Some(id),
            Err(e) => {
                self.status = format!("Could not save to the library: {e}");
                None
            }
        }
    }

    /// Stores the edits committed in Refine, each as a child of the version before it.
    fn record_refine_edits(&mut self) {
        for genome in std::mem::take(&mut self.refine.edits) {
            if let Some(id) = self.remember(&genome, self.refine_source) {
                self.refine_source = Some(id);
            }
        }
    }

    fn take_seed(&mut self) -> u64 {
        self.next_seed += 1;
        self.next_seed
    }

    pub fn has_selection(&self) -> bool {
        self.cards
            .get(self.selected)
            .is_some_and(|c| c.genome.is_some())
    }

    /// The flame being worked on: the live edit while refining, otherwise the card's.
    pub fn current_genome(&self) -> Option<&Genome> {
        match self.stage {
            Stage::Refine => Some(&self.refine.working),
            _ => self
                .cards
                .get(self.selected)
                .and_then(|c| c.genome.as_ref()),
        }
    }

    pub fn go(&mut self, stage: Stage) {
        if stage == self.stage {
            return;
        }
        if self.stage == Stage::Refine {
            self.commit_refine();
        }
        if stage != Stage::Explore && !self.has_selection() {
            return;
        }
        match stage {
            Stage::Refine => {
                let genome = self.cards[self.selected]
                    .genome
                    .clone()
                    .expect("selection checked");
                self.refine_source = self.remember(&genome, None);
                let seed = self.take_seed();
                self.refine.begin(genome, seed);
                self.preview.texture = None;
            }
            Stage::Render => {
                self.render.invalidate();
                self.preview.texture = None;
            }
            Stage::Explore => {}
        }
        self.stage = stage;
    }

    /// Writes the edited flame back to its gallery card and refreshes its thumbnail.
    fn commit_refine(&mut self) {
        let Some(card) = self.cards.get_mut(self.selected) else {
            return;
        };
        if card.genome.as_ref() != Some(&self.refine.working) {
            let genome = self.refine.working.clone();
            card.genome = Some(genome.clone());
            let generation = self.worker.generations.gallery.load(Ordering::Relaxed);
            self.worker.submit(Job {
                slot: self.selected,
                generation,
                work: Work::Thumb { genome },
            });
        }
    }

    pub fn generate(&mut self) {
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
    }

    /// Slot 0 keeps the selected flame; the rest are mutations of it.
    pub fn evolve(&mut self) {
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
            hash: None,
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
    }

    /// Shows `genomes` as the gallery, with the first one selected, and switches to Explore.
    pub fn show_batch(&mut self, genomes: Vec<Genome>) {
        let generation = self.worker.next_gallery_generation();
        self.cards = genomes
            .iter()
            .map(|g| Card {
                genome: Some(g.clone()),
                texture: None,
                hash: None,
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
        self.view = View::Create;
        self.stage = Stage::Explore;
    }

    /// Switches to the History view, first saving any edit in progress in Refine.
    pub fn enter_history(&mut self) {
        self.go(Stage::Explore);
        self.history.refresh();
        self.view = View::History;
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
                let mut warnings: Vec<_> = flames
                    .iter()
                    .flat_map(|f| f.ignored.iter().cloned())
                    .collect();
                warnings.sort();
                warnings.dedup();
                let genomes: Vec<Genome> = flames.into_iter().map(|f| f.genome).take(60).collect();
                self.show_batch(genomes);
                self.status = if warnings.is_empty() {
                    format!("Opened {}", path.display())
                } else {
                    format!(
                        "Opened {} (ignored unsupported: {})",
                        path.display(),
                        warnings.join(", ")
                    )
                };
            }
            Err(e) => self.status = format!("Could not open {}: {e}", path.display()),
        }
    }

    fn save_flame(&mut self) {
        let Some(genome) = self.current_genome() else {
            return;
        };
        let name = if genome.name.is_empty() {
            "moth".to_string()
        } else {
            genome.name.clone()
        };
        let xml = io::to_xml(genome);
        let Some(path) = rfd::FileDialog::new()
            .set_file_name(format!("{name}.flame"))
            .add_filter("Flame", &["flame"])
            .save_file()
        else {
            return;
        };
        self.status = match std::fs::write(&path, xml) {
            Ok(()) => format!("Saved {}", path.display()),
            Err(e) => format!("Could not save {}: {e}", path.display()),
        };
    }

    fn enqueue_render(&mut self) {
        let Some(genome) = self.cards.get(self.selected).and_then(|c| c.genome.clone()) else {
            return;
        };
        let r = &self.render;
        let (ext, filter) = match r.format {
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
                width: r.width,
                height: r.height,
                quality: r.quality,
                transparent: r.transparent && r.format != Format::Jpeg,
                seed: r.seed,
                oversample: r.oversample,
                denoise: r.denoise,
            },
            depth: if r.format == Format::Png16 {
                BitDepth::Sixteen
            } else {
                BitDepth::Eight
            },
            jpeg_quality: r.jpeg_quality,
            ..ExportOptions::default()
        };
        self.status = format!("Queued {}", path.display());
        self.exporter.enqueue(genome, opts, path);
    }

    fn handle_done(&mut self, ctx: &egui::Context, done: Done) {
        let current = if done.preview {
            &self.worker.generations.preview
        } else {
            &self.worker.generations.gallery
        };
        if done.generation < current.load(Ordering::Relaxed) {
            return;
        }
        let texture = ctx.load_texture(
            format!("img-{}", done.genome.name),
            to_color_image(&done.image),
            TextureOptions::LINEAR,
        );
        if done.preview {
            self.preview.texture = Some(texture);
            self.preview.pending = false;
        } else if let Some(card) = self.cards.get_mut(done.slot) {
            card.texture = Some(texture);
            if card.genome.is_none() {
                card.genome = Some(done.genome);
            }
        }
    }
}

impl eframe::App for MothApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        while let Ok(done) = self.worker.rx.try_recv() {
            self.handle_done(&ctx, done);
        }
        if self.exporter.poll() {
            ctx.request_repaint();
        }
        if self.exporter.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }

        if self.view == View::Create
            && self.stage == Stage::Explore
            && !ctx.egui_wants_keyboard_input()
        {
            if ctx.input(|i| i.key_pressed(Key::Space)) {
                self.generate();
            }
            if ctx.input(|i| i.key_pressed(Key::E)) {
                self.evolve();
            }
        }

        self.header(ui);
        self.footer(ui);
        if self.view == View::History {
            history::ui(self, ui);
        } else {
            match self.stage {
                Stage::Explore => explore::ui(self, ui),
                Stage::Refine => {
                    refine::ui(self, ui);
                    self.record_refine_edits();
                }
                Stage::Render => render_stage::ui(self, ui),
            }
        }

        if let Some(mut smoke) = self.smoke.take() {
            smoke.tick(self, &ctx);
            self.smoke = Some(smoke);
        }
    }
}

impl MothApp {
    fn header(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("header").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                ui.label(RichText::new("Moth").strong().size(18.0));
                ui.add_space(24.0);
                let enabled = self.has_selection();
                let mut target = None;
                for (n, (stage, label)) in [
                    (Stage::Explore, "Explore"),
                    (Stage::Refine, "Refine"),
                    (Stage::Render, "Render"),
                ]
                .into_iter()
                .enumerate()
                {
                    if n > 0 {
                        ui.weak(icons::ARROW_RIGHT);
                    }
                    let active = self.stage == stage;
                    let text = RichText::new(format!("{}  {label}", n + 1)).size(15.0);
                    let text = if active { text.strong() } else { text };
                    let usable = stage == Stage::Explore || enabled;
                    if ui
                        .add_enabled(usable, egui::Button::selectable(active, text))
                        .clicked()
                    {
                        target = Some(stage);
                    }
                }
                if let Some(stage) = target {
                    self.view = View::Create;
                    self.go(stage);
                }
                let creating = self.view == View::Create;
                if ui
                    .add(egui::Button::selectable(
                        creating,
                        RichText::new("Create").size(15.0),
                    ))
                    .clicked()
                {
                    self.view = View::Create;
                }
                if ui
                    .add(egui::Button::selectable(
                        !creating,
                        RichText::new("History").size(15.0),
                    ))
                    .clicked()
                    && creating
                {
                    self.enter_history();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.menu_button("File", |ui| {
                        if ui.button("Open…").clicked() {
                            ui.close();
                            self.open_file();
                        }
                        if ui
                            .add_enabled(self.has_selection(), egui::Button::new("Save flame…"))
                            .clicked()
                        {
                            ui.close();
                            self.save_flame();
                        }
                    });
                });
            });
        });
    }

    fn footer(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("footer").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                match self.stage {
                    Stage::Explore => {
                        if ui
                            .button(format!("{} Generate new", icons::SPARKLE))
                            .on_hover_text("Space")
                            .clicked()
                        {
                            self.generate();
                        }
                        let has = self.has_selection();
                        if ui
                            .add_enabled(
                                has,
                                egui::Button::new(format!("{} Evolve selected", icons::DNA)),
                            )
                            .on_hover_text("E")
                            .clicked()
                        {
                            self.evolve();
                        }
                        ui.label("drift");
                        ui.add(
                            egui::Slider::new(&mut self.settings.drift, 0.0..=1.0)
                                .fixed_decimals(2),
                        );
                        ui.separator();
                        ui.toggle_value(&mut self.show_randomness, "Randomness…");
                    }
                    Stage::Refine => {
                        if ui
                            .add_enabled(
                                self.refine.can_undo(),
                                egui::Button::new(format!("{} Undo", icons::ARROW_U_UP_LEFT)),
                            )
                            .on_hover_text(UNDO_HINT)
                            .clicked()
                        {
                            self.refine.undo();
                        }
                        if ui
                            .add_enabled(
                                self.refine.can_redo(),
                                egui::Button::new(format!("{} Redo", icons::ARROW_U_UP_RIGHT)),
                            )
                            .on_hover_text(REDO_HINT)
                            .clicked()
                        {
                            self.refine.redo();
                        }
                        ui.separator();
                        if ui
                            .button(format!("{} Back to Explore", icons::ARROW_LEFT))
                            .clicked()
                        {
                            self.go(Stage::Explore);
                        }
                    }
                    Stage::Render => {
                        if ui
                            .button(format!("{} Back to Refine", icons::ARROW_LEFT))
                            .clicked()
                        {
                            self.go(Stage::Refine);
                        }
                    }
                }
                ui.add_space(8.0);
                ui.colored_label(Color32::GRAY, &self.status);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(4.0);
                    match self.stage {
                        Stage::Explore => {
                            if ui
                                .add_enabled(
                                    self.has_selection(),
                                    primary(&format!("Refine this {}", icons::ARROW_RIGHT)),
                                )
                                .clicked()
                            {
                                self.go(Stage::Refine);
                            }
                        }
                        Stage::Refine => {
                            if ui
                                .add(primary(&format!("Render {}", icons::ARROW_RIGHT)))
                                .clicked()
                            {
                                self.go(Stage::Render);
                            }
                        }
                        Stage::Render => {
                            if ui.add(primary("Render…")).clicked() {
                                self.enqueue_render();
                            }
                        }
                    }
                });
            });
        });
    }
}

fn primary(label: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(label.to_string()).strong().size(15.0))
        .fill(Color32::from_rgb(190, 120, 20))
        .min_size(egui::vec2(130.0, 28.0))
}

fn to_color_image(img: &Image) -> egui::ColorImage {
    let bytes: Vec<u8> = img
        .rgba
        .iter()
        .map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
        .collect();
    egui::ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &bytes)
}
