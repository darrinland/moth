//! Stage 2: work on one flame. Live preview in the middle (drag to pan, scroll to
//! zoom), focused editing on the right, undo and redo underneath.

use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, CornerRadius, Key, Rect, Sense, pos2, vec2};
use moth_core::flame::{Genome, Variation, Xform, auto_frame};

use crate::app::{MothApp, Preview};
use crate::gradient::{self, GradientState};
use crate::icons;
use crate::worker::{PreviewSpec, Worker};

const PREVIEW_WIDTH: u32 = 640;
/// How long edits must pause before the finer preview is rendered.
const SETTLE: Duration = Duration::from_millis(350);
const HISTORY_LIMIT: usize = 100;

#[derive(Clone, Copy, PartialEq)]
pub enum Tab {
    Shapes,
    Color,
    Look,
}

pub struct RefineState {
    pub working: Genome,
    committed: Genome,
    undo: Vec<Genome>,
    redo: Vec<Genome>,
    pub tab: Tab,
    sent: Option<Genome>,
    last_change: Instant,
    fine_sent: bool,
    gradient: GradientState,
    palette_seed: u64,
    /// Versions committed by edits since the flame was opened, for the library to record.
    pub edits: Vec<Genome>,
}

impl RefineState {
    pub fn new() -> Self {
        let g = Genome::blank();
        Self {
            working: g.clone(),
            committed: g,
            undo: Vec::new(),
            redo: Vec::new(),
            tab: Tab::Shapes,
            sent: None,
            last_change: Instant::now(),
            fine_sent: true,
            gradient: GradientState::default(),
            palette_seed: 0,
            edits: Vec::new(),
        }
    }

    pub fn begin(&mut self, genome: Genome, seed: u64) {
        self.working = genome.clone();
        self.committed = genome;
        self.undo.clear();
        self.redo.clear();
        self.edits.clear();
        self.sent = None;
        self.palette_seed = seed;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            self.redo
                .push(std::mem::replace(&mut self.committed, prev.clone()));
            self.working = prev;
        }
    }

    pub fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo
                .push(std::mem::replace(&mut self.committed, next.clone()));
            self.working = next;
        }
    }

    /// Records a history step once the user lets go of whatever they were dragging.
    fn commit_history(&mut self, pointer_down: bool) {
        if !pointer_down && self.working != self.committed {
            self.undo
                .push(std::mem::replace(&mut self.committed, self.working.clone()));
            self.edits.push(self.committed.clone());
            if self.undo.len() > HISTORY_LIMIT {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
    }

    fn aspect(&self) -> f32 {
        self.working.camera.size.1 as f32 / self.working.camera.size.0.max(1) as f32
    }

    /// Quick preview on every change, finer one after edits pause.
    fn schedule_preview(&mut self, preview: &mut Preview, worker: &Worker, ctx: &egui::Context) {
        if self.sent.as_ref() != Some(&self.working) {
            preview.request(
                worker,
                &self.working,
                PreviewSpec::fast(PREVIEW_WIDTH, self.aspect()),
            );
            self.sent = Some(self.working.clone());
            self.last_change = Instant::now();
            self.fine_sent = false;
        }
        if !self.fine_sent {
            if self.last_change.elapsed() >= SETTLE {
                preview.request(
                    worker,
                    &self.working,
                    PreviewSpec::fine(PREVIEW_WIDTH, self.aspect()),
                );
                self.fine_sent = true;
            } else {
                ctx.request_repaint_after(SETTLE);
            }
        }
    }
}

/// A slider that only formats numbers for display. egui's `fixed_decimals` and its default
/// clamping silently rewrite the stored value every frame, which would quietly alter a
/// saved recipe (and create phantom undo steps) just by looking at it.
fn slider(
    value: &mut f64,
    range: std::ops::RangeInclusive<f64>,
    decimals: usize,
) -> egui::Slider<'_> {
    egui::Slider::new(value, range)
        .clamping(egui::SliderClamping::Edits)
        .custom_formatter(move |v, _| format!("{v:.decimals$}"))
}

/// The same for drag boxes: display rounding only, and no clamping of existing values.
fn drag(
    value: &mut f64,
    speed: f64,
    range: std::ops::RangeInclusive<f64>,
    decimals: usize,
) -> egui::DragValue<'_> {
    egui::DragValue::new(value)
        .speed(speed)
        .range(range)
        .clamp_existing_to_range(false)
        .custom_formatter(move |v, _| format!("{v:.decimals$}"))
}

pub fn ui(app: &mut MothApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let MothApp {
        refine,
        preview,
        worker,
        ..
    } = app;

    // Undo / redo shortcuts.
    if !ctx.egui_wants_keyboard_input() {
        let (z, shift) = ctx.input(|i| {
            (
                i.modifiers.command && i.key_pressed(Key::Z),
                i.modifiers.shift,
            )
        });
        if z {
            if shift { refine.redo() } else { refine.undo() }
        }
    }

    egui::Panel::right("refine-tools")
        .default_size(360.0)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut refine.tab, Tab::Shapes, "Shapes");
                ui.selectable_value(&mut refine.tab, Tab::Color, "Color");
                ui.selectable_value(&mut refine.tab, Tab::Look, "Look & frame");
            });
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| match refine.tab {
                Tab::Shapes => shapes_tab(ui, &mut refine.working),
                Tab::Color => {
                    gradient::editor(
                        ui,
                        &mut refine.working,
                        &mut refine.gradient,
                        &mut refine.palette_seed,
                    );
                }
                Tab::Look => look_tab(ui, &mut refine.working, &mut refine.palette_seed),
            });
        });

    egui::CentralPanel::default().show(ui, |ui| preview_area(ui, refine, preview));

    let pointer_down = ctx.input(|i| i.pointer.any_down());
    refine.commit_history(pointer_down);
    refine.schedule_preview(preview, worker, &ctx);
}

fn preview_area(ui: &mut egui::Ui, refine: &mut RefineState, preview: &Preview) {
    let aspect = refine.aspect();
    let avail = ui.available_size();
    let (mut w, mut h) = (avail.x, avail.x * aspect);
    if h > avail.y {
        (h, w) = (avail.y, avail.y / aspect);
    }
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click_and_drag());
    let painter = ui.painter();
    match &preview.texture {
        Some(tex) => {
            painter.image(
                tex.id(),
                rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        None => {
            painter.rect_filled(rect, CornerRadius::same(4), Color32::from_gray(24));
        }
    }
    if preview.pending {
        ui.put(
            Rect::from_min_size(rect.left_top() + vec2(8.0, 8.0), vec2(18.0, 18.0)),
            egui::Spinner::new(),
        );
    }
    if resp.hovered() || resp.dragged() {
        ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Grab);
    }

    let cam = &mut refine.working.camera;
    let ppu = cam.scale * (rect.width() / cam.size.0.max(1) as f32) as f64;
    if resp.dragged() && ppu > 0.0 {
        let d = resp.drag_delta();
        let (sin, cos) = cam.rotate.to_radians().sin_cos();
        let (dx, dy) = (d.x as f64, d.y as f64);
        cam.center.0 -= (dx * cos + dy * sin) / ppu;
        cam.center.1 -= (-dx * sin + dy * cos) / ppu;
    }
    if resp.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            cam.scale = (cam.scale * (scroll as f64 * 0.003).exp()).clamp(1.0, 100_000.0);
        }
    }
    ui.add_space(4.0);
    ui.weak("Drag to move · scroll to zoom");
}

fn shapes_tab(ui: &mut egui::Ui, g: &mut Genome) {
    let count = g.xforms.len();
    let (mut remove, mut duplicate) = (None, None);
    for (i, x) in g.xforms.iter_mut().enumerate() {
        let title = format!(
            "Transform {}  ·  {}",
            i + 1,
            x.variations
                .iter()
                .map(|(v, _)| v.name())
                .collect::<Vec<_>>()
                .join(", ")
        );
        egui::CollapsingHeader::new(title)
            .id_salt(("xform", i))
            .default_open(i == 0)
            .show(ui, |ui| {
                xform_editor(ui, x);
                ui.horizontal(|ui| {
                    if ui.button("Duplicate").clicked() {
                        duplicate = Some(i);
                    }
                    if ui
                        .add_enabled(count > 1, egui::Button::new("Delete"))
                        .clicked()
                    {
                        remove = Some(i);
                    }
                });
            });
    }
    if let Some(i) = duplicate {
        let copy = g.xforms[i].clone();
        g.xforms.insert(i + 1, copy);
    }
    if let Some(i) = remove {
        g.xforms.remove(i);
    }

    ui.add_space(6.0);
    if ui.button("+ Add transform").clicked() {
        let mut x = Xform::linear();
        x.color = (g.xforms.len() % 5) as f64 / 4.0;
        x.affine.a = 0.6;
        x.affine.e = 0.6;
        x.affine.c = 0.3;
        g.xforms.push(x);
    }

    ui.add_space(8.0);
    ui.separator();
    match &mut g.final_xform {
        Some(f) => {
            let mut remove_final = false;
            egui::CollapsingHeader::new("Final transform")
                .default_open(false)
                .show(ui, |ui| {
                    ui.weak("Warps the finished picture without feeding back into the shapes.");
                    xform_editor(ui, f);
                    if ui.button("Remove final transform").clicked() {
                        remove_final = true;
                    }
                });
            if remove_final {
                g.final_xform = None;
            }
        }
        None => {
            if ui.button("+ Add final transform").clicked() {
                let mut f = Xform::linear();
                f.color_speed = 0.0;
                g.final_xform = Some(f);
            }
        }
    }
}

fn xform_editor(ui: &mut egui::Ui, x: &mut Xform) {
    egui::Grid::new(ui.next_auto_id())
        .num_columns(2)
        .spacing([10.0, 4.0])
        .show(ui, |ui| {
            ui.label("Weight");
            ui.add(slider(&mut x.weight, 0.0..=3.0, 2));
            ui.end_row();
            ui.label("Color");
            ui.add(slider(&mut x.color, 0.0..=1.0, 2));
            ui.end_row();
            ui.label("Color blend");
            ui.add(slider(&mut x.color_speed, 0.0..=1.0, 2));
            ui.end_row();
        });
    ui.add_space(4.0);
    ui.label("Position  (x′ = a·x + b·y + c,  y′ = d·x + e·y + f)");
    let a = &mut x.affine;
    egui::Grid::new(ui.next_auto_id())
        .num_columns(3)
        .spacing([6.0, 4.0])
        .show(ui, |ui| {
            for row in [
                [&mut a.a, &mut a.b, &mut a.c],
                [&mut a.d, &mut a.e, &mut a.f],
            ] {
                for v in row {
                    ui.add(drag(v, 0.005, -8.0..=8.0, 3));
                }
                ui.end_row();
            }
        });
    ui.add_space(4.0);
    ui.label("Variations");
    let mut remove = None;
    let only_one = x.variations.len() == 1;
    for (i, (v, w)) in x.variations.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.add_sized([86.0, 18.0], egui::Label::new(v.name()));
            ui.add(slider(w, 0.0..=2.0, 2));
            if ui
                .add_enabled(!only_one, egui::Button::new(icons::X).small())
                .clicked()
            {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        x.variations.remove(i);
    }
    let missing: Vec<Variation> = Variation::ALL
        .iter()
        .copied()
        .filter(|v| !x.variations.iter().any(|(have, _)| have == v))
        .collect();
    egui::ComboBox::from_id_salt(ui.next_auto_id())
        .selected_text("+ add variation")
        .show_ui(ui, |ui| {
            for v in missing {
                if ui.selectable_label(false, v.name()).clicked() {
                    x.variations.push((v, 0.5));
                }
            }
        });
}

fn look_tab(ui: &mut egui::Ui, g: &mut Genome, seed: &mut u64) {
    ui.label("Tone");
    egui::Grid::new("look-tone")
        .num_columns(2)
        .spacing([10.0, 4.0])
        .show(ui, |ui| {
            ui.label("Brightness");
            ui.add(slider(&mut g.brightness, 0.5..=14.0, 2));
            ui.end_row();
            ui.label("Gamma");
            ui.add(slider(&mut g.gamma, 1.0..=6.0, 2));
            ui.end_row();
            ui.label("Vibrancy");
            ui.add(slider(&mut g.vibrancy, 0.0..=1.0, 2));
            ui.end_row();
        });
    ui.add_space(8.0);
    ui.separator();
    ui.label("Frame");
    egui::Grid::new("look-frame")
        .num_columns(2)
        .spacing([10.0, 4.0])
        .show(ui, |ui| {
            ui.label("Zoom");
            ui.add(slider(&mut g.camera.scale, 10.0..=20000.0, 0).logarithmic(true));
            ui.end_row();
            ui.label("Rotate");
            ui.add(slider(&mut g.camera.rotate, -180.0..=180.0, 0).suffix("°"));
            ui.end_row();
            ui.label("Center");
            ui.horizontal(|ui| {
                ui.add(drag(&mut g.camera.center.0, 0.01, f64::MIN..=f64::MAX, 3));
                ui.add(drag(&mut g.camera.center.1, 0.01, f64::MIN..=f64::MAX, 3));
            });
            ui.end_row();
        });
    if ui
        .button("Auto-frame")
        .on_hover_text("Centre the picture and fit it in the frame")
        .clicked()
    {
        *seed += 1;
        auto_frame(g, *seed);
    }
}
