//! Self-test: `MOTH_SMOKE=<dir> moth-ui` walks through every stage on its own, saves a
//! screenshot of the app's own window at each step and checks a few behaviours, then quits.
//! It captures only Moth's viewport (never the desktop) and needs no input from anyone.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use eframe::egui;
use moth_core::flame::Variation;

use crate::app::{MothApp, Stage};
use crate::refine::Tab;

pub struct Smoke {
    dir: PathBuf,
    step: usize,
    step_started: Instant,
    shot_done: Option<Arc<AtomicBool>>,
    original: Option<moth_core::flame::Genome>,
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
                app.selected = 2;
                app.go(Stage::Refine);
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
                if waited > Duration::from_secs(40) && self.step < 15 {
                    println!("SMOKE FAIL step {} timed out", self.step);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    self.step = 99;
                }
            }
        }
    }
}
