
mod launcher;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([950.0, 600.0])
            .with_title("FrostClient"),
        ..Default::default()
    };

    eframe::run_native(
        "FrostClient",
        options,
        Box::new(|cc| {
            cc.egui_ctx.set_visuals(eframe::egui::Visuals::dark());
            Ok(Box::new(launcher::FrostApp::default()))
        }),
    )
}
