mod app;
mod explore;
mod exporter;
mod gradient;
mod history;
mod icons;
mod refine;
mod render_stage;
mod smoke;
mod worker;

const ICON_PNG: &[u8] = include_bytes!("../assets/icon/moth-256.png");

fn main() -> eframe::Result {
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size([1360.0, 860.0])
        .with_min_inner_size([900.0, 560.0])
        .with_title("Moth");
    if let Ok(icon) = eframe::icon_data::from_png_bytes(ICON_PNG) {
        viewport = viewport.with_icon(icon);
    }
    if std::env::var_os("MOTH_SMOKE").is_some() {
        // A covered window stops repainting, which would stall the self-test.
        viewport = viewport.with_always_on_top();
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "Moth",
        options,
        Box::new(|cc| Ok(Box::new(app::MothApp::new(cc)))),
    )
}
