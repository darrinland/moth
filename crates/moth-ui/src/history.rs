//! The History view: every flame the library holds, newest selection first, with thumbnails.
//! Opening a flame, sending flames to Explore, starring, deleting, notes and lineage all go
//! through here.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

use eframe::egui::{self, Color32, ColorImage, RichText, TextureHandle, TextureOptions};
use moth_core::RenderSpec;
use moth_core::flame::{FlameGenerator, Genome, io};
use moth_core::generator::Generator;
use moth_core::library::{self, Stored, Usage};
use moth_core::output::{self, BitDepth};

use crate::app::{MothApp, Stage};
use crate::icons;
use crate::worker::THUMB_SIZE;

/// Most flames one History action sends to Explore at once.
const MAX_BATCH: usize = 60;
/// Width of one History card: the thumbnail plus its frame.
const CARD_WIDTH: f32 = THUMB_SIZE.0 as f32 / 2.0 + 24.0;
/// How far up the lineage the detail panel walks.
const MAX_ANCESTORS: usize = 50;

struct ThumbJob {
    id: i64,
    genome: Genome,
    path: PathBuf,
}

struct ThumbDone {
    id: i64,
    size: [usize; 2],
    rgba: Vec<u8>,
}

/// Makes History thumbnails on one background thread. A thumbnail already on disk is read
/// back; otherwise the flame is rendered and written beside the library, keyed by id.
pub struct Thumbs {
    jobs: Sender<ThumbJob>,
    done: Receiver<ThumbDone>,
    textures: HashMap<i64, TextureHandle>,
    requested: HashSet<i64>,
}

impl Thumbs {
    pub fn spawn(ctx: egui::Context) -> Self {
        let (jobs_tx, jobs_rx) = channel::<ThumbJob>();
        let (done_tx, done_rx) = channel::<ThumbDone>();
        thread::spawn(move || {
            while let Ok(job) = jobs_rx.recv() {
                if let Some(done) = make_thumb(&job) {
                    if done_tx.send(done).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            }
        });
        Self {
            jobs: jobs_tx,
            done: done_rx,
            textures: HashMap::new(),
            requested: HashSet::new(),
        }
    }

    /// Asks for a thumbnail unless it is already here or on its way.
    fn want(&mut self, id: i64, genome: impl FnOnce() -> Option<Genome>, path: PathBuf) {
        if self.textures.contains_key(&id) || self.requested.contains(&id) {
            return;
        }
        let Some(genome) = genome() else { return };
        self.requested.insert(id);
        let _ = self.jobs.send(ThumbJob { id, genome, path });
    }

    /// Uploads finished thumbnails as textures.
    fn poll(&mut self, ctx: &egui::Context) {
        while let Ok(done) = self.done.try_recv() {
            let image = ColorImage::from_rgba_unmultiplied(done.size, &done.rgba);
            let texture = ctx.load_texture(
                format!("history-{}", done.id),
                image,
                TextureOptions::LINEAR,
            );
            self.textures.insert(done.id, texture);
        }
    }

    /// How many thumbnails are ready to show.
    pub fn textures_len(&self) -> usize {
        self.textures.len()
    }

    /// Drops a flame's thumbnail, for when it is deleted.
    fn forget_all(&mut self) {
        self.textures.clear();
        self.requested.clear();
    }

    fn forget(&mut self, id: i64) {
        self.textures.remove(&id);
        self.requested.remove(&id);
    }
}

fn make_thumb(job: &ThumbJob) -> Option<ThumbDone> {
    if job.path.exists()
        && let Ok((w, h, rgba)) = output::read_png_rgba(&job.path)
    {
        return Some(ThumbDone {
            id: job.id,
            size: [w as usize, h as usize],
            rgba,
        });
    }
    let spec = RenderSpec {
        width: THUMB_SIZE.0,
        height: THUMB_SIZE.1,
        quality: 50.0,
        ..RenderSpec::default()
    };
    let img = FlameGenerator.render(&job.genome, &spec);
    if let Some(dir) = job.path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = output::write_png(&job.path, &img, BitDepth::Eight, None);
    let rgba = img
        .rgba
        .iter()
        .map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
        .collect();
    Some(ThumbDone {
        id: job.id,
        size: [img.width as usize, img.height as usize],
        rgba,
    })
}

/// What the History view remembers between frames.
pub struct HistoryState {
    pub thumbs: Thumbs,
    search: String,
    favorites_only: bool,
    selected: Option<i64>,
    note_draft: String,
    note_for: Option<i64>,
    confirm_delete: Option<i64>,
    /// Storage figures and the clearing offer, read once and again after anything changes.
    retention: Option<Retention>,
    /// "Keep" hides the banner until the next launch.
    banner_dismissed: bool,
    confirm_clear_all: bool,
}

/// What the retention banner and the storage indicator show.
#[derive(Clone, Copy)]
struct Retention {
    storage: u64,
    offer: Option<Usage>,
}

impl HistoryState {
    pub fn new(ctx: egui::Context) -> Self {
        Self {
            thumbs: Thumbs::spawn(ctx),
            search: String::new(),
            favorites_only: false,
            selected: None,
            note_draft: String::new(),
            note_for: None,
            confirm_delete: None,
            retention: None,
            banner_dismissed: false,
            confirm_clear_all: false,
        }
    }

    /// Forces the storage figures and the offer to be read again.
    pub fn refresh(&mut self) {
        self.retention = None;
    }
}

enum Action {
    Select(i64),
    Open(i64),
    SendToExplore(Vec<i64>),
    Star(i64, bool),
    AskDelete(i64),
    Delete(i64),
    SaveNote(i64, String),
    ClearStale,
    RemindLater,
    DismissBanner,
    AskClearAll,
    CancelClearAll,
    ClearAll,
}

pub fn ui(app: &mut MothApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    app.history.thumbs.poll(&ctx);
    let Some(library) = app.library.as_ref() else {
        ui.label("The library is not available. See the status line for why.");
        return;
    };
    let entries = match library.list(&app.history.search, app.history.favorites_only) {
        Ok(entries) => entries,
        Err(e) => {
            ui.colored_label(
                Color32::LIGHT_RED,
                format!("Could not read the library: {e}"),
            );
            return;
        }
    };
    let mut actions = Vec::new();
    if app.history.retention.is_none() {
        let now = library::now();
        app.history.retention = Some(Retention {
            storage: library.storage_bytes(),
            offer: library.retention_offer(now).ok().flatten(),
        });
    }
    let retention = app.history.retention.expect("set above");

    egui::Panel::right("history-detail")
        .default_size(320.0)
        .show(ui, |ui| {
            detail(app, ui, &entries, &mut actions);
        });

    egui::CentralPanel::default().show(ui, |ui| {
        if let Some(offer) = retention.offer
            && !app.history.banner_dismissed
        {
            banner(ui, offer, &mut actions);
        }
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut app.history.search)
                    .hint_text("Search name or note")
                    .desired_width(240.0),
            );
            ui.toggle_value(&mut app.history.favorites_only, "Favorites");
            ui.weak(format!("{} flames", entries.len()));
            ui.weak(format!("{} on disk", size_label(retention.storage)));
            if app.history.confirm_clear_all {
                ui.colored_label(Color32::LIGHT_RED, "Remove everything but favorites?");
                if ui.button("Clear").clicked() {
                    actions.push(Action::ClearAll);
                }
                if ui.button("Cancel").clicked() {
                    actions.push(Action::CancelClearAll);
                }
            } else if ui.small_button("Clear all (keep favorites)").clicked() {
                actions.push(Action::AskClearAll);
            }
            if (!app.history.search.is_empty() || app.history.favorites_only)
                && ui.small_button("Clear filters").clicked()
            {
                app.history.search.clear();
                app.history.favorites_only = false;
            }
        });
        ui.separator();
        if entries.is_empty() {
            ui.weak("Nothing matches. Flames you select or edit are kept here.");
        }
        egui::ScrollArea::vertical().show(ui, |ui| {
            // Cards are a fixed width, so the column count follows the window width.
            let columns = ((ui.available_width() / CARD_WIDTH).floor() as usize).max(1);
            egui::Grid::new("history-grid")
                .spacing([12.0, 12.0])
                .show(ui, |ui| {
                    for (i, stored) in entries.iter().enumerate() {
                        card(app, ui, stored, &mut actions);
                        if (i + 1) % columns == 0 {
                            ui.end_row();
                        }
                    }
                });
        });
    });

    for action in actions {
        apply(app, action);
    }
}

fn banner(ui: &mut egui::Ui, offer: Usage, actions: &mut Vec<Action>) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(format!(
                "{} flames not selected in the last 30 days use {}. Clear them? Favorites stay.",
                offer.count,
                size_label(offer.bytes)
            ));
            if ui.button("Clear them").clicked() {
                actions.push(Action::ClearStale);
            }
            if ui.button("Keep").clicked() {
                actions.push(Action::DismissBanner);
            }
            if ui.button("Remind me in 7 days").clicked() {
                actions.push(Action::RemindLater);
            }
        });
    });
}

fn card(app: &mut MothApp, ui: &mut egui::Ui, stored: &Stored, actions: &mut Vec<Action>) {
    let selected = app.history.selected == Some(stored.id);
    let frame = egui::Frame::group(ui.style()).stroke(if selected {
        egui::Stroke::new(2.0, Color32::from_rgb(190, 120, 20))
    } else {
        ui.style().visuals.widgets.noninteractive.bg_stroke
    });
    frame.show(ui, |ui| {
        // Grid cells lay out left to right, so the card stacks its parts itself.
        ui.vertical(|ui| {
            ui.set_width(CARD_WIDTH - 24.0);
            let path = app
                .library
                .as_ref()
                .map(|l| l.thumbnail_path(stored.id))
                .unwrap_or_default();
            app.history.thumbs.want(
                stored.id,
                || {
                    io::from_xml(&stored.xml)
                        .ok()
                        .and_then(|mut v| v.pop())
                        .map(|f| f.genome)
                },
                path,
            );
            match app.history.thumbs.textures.get(&stored.id) {
                Some(texture) => {
                    let size = egui::vec2(THUMB_SIZE.0 as f32 / 2.0, THUMB_SIZE.1 as f32 / 2.0);
                    if ui
                        .add(
                            egui::Image::new(texture)
                                .fit_to_exact_size(size)
                                .sense(egui::Sense::click()),
                        )
                        .clicked()
                    {
                        actions.push(Action::Select(stored.id));
                    }
                }
                None => {
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(THUMB_SIZE.0 as f32 / 2.0, THUMB_SIZE.1 as f32 / 2.0),
                        egui::Sense::click(),
                    );
                    ui.painter().rect_filled(rect, 4.0, Color32::from_gray(40));
                    if response.clicked() {
                        actions.push(Action::Select(stored.id));
                    }
                }
            }
            let title = if stored.name.is_empty() {
                format!("flame {}", stored.id)
            } else {
                stored.name.clone()
            };
            ui.horizontal(|ui| {
                let star = if stored.favorite {
                    icons::CHECK_CIRCLE
                } else {
                    icons::SPARKLE
                };
                if ui.small_button(star).on_hover_text("Favorite").clicked() {
                    actions.push(Action::Star(stored.id, !stored.favorite));
                }
                ui.add(egui::Label::new(RichText::new(title).strong()).truncate());
            });
            ui.weak(date_label(stored.last_selected));
        });
    });
}

fn detail(app: &mut MothApp, ui: &mut egui::Ui, entries: &[Stored], actions: &mut Vec<Action>) {
    let Some(id) = app.history.selected else {
        ui.heading("History");
        ui.weak("Pick a flame to see its details.");
        return;
    };
    let Some(stored) = entries
        .iter()
        .find(|s| s.id == id)
        .cloned()
        .or_else(|| app.library.as_ref().and_then(|l| l.get(id).ok().flatten()))
    else {
        ui.weak("That flame is gone.");
        return;
    };
    if app.history.note_for != Some(id) {
        app.history.note_draft = stored.note.clone();
        app.history.note_for = Some(id);
    }
    ui.heading(if stored.name.is_empty() {
        "Untitled"
    } else {
        &stored.name
    });
    ui.weak(format!(
        "First selected {}\nLast selected {}",
        date_label(stored.created),
        date_label(stored.last_selected)
    ));
    ui.add_space(8.0);
    if ui.button("Open in Refine").clicked() {
        actions.push(Action::Open(id));
    }
    if ui
        .button("Send to Explore as a new batch")
        .on_hover_text(format!(
            "Up to {MAX_BATCH} flames: this one and the rest of the list"
        ))
        .clicked()
    {
        let mut ids: Vec<i64> = vec![id];
        ids.extend(entries.iter().map(|s| s.id).filter(|&other| other != id));
        ids.truncate(MAX_BATCH);
        actions.push(Action::SendToExplore(ids));
    }
    ui.horizontal(|ui| {
        let label = if stored.favorite { "Unstar" } else { "Star" };
        if ui.button(label).clicked() {
            actions.push(Action::Star(id, !stored.favorite));
        }
        if app.history.confirm_delete == Some(id) {
            if ui
                .button(RichText::new("Really delete").color(Color32::LIGHT_RED))
                .clicked()
            {
                actions.push(Action::Delete(id));
            }
            if ui.button("Keep").clicked() {
                app.history.confirm_delete = None;
            }
        } else if ui.button("Delete").clicked() {
            actions.push(Action::AskDelete(id));
        }
    });
    ui.add_space(8.0);
    ui.label("Note");
    ui.add(
        egui::TextEdit::multiline(&mut app.history.note_draft)
            .desired_rows(3)
            .desired_width(f32::INFINITY),
    );
    if ui.button("Save note").clicked() {
        actions.push(Action::SaveNote(id, app.history.note_draft.clone()));
    }
    ui.add_space(8.0);
    ui.label("Lineage");
    let ancestors = ancestors(app, &stored);
    if ancestors.is_empty() {
        ui.weak("No earlier versions: this flame came from nowhere the library knows.");
    }
    for ancestor in ancestors {
        let label = if ancestor.name.is_empty() {
            format!("flame {}", ancestor.id)
        } else {
            ancestor.name.clone()
        };
        if ui.link(format!("{} {label}", icons::ARROW_LEFT)).clicked() {
            actions.push(Action::Select(ancestor.id));
        }
    }
}

/// The chain of parents above `stored`, nearest first.
fn ancestors(app: &MothApp, stored: &Stored) -> Vec<Stored> {
    let Some(library) = app.library.as_ref() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut next = stored.parent;
    while let Some(parent) = next {
        if out.len() >= MAX_ANCESTORS {
            break;
        }
        match library.get(parent).ok().flatten() {
            Some(s) => {
                next = s.parent;
                out.push(s);
            }
            None => break,
        }
    }
    out
}

fn apply(app: &mut MothApp, action: Action) {
    app.history.refresh();
    match action {
        Action::DismissBanner => app.history.banner_dismissed = true,
        Action::AskClearAll => app.history.confirm_clear_all = true,
        Action::CancelClearAll => app.history.confirm_clear_all = false,
        Action::RemindLater => {
            if let Some(library) = app.library.as_mut() {
                match library.remind_later(library::now()) {
                    Ok(()) => app.status = "Will ask again in 7 days".to_string(),
                    Err(e) => app.status = format!("Could not save the reminder: {e}"),
                }
            }
            app.history.banner_dismissed = true;
        }
        Action::ClearStale | Action::ClearAll => {
            let all = matches!(action, Action::ClearAll);
            app.history.confirm_clear_all = false;
            let Some(library) = app.library.as_mut() else {
                return;
            };
            let result = if all {
                library.clear_all_but_favorites()
            } else {
                library.clear_older_than(library::now() - library::STALE_AFTER)
            };
            match result {
                Ok(done) => {
                    app.history.thumbs.forget_all();
                    app.history.selected = None;
                    app.status = format!(
                        "Cleared {} flames, freeing {}",
                        done.count,
                        size_label(done.bytes)
                    );
                }
                Err(e) => app.status = format!("Could not clear the library: {e}"),
            }
        }
        Action::Select(id) => {
            app.history.selected = Some(id);
            app.history.confirm_delete = None;
        }
        Action::AskDelete(id) => app.history.confirm_delete = Some(id),
        Action::Star(id, favorite) => {
            if let Some(library) = app.library.as_mut()
                && let Err(e) = library.set_favorite(id, favorite)
            {
                app.status = format!("Could not star the flame: {e}");
            }
        }
        Action::SaveNote(id, note) => {
            if let Some(library) = app.library.as_mut() {
                match library.set_note(id, &note) {
                    Ok(_) => app.status = "Note saved".to_string(),
                    Err(e) => app.status = format!("Could not save the note: {e}"),
                }
            }
        }
        Action::Delete(id) => {
            let Some(library) = app.library.as_mut() else {
                return;
            };
            let path = library.thumbnail_path(id);
            match library.delete(id) {
                Ok(_) => {
                    let _ = std::fs::remove_file(path);
                    app.history.thumbs.forget(id);
                    app.history.confirm_delete = None;
                    if app.history.selected == Some(id) {
                        app.history.selected = None;
                    }
                    app.status = "Flame deleted".to_string();
                }
                Err(e) => app.status = format!("Could not delete the flame: {e}"),
            }
        }
        Action::Open(id) => {
            let Some(genome) = genome_of(app, id) else {
                app.status = "Could not open that flame".to_string();
                return;
            };
            app.show_batch(vec![genome]);
            app.select(0);
            app.go(Stage::Refine);
        }
        Action::SendToExplore(ids) => {
            let genomes: Vec<Genome> = ids.iter().filter_map(|&id| genome_of(app, id)).collect();
            if genomes.is_empty() {
                return;
            }
            app.show_batch(genomes);
        }
    }
}

fn genome_of(app: &MothApp, id: i64) -> Option<Genome> {
    let stored = app.library.as_ref()?.get(id).ok()??;
    io::from_xml(&stored.xml).ok()?.pop().map(|f| f.genome)
}

/// A size such as `12 KB` or `3.4 MB`.
fn size_label(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b < KB {
        format!("{bytes} B")
    } else if b < KB * KB {
        format!("{:.0} KB", b / KB)
    } else if b < KB * KB * KB {
        format!("{:.1} MB", b / (KB * KB))
    } else {
        format!("{:.2} GB", b / (KB * KB * KB))
    }
}

/// A date as `YYYY-MM-DD HH:MM` (UTC), without a date library.
fn date_label(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        secs / 3600,
        (secs % 3600) / 60
    )
}

#[cfg(test)]
mod tests {
    use super::{date_label, size_label};

    #[test]
    fn sizes_are_readable() {
        assert_eq!(size_label(512), "512 B");
        assert_eq!(size_label(2048), "2 KB");
        assert_eq!(size_label(3 * 1024 * 1024 + 400_000), "3.4 MB");
    }

    #[test]
    fn dates_are_civil() {
        assert_eq!(date_label(0), "1970-01-01 00:00");
        assert_eq!(date_label(1_000_000_000), "2001-09-09 01:46");
        assert_eq!(date_label(951_782_400), "2000-02-29 00:00", "leap day");
    }
}
