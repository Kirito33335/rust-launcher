#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

mod launcher;

use eframe::egui;
use launcher::*;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Serialize, Deserialize, Clone)]
struct Settings {
    username: String,
    ram_mb: u32,
    java: String,
    show_snapshots: bool,
    last_version: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            username: "Player".into(),
            ram_mb: 4096,
            java: "java".into(),
            show_snapshots: false,
            last_version: String::new(),
        }
    }
}

#[derive(PartialEq, Clone, Copy)]
enum Page {
    Play,
    Settings,
    About,
}

struct App {
    client: Client,
    root: PathBuf,
    settings: Settings,
    page: Page,
    versions: Arc<Mutex<Vec<VersionEntry>>>,
    versions_error: Arc<Mutex<Option<String>>>,
    selected: String,
    progress: Arc<Progress>,
    busy: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
}

impl App {
    fn new() -> Self {
        let root = directories::ProjectDirs::from("de", "local", "RustLauncher")
            .map(|d| d.data_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("rust-launcher-data"));
        let _ = std::fs::create_dir_all(&root);

        let settings: Settings = std::fs::read_to_string(root.join("settings.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();

        let client = Client::builder()
            .user_agent("RustLauncher/0.1")
            .timeout(Duration::from_secs(60))
            .build()
            .expect("HTTP-Client");

        let app = Self {
            client,
            root,
            selected: settings.last_version.clone(),
            settings,
            page: Page::Play,
            versions: Arc::new(Mutex::new(Vec::new())),
            versions_error: Arc::new(Mutex::new(None)),
            progress: Arc::new(Progress::default()),
            busy: Arc::new(AtomicBool::new(false)),
            error: Arc::new(Mutex::new(None)),
        };
        app.load_versions();
        app
    }

    fn load_versions(&self) {
        let (c, vs, err) = (self.client.clone(), self.versions.clone(), self.versions_error.clone());
        *err.lock().unwrap() = None;
        std::thread::spawn(move || match fetch_versions(&c) {
            Ok(list) => *vs.lock().unwrap() = list,
            Err(e) => *err.lock().unwrap() = Some(e.to_string()),
        });
    }

    fn save_settings(&mut self) {
        self.settings.last_version = self.selected.clone();
        if let Ok(s) = serde_json::to_string_pretty(&self.settings) {
            let _ = std::fs::write(self.root.join("settings.json"), s);
        }
    }

    fn start(&mut self, entry: VersionEntry) {
        self.save_settings();
        self.busy.store(true, Ordering::SeqCst);
        *self.error.lock().unwrap() = None;

        let (c, root, p, busy, err) = (
            self.client.clone(),
            self.root.clone(),
            self.progress.clone(),
            self.busy.clone(),
            self.error.clone(),
        );
        let opts = LaunchOpts {
            java: self.settings.java.trim().to_string(),
            username: self.settings.username.trim().to_string(),
            ram_mb: self.settings.ram_mb,
        };

        std::thread::spawn(move || {
            let result = (|| -> Res<()> {
                let v = prepare(&c, &root, &entry, &p)?;
                p.reset();
                p.set("Starte Minecraft …");
                let mut child = launch(&root, &v, &opts)?;
                p.set("Minecraft läuft.");
                let status = child.wait()?;
                p.set(&format!(
                    "Spiel beendet ({status}). Log: game/launcher_log.txt"
                ));
                Ok(())
            })();
            if let Err(e) = result {
                *err.lock().unwrap() = Some(e.to_string());
                p.set("Fehler");
            }
            busy.store(false, Ordering::SeqCst);
        });
    }

    fn ui_play(&mut self, ui: &mut egui::Ui) {
        ui.heading("Spielen");
        ui.add_space(8.0);

        let list = self.versions.lock().unwrap().clone();
        if let Some(e) = self.versions_error.lock().unwrap().clone() {
            ui.colored_label(egui::Color32::LIGHT_RED, format!("Versionsliste: {e}"));
            if ui.button("Erneut versuchen").clicked() {
                self.load_versions();
            }
        }

        // Standardauswahl: neueste Release-Version
        if self.selected.is_empty() {
            if let Some(v) = list.iter().find(|v| v.kind == "release") {
                self.selected = v.id.clone();
            }
        }

        ui.horizontal(|ui| {
            ui.label("Version:");
            egui::ComboBox::from_id_salt("version")
                .selected_text(&self.selected)
                .width(220.0)
                .show_ui(ui, |ui| {
                    for v in list.iter().filter(|v| {
                        v.kind == "release" || (self.settings.show_snapshots && v.kind == "snapshot")
                    }) {
                        ui.selectable_value(&mut self.selected, v.id.clone(), &v.id);
                    }
                });
            ui.checkbox(&mut self.settings.show_snapshots, "Snapshots / Pre-Releases");
        });

        ui.add_space(16.0);
        let busy = self.busy.load(Ordering::SeqCst);
        let entry = list.iter().find(|v| v.id == self.selected).cloned();

        let play = ui.add_enabled(
            !busy && entry.is_some(),
            egui::Button::new(egui::RichText::new("▶  Spielen").size(22.0))
                .min_size(egui::vec2(240.0, 52.0)),
        );
        if play.clicked() {
            if let Some(e) = entry {
                self.start(e);
            }
        }

        ui.add_space(12.0);
        let total = self.progress.total.load(Ordering::SeqCst);
        let done = self.progress.done.load(Ordering::SeqCst);
        if busy && total > 0 && done < total {
            ui.add(
                egui::ProgressBar::new(done as f32 / total as f32)
                    .show_percentage()
                    .text(format!("{done} / {total} Dateien")),
            );
        }
        let status = self.progress.status.lock().unwrap().clone();
        if !status.is_empty() {
            ui.label(status);
        }
        if let Some(e) = self.error.lock().unwrap().clone() {
            ui.colored_label(egui::Color32::LIGHT_RED, e);
        }
    }

    fn ui_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Einstellungen");
        ui.add_space(8.0);
        egui::Grid::new("settings").num_columns(2).spacing([16.0, 10.0]).show(ui, |ui| {
            ui.label("Spielername (Offline):");
            ui.text_edit_singleline(&mut self.settings.username);
            ui.end_row();

            ui.label("Arbeitsspeicher:");
            ui.add(
                egui::Slider::new(&mut self.settings.ram_mb, 1024..=16384)
                    .step_by(512.0)
                    .suffix(" MB"),
            );
            ui.end_row();

            ui.label("Java-Pfad:");
            ui.text_edit_singleline(&mut self.settings.java);
            ui.end_row();
        });
        ui.add_space(8.0);
        ui.weak("Neue Versionen brauchen aktuelles Java (26.x: Java 25).");
        ui.weak(format!("Datenordner: {}", self.root.display()));
        ui.add_space(12.0);
        if ui.button("Speichern").clicked() {
            self.save_settings();
        }
    }

    fn ui_about(&mut self, ui: &mut egui::Ui) {
        ui.heading("Info");
        ui.add_space(8.0);
        ui.label("Rust Launcher 0.1");
        ui.label("Lädt Minecraft Java Edition von den offiziellen Mojang-Servern.");
        ui.label("Login: aktuell nur Offline-Modus (Singleplayer / Offline-Server).");
        ui.add_space(8.0);
        ui.weak("Minecraft ist eine Marke von Mojang / Microsoft. Kein offizielles Produkt.");
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.busy.load(Ordering::SeqCst) || self.versions.lock().unwrap().is_empty() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        egui::SidePanel::left("menu").exact_width(170.0).show(ctx, |ui| {
            ui.add_space(12.0);
            ui.heading("⛏ Rust Launcher");
            ui.add_space(16.0);
            for (page, label) in [
                (Page::Play, "▶  Spielen"),
                (Page::Settings, "⚙  Einstellungen"),
                (Page::About, "ℹ  Info"),
            ] {
                if ui
                    .add_sized([150.0, 34.0], egui::SelectableLabel::new(self.page == page, label))
                    .clicked()
                {
                    self.page = page;
                }
            }
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(8.0);
            match self.page {
                Page::Play => self.ui_play(ui),
                Page::Settings => self.ui_settings(ui),
                Page::About => self.ui_about(ui),
            }
        });
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([780.0, 500.0])
            .with_min_inner_size([640.0, 420.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Rust Launcher",
        options,
        Box::new(|cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            Ok(Box::new(App::new()))
        }),
    )
}
