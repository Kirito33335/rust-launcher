
use eframe::egui;

pub struct FrostApp {
    version: String,
    status: String,
}

impl Default for FrostApp {
    fn default() -> Self {
        Self {
            version: "1.21.11".to_string(),
            status: "FrostClient ist bereit.".to_string(),
        }
    }
}

impl eframe::App for FrostApp {
    fn update(
        &mut self,
        ctx: &egui::Context,
        _frame: &mut eframe::Frame,
    ) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(25.0);

            ui.heading("❄ FROSTCLIENT");
            ui.label("Minecraft Java Edition | PvP & Performance");

            ui.add_space(35.0);
            ui.separator();

            ui.heading("Dashboard");
            ui.label("Willkommen bei FrostClient!");

            ui.add_space(20.0);

            ui.label("Minecraft-Version:");

            egui::ComboBox::from_id_salt("minecraft_version")
                .selected_text(&self.version)
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.version,
                        "1.21.11".to_string(),
                        "1.21.11",
                    );
                });

            ui.add_space(25.0);

            if ui.button("▶  Minecraft starten").clicked() {
                self.status =
                    "Minecraft-Integration kommt als nächster Schritt."
                        .to_string();
            }

            ui.add_space(15.0);

            ui.label(format!("Status: {}", self.status));

            ui.add_space(30.0);
            ui.separator();

            ui.label("FrostClient v0.1.0");
            ui.label("Developed with Rust");
        });
    }
}
