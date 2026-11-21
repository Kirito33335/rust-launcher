#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

mod launcher;

use eframe::egui::{
    self, pos2, vec2, Align, Align2, Color32, FontId, Layout, Margin, Rect, RichText, Rounding,
    Stroke,
};
use launcher::*;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

// ---------- Farben (dunkles Navy-Blau) ----------
const BG_DEEP: Color32 = Color32::from_rgb(4, 10, 26);
const TOPBAR: Color32 = Color32::from_rgb(7, 16, 38);
const PANEL: Color32 = Color32::from_rgb(8, 19, 44);
const SIDEBAR: Color32 = Color32::from_rgb(26, 62, 140);
const CARD: Color32 = Color32::from_rgb(12, 26, 58);
const CARD2: Color32 = Color32::from_rgb(16, 36, 80);
const BORDER: Color32 = Color32::from_rgb(74, 130, 255);
const BORDER_DIM: Color32 = Color32::from_rgb(36, 70, 150);
const TEXT: Color32 = Color32::from_rgb(232, 240, 255);
const DIM: Color32 = Color32::from_rgb(125, 148, 195);
const GREEN: Color32 = Color32::from_rgb(40, 220, 120);
const DANGER: Color32 = Color32::from_rgb(255, 110, 110);

#[derive(Serialize, Deserialize, Clone)]
struct Settings {
    username: String,
    ram_mb: u32,
    java: String,
    show_snapshots: bool,
    last_version: String,
    #[serde(default)]
    auto_start: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            username: "Player".into(),
            ram_mb: 4096,
            java: "java".into(),
            show_snapshots: false,
            last_version: String::new(),
            auto_start: false,
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
    auto_started: bool,
}

// ---------- Theme ----------

fn apply_theme(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG_DEEP;
    v.window_fill = CARD;
    v.extreme_bg_color = Color32::from_rgb(3, 8, 20);
    v.selection.bg_fill = Color32::from_rgb(40, 90, 200);
    v.hyperlink_color = BORDER;
    v.override_text_color = Some(TEXT);

    v.widgets.inactive.weak_bg_fill = CARD2;
    v.widgets.inactive.bg_fill = CARD2;
    v.widgets.inactive.bg_stroke = Stroke::new(1.5, BORDER_DIM);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(24, 52, 112);
    v.widgets.hovered.bg_fill = Color32::from_rgb(24, 52, 112);
    v.widgets.hovered.bg_stroke = Stroke::new(1.5, BORDER);
    v.widgets.active.weak_bg_fill = Color32::from_rgb(34, 70, 150);
    v.widgets.active.bg_fill = Color32::from_rgb(34, 70, 150);
    v.widgets.active.bg_stroke = Stroke::new(1.5, BORDER);
    v.widgets.open.weak_bg_fill = Color32::from_rgb(24, 52, 112);
    v.widgets.open.bg_stroke = Stroke::new(1.5, BORDER);
    for w in [
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.rounding = Rounding::same(12.0);
    }
    v.window_rounding = Rounding::same(12.0);
    v.menu_rounding = Rounding::same(12.0);
    ctx.set_visuals(v);
    ctx.style_mut(|s| {
        s.spacing.item_spacing = vec2(10.0, 10.0);
        s.spacing.button_padding = vec2(14.0, 8.0);
    });
}

fn card(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::none()
        .fill(CARD)
        .stroke(Stroke::new(1.0, BORDER_DIM))
        .rounding(Rounding::same(14.0))
        .inner_margin(Margin::same(18.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
}

// ---------- Zeichnen: Gitter-Raum ----------

fn draw_grid(p: &egui::Painter, rect: Rect) {
    p.rect_filled(rect, Rounding::ZERO, BG_DEEP);
    let hz = rect.min.y + rect.height() * 0.5;
    let cx = rect.center().x;
    let stroke = Stroke::new(1.0, Color32::from_rgba_unmultiplied(60, 110, 230, 70));

    for dir in [1.0f32, -1.0] {
        let edge = if dir > 0.0 { rect.max.y } else { rect.min.y };
        for i in -16..=16 {
            let x = cx + i as f32 * rect.width() / 12.0;
            p.line_segment([pos2(cx + i as f32 * 6.0, hz), pos2(x, edge)], stroke);
        }
        for k in 1..=12 {
            let tt = (k as f32 / 12.0).powi(2);
            let y = hz + (edge - hz) * tt;
            p.line_segment([pos2(rect.min.x, y), pos2(rect.max.x, y)], stroke);
        }
    }
    // Weicher Schleier am Horizont
    p.rect_filled(
        Rect::from_min_max(pos2(rect.min.x, hz - 50.0), pos2(rect.max.x, hz + 50.0)),
        Rounding::ZERO,
        Color32::from_rgba_unmultiplied(4, 10, 26, 150),
    );
    p.line_segment(
        [pos2(rect.min.x, hz), pos2(rect.max.x, hz)],
        Stroke::new(1.5, Color32::from_rgba_unmultiplied(80, 140, 255, 90)),
    );
}

// ---------- Zeichnen: Spielerfigur aus Pixelblöcken ----------

fn blocks(p: &egui::Painter, x: f32, y: f32, gw: i32, gh: i32, base: [i32; 3], seed: i32) {
    let u = 8.0;
    for gy in 0..gh {
        for gx in 0..gw {
            let n = ((gx * 7 + gy * 13 + seed * 5).rem_euclid(5) - 2) * 5;
            let c = Color32::from_rgb(
                (base[0] + n).clamp(0, 255) as u8,
                (base[1] + n).clamp(0, 255) as u8,
                (base[2] + n + n / 2).clamp(0, 255) as u8,
            );
            let r = Rect::from_min_size(
                pos2(x + gx as f32 * u, y + gy as f32 * u),
                vec2(u + 0.6, u + 0.6),
            );
            p.rect_filled(r, Rounding::ZERO, c);
        }
    }
}

fn draw_player(p: &egui::Painter, cx: f32, top: f32, t: f32, name: &str) {
    let u = 8.0;
    let top = top + (t * 2.0).sin() * 3.0;

    // Leuchten hinter der Figur
    for (r, a) in [(210.0, 7u8), (155.0, 9), (110.0, 12)] {
        p.circle_filled(
            pos2(cx, top + 130.0),
            r,
            Color32::from_rgba_unmultiplied(50, 90, 230, a),
        );
    }
    // Schatten
    p.rect_filled(
        Rect::from_center_size(pos2(cx, top + 266.0), vec2(150.0, 12.0)),
        Rounding::same(6.0),
        Color32::from_rgba_unmultiplied(0, 0, 0, 120),
    );

    let swing = (t * 2.0).sin() * 6.0;
    blocks(p, cx - 64.0, top + 64.0 + swing, 4, 12, [26, 30, 44], 3); // linker Arm
    blocks(p, cx + 32.0, top + 64.0 - swing, 4, 12, [26, 30, 44], 4); // rechter Arm
    blocks(p, cx - 32.0, top + 160.0, 4, 12, [20, 24, 38], 5); // linkes Bein
    blocks(p, cx, top + 160.0, 4, 12, [20, 24, 38], 6); // rechtes Bein
    blocks(p, cx - 32.0, top + 64.0, 8, 12, [28, 34, 54], 1); // Körper
    blocks(p, cx - 32.0, top, 8, 8, [30, 34, 48], 2); // Kopf

    // Blaue Akzente auf dem Körper
    for gy in 0..12 {
        for gx in 0..8 {
            if (gx * 5 + gy * 3) % 11 == 0 {
                p.rect_filled(
                    Rect::from_min_size(
                        pos2(cx - 32.0 + gx as f32 * u, top + 64.0 + gy as f32 * u),
                        vec2(u, u),
                    ),
                    Rounding::ZERO,
                    Color32::from_rgb(40, 70, 200),
                );
            }
        }
    }
    // Augen
    for ex in [1.0f32, 5.0] {
        let eye = Rect::from_min_size(pos2(cx - 32.0 + ex * u, top + 3.0 * u), vec2(2.0 * u, u));
        p.rect_stroke(
            eye.expand(2.0),
            Rounding::ZERO,
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(80, 120, 255, 90)),
        );
        p.rect_filled(eye, Rounding::ZERO, Color32::from_rgb(70, 110, 255));
    }

    // Namensschild
    let n = name.chars().count() as f32;
    let w = 64.0 + n * 16.0;
    let tag = Rect::from_center_size(pos2(cx, top - 40.0), vec2(w, 40.0));
    p.rect_filled(tag, Rounding::same(4.0), Color32::from_rgba_unmultiplied(0, 0, 0, 170));
    p.text(
        tag.center(),
        Align2::CENTER_CENTER,
        format!("⚡ {name}"),
        FontId::proportional(24.0),
        Color32::WHITE,
    );
}

// ---------- News-Karten (zeigen echte Daten aus der Versionsliste) ----------

fn news_card(ui: &mut egui::Ui, label: &str, headline: &str, sub: &str, color: Color32, seed: i32) {
    ui.label(RichText::new(label).size(12.0).color(DIM).strong());
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, 140.0), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, Rounding::same(12.0), color);
    // dekorative Blöcke
    for i in 0..5 {
        let s = 16.0 + ((i + seed) % 3) as f32 * 8.0;
        let x = rect.min.x + w * (0.45 + 0.11 * i as f32);
        let y = rect.min.y + 14.0 + ((i * 29 + seed * 17) % 60) as f32;
        p.rect_filled(
            Rect::from_min_size(pos2(x, y), vec2(s, s)),
            Rounding::same(3.0),
            Color32::from_rgba_unmultiplied(255, 255, 255, 28),
        );
    }
    p.rect_stroke(rect, Rounding::same(12.0), Stroke::new(1.5, BORDER_DIM));
    p.text(
        pos2(rect.min.x + 16.0, rect.center().y - 6.0),
        Align2::LEFT_CENTER,
        headline,
        FontId::proportional(30.0),
        Color32::WHITE,
    );
    p.text(
        pos2(rect.min.x + 16.0, rect.max.y - 24.0),
        Align2::LEFT_CENTER,
        sub,
        FontId::proportional(13.0),
        Color32::from_rgb(200, 215, 245),
    );
}

fn nav_button(ui: &mut egui::Ui, icon: &str, selected: bool) -> bool {
    let b = egui::Button::new(
        RichText::new(icon)
            .size(26.0)
            .color(if selected { Color32::WHITE } else { Color32::from_rgb(170, 195, 240) }),
    )
    .fill(if selected { BG_DEEP } else { Color32::TRANSPARENT })
    .stroke(if selected {
        Stroke::new(2.0, Color32::from_rgb(120, 170, 255))
    } else {
        Stroke::NONE
    })
    .rounding(Rounding::same(16.0));
    ui.add_sized([56.0, 56.0], b).clicked()
}

fn version_menu(
    ui: &mut egui::Ui,
    selected: &mut String,
    show_snapshots: &mut bool,
    list: &[VersionEntry],
) {
    ui.set_min_width(230.0);
    ui.checkbox(show_snapshots, "Snapshots / Pre-Releases");
    ui.separator();
    let snaps = *show_snapshots;
    egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
        for v in list
            .iter()
            .filter(|v| v.kind == "release" || (snaps && v.kind == "snapshot"))
        {
            if ui.selectable_label(*selected == v.id, &v.id).clicked() {
                *selected = v.id.clone();
                ui.close_menu();
            }
        }
    });
}

fn open_folder(p: &std::path::Path) {
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer").arg(p).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(p).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(p).spawn();
}

// ---------- App ----------

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
            .user_agent("RustLauncher/0.3")
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
            auto_started: false,
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
                p.set(&format!("Spiel beendet ({status})."));
                Ok(())
            })();
            if let Err(e) = result {
                *err.lock().unwrap() = Some(e.to_string());
                p.set("Fehler");
            }
            busy.store(false, Ordering::SeqCst);
        });
    }

    // ----- Hauptseite: Spielen -----
    fn ui_play(&mut self, ui: &mut egui::Ui, list: &[VersionEntry]) {
        let rect = ui.available_rect_before_wrap();
        ui.allocate_rect(rect, egui::Sense::hover());
        let painter = ui.painter_at(rect);
        draw_grid(&painter, rect);

        let t = ui.input(|i| i.time) as f32;
        let cx = rect.center().x;
        let avail = rect.height() - 150.0;
        let top = rect.min.y + ((avail - 256.0) / 2.0).max(70.0);
        draw_player(&painter, cx, top, t, &self.settings.username);

        let busy = self.busy.load(Ordering::SeqCst);
        let entry = list.iter().find(|v| v.id == self.selected).cloned();

        // Launch-Button + Dropdown
        let sx = cx - 187.0;
        let by = rect.max.y - 140.0;
        let launch_rect = Rect::from_min_size(pos2(sx, by), vec2(300.0, 66.0));
        let dd_rect = Rect::from_min_size(pos2(sx + 308.0, by), vec2(66.0, 66.0));

        let mut job = egui::text::LayoutJob::default();
        job.halign = Align::Center;
        job.append(
            if busy { "LÄUFT …\n" } else { "LAUNCH\n" },
            0.0,
            egui::text::TextFormat {
                font_id: FontId::proportional(22.0),
                color: Color32::WHITE,
                ..Default::default()
            },
        );
        job.append(
            if self.selected.is_empty() { "Lade Versionen …" } else { &self.selected },
            0.0,
            egui::text::TextFormat {
                font_id: FontId::proportional(14.0),
                color: DIM,
                ..Default::default()
            },
        );
        let launch_btn = egui::Button::new(job)
            .fill(CARD2)
            .stroke(Stroke::new(2.0, BORDER))
            .rounding(Rounding::same(18.0))
            .min_size(vec2(300.0, 66.0));
        let resp = ui.put(launch_rect, launch_btn);
        if resp.clicked() && !busy {
            if let Some(e) = entry {
                self.start(e);
            }
        }

        ui.allocate_ui_at_rect(dd_rect, |ui| {
            ui.spacing_mut().button_padding = vec2(24.0, 21.0);
            ui.menu_button(RichText::new("▼").size(18.0), |ui| {
                version_menu(ui, &mut self.selected, &mut self.settings.show_snapshots, list);
            });
        });

        // Status / Fortschritt
        let total = self.progress.total.load(Ordering::SeqCst);
        let done = self.progress.done.load(Ordering::SeqCst);
        let status = self.progress.status.lock().unwrap().clone();
        let error = self.error.lock().unwrap().clone();
        let verr = self.versions_error.lock().unwrap().clone();
        let status_rect = Rect::from_min_size(pos2(sx, by + 76.0), vec2(374.0, 70.0));
        ui.allocate_ui_at_rect(status_rect, |ui| {
            if busy && total > 0 && done < total {
                ui.add(
                    egui::ProgressBar::new(done as f32 / total as f32)
                        .fill(BORDER)
                        .desired_height(18.0)
                        .text(format!("{done} / {total} Dateien")),
                );
            }
            if !status.is_empty() {
                let color = if status.starts_with("Fehler") {
                    DANGER
                } else if status.contains("läuft") {
                    GREEN
                } else {
                    DIM
                };
                ui.label(RichText::new(&status).color(color));
            }
            if let Some(e) = &error {
                ui.colored_label(DANGER, e);
            }
            if let Some(e) = &verr {
                ui.colored_label(DANGER, format!("Versionsliste: {e}"));
                if ui.button("Erneut versuchen").clicked() {
                    self.load_versions();
                }
            }
        });
    }

    // ----- Einstellungen -----
    fn ui_settings(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("EINSTELLUNGEN").size(24.0).strong());

        card(ui, |ui| {
            ui.label(RichText::new("SPIELER").size(12.0).color(DIM).strong());
            ui.add(
                egui::TextEdit::singleline(&mut self.settings.username).desired_width(f32::INFINITY),
            );
            ui.label(RichText::new("Offline-Name, gilt für Singleplayer und Offline-Server.").color(DIM).size(12.0));
        });

        card(ui, |ui| {
            ui.label(RichText::new("ARBEITSSPEICHER").size(12.0).color(DIM).strong());
            ui.add(
                egui::Slider::new(&mut self.settings.ram_mb, 1024..=16384)
                    .step_by(512.0)
                    .suffix(" MB"),
            );
            ui.label(RichText::new("4096 MB reichen für Vanilla meist aus.").color(DIM).size(12.0));
        });

        card(ui, |ui| {
            ui.label(RichText::new("JAVA").size(12.0).color(DIM).strong());
            ui.add(egui::TextEdit::singleline(&mut self.settings.java).desired_width(f32::INFINITY));
            ui.label(RichText::new("Standard: java (aus dem PATH). Minecraft 26.x braucht Java 25.").color(DIM).size(12.0));
        });

        card(ui, |ui| {
            ui.label(RichText::new("DIREKTSTART").size(12.0).color(DIM).strong());
            ui.checkbox(
                &mut self.settings.auto_start,
                "Minecraft sofort starten, sobald der Launcher öffnet",
            );
            ui.label(RichText::new("Nutzt die zuletzt gewählte Version. Tipp: Mit der Verknüpfung \"rust-launcher.exe --play\" startet das Spiel ebenfalls direkt.").color(DIM).size(12.0));
        });

        ui.horizontal(|ui| {
            let save = egui::Button::new(RichText::new("SPEICHERN").strong().color(Color32::WHITE))
                .fill(Color32::from_rgb(40, 90, 200))
                .stroke(Stroke::new(1.5, BORDER));
            if ui.add(save).clicked() {
                self.save_settings();
            }
            if ui.button("Datenordner öffnen").clicked() {
                open_folder(&self.root);
            }
        });
        ui.label(RichText::new(format!("{}", self.root.display())).color(DIM).size(12.0));
    }

    // ----- Info -----
    fn ui_about(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("INFO").size(24.0).strong());
        card(ui, |ui| {
            ui.label(RichText::new("Rust Client").size(20.0).strong());
            ui.label(RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION"))).color(DIM));
            ui.add_space(6.0);
            ui.label("Lädt Minecraft Java Edition von den offiziellen Mojang-Servern.");
            ui.label("Login: aktuell nur Offline-Modus.");
        });
        ui.label(
            RichText::new("Minecraft ist eine Marke von Mojang / Microsoft. Kein offizielles Produkt.")
                .color(DIM)
                .size(12.0),
        );
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let list = self.versions.lock().unwrap().clone();
        if self.selected.is_empty() {
            if let Some(v) = list.iter().find(|v| v.kind == "release") {
                self.selected = v.id.clone();
            }
        }

        // Direktstart: per Einstellung oder mit dem Argument --play
        let cli_play = std::env::args().any(|a| a == "--play");
        if (self.settings.auto_start || cli_play) && !self.auto_started && !self.selected.is_empty() {
            if let Some(e) = list.iter().find(|v| v.id == self.selected).cloned() {
                self.auto_started = true;
                self.start(e);
            }
        }

        let busy = self.busy.load(Ordering::SeqCst);
        if self.page == Page::Play || busy || list.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(33)); // Animation
        }

        // ----- Obere Leiste -----
        egui::TopBottomPanel::top("top")
            .exact_height(66.0)
            .frame(
                egui::Frame::none()
                    .fill(TOPBAR)
                    .stroke(Stroke::new(1.0, BORDER_DIM))
                    .inner_margin(Margin::symmetric(22.0, 10.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.vertical(|ui| {
                        ui.add_space(2.0);
                        ui.label(RichText::new("RUST CLIENT").size(20.0).strong().color(Color32::WHITE));
                        let st = self.progress.status.lock().unwrap().clone();
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            ui.label(RichText::new("●").size(9.0).color(GREEN));
                            ui.label(
                                RichText::new(if busy && !st.is_empty() { st } else { "Bereit".to_string() })
                                    .size(11.0)
                                    .color(DIM),
                            );
                        });
                    });

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        // Konto
                        let name_label = format!("{}  ▾", self.settings.username);
                        ui.menu_button(RichText::new(name_label).strong(), |ui| {
                            ui.label("Offline-Name");
                            ui.text_edit_singleline(&mut self.settings.username);
                            if ui.button("Speichern").clicked() {
                                self.save_settings();
                                ui.close_menu();
                            }
                        });
                        // Version
                        let ver_label = if self.selected.is_empty() {
                            "Version  ▾".to_string()
                        } else {
                            format!("{}  ▾", self.selected)
                        };
                        ui.menu_button(RichText::new(ver_label).strong(), |ui| {
                            version_menu(ui, &mut self.selected, &mut self.settings.show_snapshots, &list);
                        });
                    });
                });
            });

        // ----- Linke Icon-Leiste -----
        egui::SidePanel::left("nav")
            .exact_width(84.0)
            .resizable(false)
            .frame(
                egui::Frame::none()
                    .fill(SIDEBAR)
                    .stroke(Stroke::new(1.0, Color32::from_rgb(60, 110, 210)))
                    .inner_margin(Margin::same(14.0)),
            )
            .show(ctx, |ui| {
                ui.with_layout(Layout::top_down(Align::Center), |ui| {
                    ui.add_space(4.0);
                    ui.label(RichText::new("⚡").size(30.0).color(Color32::WHITE));
                    ui.add_space(14.0);
                    if nav_button(ui, "▶", self.page == Page::Play) {
                        self.page = Page::Play;
                    }
                });
                ui.with_layout(Layout::bottom_up(Align::Center), |ui| {
                    if nav_button(ui, "⚙", self.page == Page::Settings) {
                        self.page = Page::Settings;
                    }
                    if nav_button(ui, "ℹ", self.page == Page::About) {
                        self.page = Page::About;
                    }
                });
            });

        // ----- Rechtes News-Panel (nur auf der Spielen-Seite) -----
        if self.page == Page::Play {
            egui::SidePanel::right("news")
                .exact_width(310.0)
                .resizable(false)
                .frame(
                    egui::Frame::none()
                        .fill(PANEL)
                        .stroke(Stroke::new(1.0, BORDER_DIM))
                        .inner_margin(Margin::same(14.0)),
                )
                .show(ctx, |ui| {
                    ui.label(RichText::new("NEUIGKEITEN").size(16.0).strong());
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        if let Some(v) = list.iter().find(|v| v.kind == "release") {
                            news_card(ui, "NEUESTES RELEASE", &v.id, "Offizielle Version von Mojang", Color32::from_rgb(24, 84, 58), 1);
                        }
                        if let Some(v) = list.iter().find(|v| v.kind == "snapshot") {
                            news_card(ui, "NEUESTER SNAPSHOT", &v.id, "Aktivieren über das Version-Menü", Color32::from_rgb(70, 44, 120), 2);
                        }
                        news_card(ui, "TIPP", "Java 25", "Minecraft 26.x braucht Java 25", Color32::from_rgb(28, 56, 120), 3);
                    });
                });
        }

        // ----- Mitte -----
        let play = self.page == Page::Play;
        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(BG_DEEP)
                    .inner_margin(Margin::same(if play { 0.0 } else { 22.0 })),
            )
            .show(ctx, |ui| match self.page {
                Page::Play => self.ui_play(ui, &list),
                Page::Settings => {
                    egui::ScrollArea::vertical().show(ui, |ui| self.ui_settings(ui));
                }
                Page::About => {
                    egui::ScrollArea::vertical().show(ui, |ui| self.ui_about(ui));
                }
            });
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1150.0, 720.0])
            .with_min_inner_size([940.0, 580.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Rust Client",
        options,
        Box::new(|cc| {
            apply_theme(&cc.egui_ctx);
            Ok(Box::new(App::new()))
        }),
    )
}
