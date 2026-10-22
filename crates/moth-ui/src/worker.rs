//! Background rendering. One thread works through jobs in order, so each render gets
//! every core and finished images stream into the UI as they complete. Jobs from a
//! superseded generation (the user clicked "Generate" again) are dropped unrendered.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};

use moth_core::flame::{FlameGenerator, Genome, RandomSettings};
use moth_core::{Generator, Image, RenderSpec};

pub const THUMB_SIZE: (u32, u32) = (320, 240);

pub enum Work {
    /// A brand new random flame for a gallery slot.
    Fresh { seed: u64, settings: RandomSettings },
    /// A mutation of an existing flame.
    Evolve {
        parent: Genome,
        amount: f32,
        seed: u64,
        settings: RandomSettings,
    },
    /// Re-render a flame that already exists (opened from a file).
    Thumb { genome: Genome },
    /// A larger render of one flame for the preview panel.
    Preview { genome: Genome, size: (u32, u32) },
}

pub struct Job {
    pub slot: usize,
    pub generation: u64,
    pub work: Work,
}

pub struct Done {
    pub slot: usize,
    pub generation: u64,
    pub preview: bool,
    pub genome: Genome,
    pub image: Image,
}

#[derive(Clone, Default)]
pub struct Generations {
    pub gallery: Arc<AtomicU64>,
    pub preview: Arc<AtomicU64>,
}

pub struct Worker {
    tx: Sender<Job>,
    pub rx: Receiver<Done>,
    pub generations: Generations,
}

impl Worker {
    pub fn spawn(ctx: eframe::egui::Context) -> Self {
        let (tx, jobs) = channel::<Job>();
        let (done_tx, rx) = channel::<Done>();
        let generations = Generations::default();
        let gens = generations.clone();
        std::thread::Builder::new()
            .name("moth-render".into())
            .spawn(move || {
                for job in jobs {
                    let preview = matches!(job.work, Work::Preview { .. });
                    let current = if preview {
                        &gens.preview
                    } else {
                        &gens.gallery
                    };
                    if job.generation < current.load(Ordering::Relaxed) {
                        continue;
                    }
                    let (genome, image) = run(job.work);
                    let done = Done {
                        slot: job.slot,
                        generation: job.generation,
                        preview,
                        genome,
                        image,
                    };
                    if done_tx.send(done).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            })
            .expect("spawn render thread");
        Self {
            tx,
            rx,
            generations,
        }
    }

    pub fn submit(&self, job: Job) {
        let _ = self.tx.send(job);
    }

    /// Marks everything queued so far as stale and returns the new generation.
    pub fn next_gallery_generation(&self) -> u64 {
        self.generations.gallery.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub fn next_preview_generation(&self) -> u64 {
        self.generations.preview.fetch_add(1, Ordering::Relaxed) + 1
    }
}

fn run(work: Work) -> (Genome, Image) {
    let gen_ = FlameGenerator;
    match work {
        Work::Fresh { seed, settings } => {
            let mut g = gen_.randomize(&settings, seed);
            g.name = format!("moth-{seed}");
            let img = gen_.render(&g, &thumb_spec());
            (g, img)
        }
        Work::Evolve {
            parent,
            amount,
            seed,
            settings,
        } => {
            let mut g = gen_.mutate(&parent, &settings, amount, seed);
            g.name = format!("{}-{seed}", base_name(&parent.name));
            let img = gen_.render(&g, &thumb_spec());
            (g, img)
        }
        Work::Thumb { genome } => {
            let img = gen_.render(&genome, &thumb_spec());
            (genome, img)
        }
        Work::Preview { genome, size } => {
            let spec = RenderSpec {
                width: size.0,
                height: size.1,
                quality: 150.0,
                oversample: 2,
                ..RenderSpec::default()
            };
            let img = gen_.render(&genome, &spec);
            (genome, img)
        }
    }
}

fn thumb_spec() -> RenderSpec {
    RenderSpec {
        width: THUMB_SIZE.0,
        height: THUMB_SIZE.1,
        quality: 50.0,
        ..RenderSpec::default()
    }
}

/// Strips an earlier `-<seed>` suffix so repeated evolving does not grow the name.
fn base_name(name: &str) -> &str {
    match name.rsplit_once('-') {
        Some((head, tail)) if !head.is_empty() && tail.chars().all(|c| c.is_ascii_digit()) => head,
        _ => {
            if name.is_empty() {
                "moth"
            } else {
                name
            }
        }
    }
}
