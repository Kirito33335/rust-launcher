# Rust Launcher

Minecraft-Launcher mit grafischer Oberfläche (egui). Lädt Minecraft Java Edition
von den offiziellen Mojang-Servern (parallel, 16 Threads) und startet es.

## Voraussetzungen

1. Rust: https://rustup.rs
2. Java 25 (für Minecraft 26.x), z. B. Temurin: https://adoptium.net
3. Linux zusätzlich: `sudo apt install build-essential pkg-config libxkbcommon-dev libwayland-dev libx11-dev libxcursor-dev libxi-dev libgl1-mesa-dev`

## Bauen und starten

```
cargo run --release
```

Fertige Datei zum Weitergeben (liegt danach in `target/release/`):

```
cargo build --release
```

- Windows: `rust-launcher.exe`
- Linux/macOS: `rust-launcher`

## Benutzung

1. Version wählen (für Pre-Releases wie 26.3-pre-1 "Snapshots / Pre-Releases" anhaken).
2. Unter Einstellungen Name, RAM und ggf. den Java-Pfad setzen.
3. "Spielen" klicken. Beim ersten Start werden alle Dateien geladen.

Daten liegen im Benutzerverzeichnis (Pfad steht in den Einstellungen).
Spiel-Log: `game/launcher_log.txt`.

## Noch nicht enthalten

- Microsoft-Login (nötig für Online-Server, braucht eine eigene Azure-App-ID)
- Fabric/Mods/Profile (wie bei NoRisk), automatischer Java-Download
- Alte Versionen (< 1.19) mit separaten Natives-JARs
