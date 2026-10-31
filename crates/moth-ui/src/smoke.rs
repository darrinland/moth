//! Self-test: `MOTH_SMOKE=<dir> moth-ui` walks through every stage on its own, saves a
//! screenshot of the app's own window at each step and checks a few behaviours, then quits.
//! It captures only Moth's viewport (never the desktop) and needs no input from anyone.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use eframe::egui;
use moth_core::flame::Variation;

use crate::app::{MothApp, Stage, View};
use crate::refine::Tab;

pub struct Smoke {
    dir: PathBuf,
    step: usize,
    step_started: Instant,
    shot_done: Option<Arc<AtomicBool>>,
    original: Option<moth_core::flame::Genome>,
    /// Library rows before the self-test selected anything, and the library id of the flame it
    /// refines. The library may already hold rows from an earlier run in the same folder.
    base_rows: i64,
    source: Option<i64>,
    failures: usize,
}

impl Smoke {
    pub fn from_env() -> Option<Self> {
        let dir = PathBuf::from(std::env::var_os("MOTH_SMOKE")?);
        std::fs::create_dir_all(&dir).ok()?;
        Some(Self {
            dir,
            step: 0,
            step_started: Instant::now(),
            shot_done: None,
            original: None,
            base_rows: 0,
            source: None,
            failures: 0,
        })
    }

    fn next(&mut self) {
        println!("SMOKE step {} -> {}", self.step, self.step + 1);
        self.step += 1;
        self.step_started = Instant::now();
    }

    fn check(&mut self, ok: bool, what: &str) {
        println!("SMOKE {} {what}", if ok { "ok  " } else { "FAIL" });
        if !ok {
            self.failures += 1;
        }
    }

    fn shoot(&mut self, ctx: &egui::Context, name: &str) {
        let done = Arc::new(AtomicBool::new(false));
        let flag = done.clone();
        let path = self.dir.join(format!("{name}.png"));
        let repaint = ctx.clone();
        ctx.request_screenshot(move |image| {
            let bytes: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
            if let Ok(file) = std::fs::File::create(&path) {
                let mut enc = png::Encoder::new(
                    std::io::BufWriter::new(file),
                    image.size[0] as u32,
                    image.size[1] as u32,
                );
                enc.set_color(png::ColorType::Rgba);
                enc.set_depth(png::BitDepth::Eight);
                if let Ok(mut w) = enc.write_header() {
                    let _ = w.write_image_data(&bytes);
                }
            }
            flag.store(true, Ordering::Relaxed);
            repaint.request_repaint();
        });
        self.shot_done = Some(done);
    }

    /// Called once per frame. Each step waits for the app to settle, acts, or takes a shot.
    pub fn tick(&mut self, app: &mut MothApp, ctx: &egui::Context) {
        ctx.request_repaint_after(Duration::from_millis(100));
        if let Some(done) = &self.shot_done {
            if !done.load(Ordering::Relaxed) {
                return;
            }
            self.shot_done = None;
            self.next();
            return;
        }
        let waited = self.step_started.elapsed();
        let settled = !app.preview.pending && waited > Duration::from_millis(1800);
        match self.step {
            0 if app.cards.len() == 10
                && app.cards.iter().all(|c| c.texture.is_some())
                && waited > Duration::from_millis(500) =>
            {
                self.check(true, "explore: 10 thumbnails rendered");
                self.shoot(ctx, "1-explore");
            }
            1 => {
                let rows =
                    |app: &MothApp| app.library.as_ref().map_or(-1, |l| l.len().unwrap_or(-1));
                self.base_rows = rows(app);
                // Restart check: a previous run left its row count in the scratch folder.
                if let Ok(text) = std::fs::read_to_string(self.dir.join("rows.txt")) {
                    let before: i64 = text.trim().parse().unwrap_or(-2);
                    self.check(
                        self.base_rows == before,
                        "library: rows from the last run are still here after a restart",
                    );
                }
                for i in 0..5 {
                    app.select(i);
                }
                app.select(0); // a repeat must not add a row
                self.check(
                    rows(app) == self.base_rows + 5,
                    "library: five selections make five rows, a repeat none",
                );
                app.selected = 2;
                app.go(Stage::Refine);
                self.source = app.refine_source;
                self.original = Some(app.refine.working.clone());
                self.next();
            }
            2 if settled && app.preview.texture.is_some() => {
                self.check(app.stage == Stage::Refine, "refine: entered the stage");
                self.shoot(ctx, "2-refine-shapes");
            }
            3 => {
                app.refine.tab = Tab::Color;
                self.next();
            }
            4 if waited > Duration::from_millis(400) => self.shoot(ctx, "3-refine-color"),
            5 => {
                app.refine.tab = Tab::Look;
                self.next();
            }
            6 if waited > Duration::from_millis(400) => self.shoot(ctx, "4-refine-look"),
            7 => {
                // Edit several things at once, the way a user would across a minute.
                let g = &mut app.refine.working;
                g.brightness = 9.0;
                g.xforms[0].variations.push((Variation::Spherical, 0.6));
                g.palette.shift_hue(0.3);
                g.camera.scale *= 1.25;
                app.refine.tab = Tab::Shapes;
                self.next();
            }
            8 if settled => {
                let changed = Some(&app.refine.working) != self.original.as_ref();
                self.check(
                    changed && app.refine.can_undo(),
                    "refine: edits create an undo step",
                );
                let rows = app.library.as_ref().map_or(-1, |l| l.len().unwrap_or(-1));
                self.check(
                    rows == self.base_rows + 6,
                    "library: one committed edit adds one row",
                );
                let parent = app
                    .library
                    .as_ref()
                    .zip(app.refine_source)
                    .and_then(|(l, id)| l.get(id).ok().flatten())
                    .map(|stored| stored.parent);
                self.check(
                    parent == Some(self.source),
                    "library: the edit records the flame it came from as its parent",
                );
                self.shoot(ctx, "5-refine-edited");
            }
            9 => {
                app.refine.undo();
                let restored = Some(&app.refine.working) == self.original.as_ref();
                if !restored {
                    if let Some(o) = &self.original {
                        let w = &app.refine.working;
                        println!(
                            "SMOKE diff: xforms={} final={} palette={} camera={} brightness={} gamma={} vibrancy={} background={}",
                            o.xforms != w.xforms,
                            o.final_xform != w.final_xform,
                            o.palette != w.palette,
                            o.camera != w.camera,
                            o.brightness != w.brightness,
                            o.gamma != w.gamma,
                            o.vibrancy != w.vibrancy,
                            o.background != w.background
                        );
                        if o.camera != w.camera {
                            println!("SMOKE camera {:?} vs {:?}", o.camera, w.camera);
                        }
                        for (i, (a, b)) in o.xforms.iter().zip(&w.xforms).enumerate() {
                            if a != b {
                                println!("SMOKE xform {i}: {a:?}\n   vs {b:?}");
                            }
                        }
                    }
                }
                self.check(restored, "refine: undo restores the original flame exactly");
                app.refine.redo();
                self.check(
                    Some(&app.refine.working) != self.original.as_ref(),
                    "refine: redo reapplies the edit",
                );
                let rows = app.library.as_ref().map_or(-1, |l| l.len().unwrap_or(-1));
                self.check(
                    rows == self.base_rows + 6,
                    "library: undo and redo add no rows",
                );
                self.next();
            }
            10 => {
                app.go(Stage::Render);
                let kept = app.cards[app.selected].genome.as_ref() == Some(&app.refine.working);
                self.check(kept, "render: leaving Refine kept the edited flame");
                self.next();
            }
            11 if settled && app.preview.texture.is_some() => self.shoot(ctx, "6-render"),
            12 => {
                app.go(Stage::Explore);
                self.next();
            }
            13 if app.cards.iter().all(|c| c.texture.is_some())
                && waited > Duration::from_millis(1500) =>
            {
                self.shoot(ctx, "7-explore-after");
            }
            14 => {
                // One flame last selected 40 days ago, so History shows the clearing banner.
                if let Some(lib) = app.library.as_mut() {
                    let old = moth_core::library::now() - 40 * moth_core::library::DAY;
                    let _ = lib.record("<flame name=\"smoke-old\"/>", "smoke-old", None, old);
                }
                app.enter_history();
                self.next();
            }
            15 if waited > Duration::from_millis(1500) => {
                // Thumbnails are made in the background, a few per second; wait for them.
                let rows = app.library.as_ref().map_or(-1, |l| l.len().unwrap_or(-1));
                let shown = app.history.thumbs.textures_len();
                self.check(
                    app.view == View::History && rows == self.base_rows + 7,
                    "history: the view opens and lists the library",
                );
                if shown < (rows as usize).saturating_sub(1) && waited < Duration::from_secs(60) {
                    return;
                }
                self.check(shown > 0, "history: thumbnails appear");
                self.shoot(ctx, "8-history");
            }
            16 => {
                let offered = app
                    .library
                    .as_ref()
                    .and_then(|l| l.retention_offer(moth_core::library::now()).ok().flatten());
                self.check(
                    offered.is_some_and(|u| u.count == 1),
                    "history: the offer counts the one old flame",
                );
                let cleared = app.library.as_mut().and_then(|l| {
                    l.clear_older_than(moth_core::library::now() - moth_core::library::STALE_AFTER)
                        .ok()
                });
                let rows = app.library.as_ref().map_or(-1, |l| l.len().unwrap_or(-1));
                self.check(
                    cleared.is_some_and(|u| u.count == 1) && rows == self.base_rows + 6,
                    "history: clearing removes only the old flame",
                );
                // A star on an Explore card survives clearing the whole history.
                app.view = View::Create;
                app.toggle_card_star(3);
                let hash = app.cards[3].hash.clone().unwrap_or_default();
                let hash = if hash.is_empty() {
                    moth_core::library::content_hash(&moth_core::flame::io::to_xml(
                        app.cards[3].genome.as_ref().expect("card has a flame"),
                    ))
                } else {
                    hash
                };
                self.check(
                    app.favorite_hashes.contains(&hash),
                    "favorites: starring a card marks it",
                );
                if let Some(l) = app.library.as_mut() {
                    let _ = l.clear_all_but_favorites();
                }
                app.refresh_favorites();
                let rows = app.library.as_ref().map_or(-1, |l| l.len().unwrap_or(-1));
                self.check(
                    rows == app.favorite_hashes.len() as i64 && app.favorite_hashes.contains(&hash),
                    "favorites: the starred flame survives clearing",
                );
                self.shoot(ctx, "9-explore-star");
            }
            17 => {
                let rows = app.library.as_ref().map_or(-1, |l| l.len().unwrap_or(-1));
                let _ = std::fs::write(self.dir.join("rows.txt"), rows.to_string());
                println!("SMOKE done, {} failure(s)", self.failures);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                self.next();
            }
            _ => {
                if waited > Duration::from_secs(8) && waited.as_secs() % 4 == 0 {
                    println!(
                        "SMOKE waiting at step {}: stage={} pending={} has_texture={} cards_textured={}",
                        self.step,
                        match app.stage {
                            Stage::Explore => "explore",
                            Stage::Refine => "refine",
                            Stage::Render => "render",
                        },
                        app.preview.pending,
                        app.preview.texture.is_some(),
                        app.cards.iter().filter(|c| c.texture.is_some()).count()
                    );
                }
                if waited > Duration::from_secs(40) && self.step < 18 {
                    println!("SMOKE FAIL step {} timed out", self.step);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    self.step = 99;
                }
            }
        }
    }
}
