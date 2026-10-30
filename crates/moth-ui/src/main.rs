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
        // The self-test must not take focus from whatever the user is doing. The window opens
        // inactive and off screen, and is never raised.
        viewport = viewport
            .with_active(false)
            .with_taskbar(false)
            .with_position([-3000.0, 100.0]);
    }
    let mut options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    #[cfg(target_os = "macos")]
    if std::env::var_os("MOTH_SMOKE").is_some() {
        // An accessory app has no Dock icon and never becomes the frontmost app.
        options.event_loop_builder = Some(Box::new(|builder| {
            use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
            builder
                .with_activation_policy(ActivationPolicy::Accessory)
                .with_activate_ignoring_other_apps(false);
        }));
    }
    eframe::run_native(
        "Moth",
        options,
        Box::new(|cc| Ok(Box::new(app::MothApp::new(cc)))),
    )
}
