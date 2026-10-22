//! A queue of image exports. One background thread renders them in order, so a second
//! export waits for the first instead of fighting it for the CPU.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Instant;

use moth_core::export::{ExportOptions, export};
use moth_core::flame::Genome;

struct Job {
    id: u64,
    genome: Genome,
    opts: ExportOptions,
    path: PathBuf,
}

enum Update {
    Started(u64),
    Finished(u64, Result<(), String>),
}

#[derive(Clone, PartialEq)]
pub enum State {
    Queued,
    Rendering,
    Done,
    Failed(String),
}

pub struct Item {
    pub id: u64,
    pub path: PathBuf,
    pub size: (u32, u32),
    pub state: State,
    started: Option<Instant>,
    pub seconds: Option<f32>,
}

impl Item {
    pub fn elapsed(&self) -> Option<f32> {
        match (self.seconds, self.started) {
            (Some(s), _) => Some(s),
            (None, Some(t)) if self.state == State::Rendering => Some(t.elapsed().as_secs_f32()),
            _ => None,
        }
    }
}

pub struct Exporter {
    tx: Sender<Job>,
    rx: Receiver<Update>,
    pub items: Vec<Item>,
    next_id: u64,
}

impl Exporter {
    pub fn spawn(ctx: eframe::egui::Context) -> Self {
        let (tx, jobs) = channel::<Job>();
        let (updates_tx, rx) = channel::<Update>();
        std::thread::Builder::new()
            .name("moth-export".into())
            .spawn(move || {
                for job in jobs {
                    let _ = updates_tx.send(Update::Started(job.id));
                    ctx.request_repaint();
                    let result =
                        export(&job.genome, &job.opts, &job.path).map_err(|e| e.to_string());
                    let _ = updates_tx.send(Update::Finished(job.id, result));
                    ctx.request_repaint();
                }
            })
            .expect("spawn export thread");
        Self {
            tx,
            rx,
            items: Vec::new(),
            next_id: 1,
        }
    }

    pub fn enqueue(&mut self, genome: Genome, opts: ExportOptions, path: PathBuf) {
        let id = self.next_id;
        self.next_id += 1;
        self.items.push(Item {
            id,
            path: path.clone(),
            size: (opts.spec.width, opts.spec.height),
            state: State::Queued,
            started: None,
            seconds: None,
        });
        let _ = self.tx.send(Job {
            id,
            genome,
            opts,
            path,
        });
    }

    /// Applies finished-work messages; returns true if anything changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(update) = self.rx.try_recv() {
            changed = true;
            match update {
                Update::Started(id) => {
                    if let Some(item) = self.items.iter_mut().find(|i| i.id == id) {
                        item.state = State::Rendering;
                        item.started = Some(Instant::now());
                    }
                }
                Update::Finished(id, result) => {
                    if let Some(item) = self.items.iter_mut().find(|i| i.id == id) {
                        item.seconds = item.started.map(|t| t.elapsed().as_secs_f32());
                        item.state = match result {
                            Ok(()) => State::Done,
                            Err(e) => State::Failed(e),
                        };
                    }
                }
            }
        }
        changed
    }

    pub fn busy(&self) -> bool {
        self.items
            .iter()
            .any(|i| matches!(i.state, State::Queued | State::Rendering))
    }
}
