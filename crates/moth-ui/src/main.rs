mod app;
mod worker;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1360.0, 860.0])
            .with_min_inner_size([900.0, 560.0])
            .with_title("Moth"),
        ..Default::default()
    };
    eframe::run_native(
        "Moth",
        options,
        Box::new(|cc| Ok(Box::new(app::MothApp::new(cc)))),
    )
}
