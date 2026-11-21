//! Launcher-Logik: Mojang-Manifest lesen, Dateien laden, Java-Prozess starten.

use reqwest::blocking::Client;
use serde::Deserialize;
use serde_json::Value;
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

pub type Res<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
const ASSET_URL: &str = "https://resources.download.minecraft.net";
const DOWNLOAD_THREADS: usize = 16;

// ---------- Versionsliste ----------

#[derive(Deserialize, Clone)]
pub struct VersionEntry {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub url: String,
}

#[derive(Deserialize)]
struct Manifest {
    versions: Vec<VersionEntry>,
}

pub fn fetch_versions(c: &Client) -> Res<Vec<VersionEntry>> {
    let m: Manifest = c.get(MANIFEST_URL).send()?.error_for_status()?.json()?;
    Ok(m.versions)
}

// ---------- Fortschritt ----------

#[derive(Default)]
pub struct Progress {
    pub status: Mutex<String>,
    pub done: AtomicUsize,
    pub total: AtomicUsize,
}

impl Progress {
    pub fn set(&self, s: &str) {
        *self.status.lock().unwrap() = s.to_string();
    }
    pub fn reset(&self) {
        self.done.store(0, Ordering::SeqCst);
        self.total.store(0, Ordering::SeqCst);
    }
}

// ---------- Download ----------

struct Job {
    url: String,
    path: PathBuf,
    size: Option<u64>,
}

fn download_one(c: &Client, j: &Job) -> Res<()> {
    if let Ok(m) = fs::metadata(&j.path) {
        if j.size.map_or(true, |s| s == m.len()) {
            return Ok(());
        }
    }
    if let Some(dir) = j.path.parent() {
        fs::create_dir_all(dir)?;
    }
    let bytes = c.get(&j.url).send()?.error_for_status()?.bytes()?;
    let tmp = j.path.with_extension("part");
    fs::write(&tmp, &bytes)?;
    fs::rename(&tmp, &j.path)?;
    Ok(())
}

fn download_all(client: &Client, jobs: Vec<Job>, p: &Arc<Progress>) -> Res<()> {
    p.total.store(jobs.len(), Ordering::SeqCst);
    p.done.store(0, Ordering::SeqCst);
    let queue = Arc::new(Mutex::new(jobs));
    let error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let mut handles = Vec::new();

    for _ in 0..DOWNLOAD_THREADS {
        let (q, c, p, error) = (queue.clone(), client.clone(), p.clone(), error.clone());
        handles.push(thread::spawn(move || loop {
            if error.lock().unwrap().is_some() {
                return;
            }
            let job = q.lock().unwrap().pop();
            let Some(job) = job else { return };
            // Bis zu 3 Versuche pro Datei
            let mut last = None;
            for _ in 0..3 {
                match download_one(&c, &job) {
                    Ok(()) => {
                        last = None;
                        break;
                    }
                    Err(e) => last = Some(e.to_string()),
                }
            }
            if let Some(e) = last {
                *error.lock().unwrap() = Some(format!("{}: {e}", job.url));
                return;
            }
            p.done.fetch_add(1, Ordering::SeqCst);
        }));
    }
    for h in handles {
        let _ = h.join();
    }
    let e = error.lock().unwrap().take();
    match e {
        Some(e) => Err(e.into()),
        None => Ok(()),
    }
}

// ---------- Regeln (OS-spezifische Libraries und Argumente) ----------

fn os_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "osx"
    } else {
        "linux"
    }
}

fn arch_name() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        a => a,
    }
}

fn rules_ok(rules: Option<&Value>) -> bool {
    let Some(arr) = rules.and_then(|r| r.as_array()) else {
        return true;
    };
    let mut allowed = false;
    for r in arr {
        let mut matches = true;
        if let Some(os) = r.get("os") {
            if let Some(n) = os.get("name").and_then(|v| v.as_str()) {
                if n != os_name() {
                    matches = false;
                }
            }
            if let Some(a) = os.get("arch").and_then(|v| v.as_str()) {
                if a != arch_name() {
                    matches = false;
                }
            }
            if os.get("version").is_some() {
                matches = false;
            }
        }
        if r.get("features").is_some() {
            matches = false; // Demo, eigene Auflösung usw. nutzen wir nicht
        }
        if matches {
            allowed = r.get("action").and_then(|v| v.as_str()) == Some("allow");
        }
    }
    allowed
}

fn subst(s: &str, vars: &HashMap<&str, String>) -> String {
    let mut out = s.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("${{{k}}}"), v);
    }
    out
}

fn collect_args(v: &Value, which: &str, vars: &HashMap<&str, String>) -> Vec<String> {
    let mut out = Vec::new();
    let Some(arr) = v
        .pointer(&format!("/arguments/{which}"))
        .and_then(|a| a.as_array())
    else {
        return out;
    };
    for a in arr {
        match a {
            Value::String(s) => out.push(subst(s, vars)),
            Value::Object(o) => {
                if !rules_ok(o.get("rules")) {
                    continue;
                }
                match o.get("value") {
                    Some(Value::String(s)) => out.push(subst(s, vars)),
                    Some(Value::Array(l)) => {
                        for x in l.iter().filter_map(|x| x.as_str()) {
                            out.push(subst(x, vars));
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    out
}

// ---------- Vorbereiten: alles herunterladen ----------

pub fn prepare(c: &Client, root: &Path, entry: &VersionEntry, p: &Arc<Progress>) -> Res<Value> {
    p.reset();
    p.set("Lade Versionsdaten …");
    let vdir = root.join("versions").join(&entry.id);
    fs::create_dir_all(&vdir)?;

    let text = c.get(&entry.url).send()?.error_for_status()?.text()?;
    fs::write(vdir.join(format!("{}.json", entry.id)), &text)?;
    let v: Value = serde_json::from_str(&text)?;

    let mut jobs: Vec<Job> = Vec::new();

    // Client-JAR
    let cl = v.pointer("/downloads/client").ok_or("Kein Client-Download in dieser Version")?;
    jobs.push(Job {
        url: cl["url"].as_str().ok_or("Client-URL fehlt")?.into(),
        path: vdir.join(format!("{}.jar", entry.id)),
        size: cl["size"].as_u64(),
    });

    // Libraries (ab 1.19 sind die LWJGL-Natives eigene Library-Einträge)
    for lib in v["libraries"].as_array().into_iter().flatten() {
        if !rules_ok(lib.get("rules")) {
            continue;
        }
        if let Some(a) = lib.pointer("/downloads/artifact") {
            if let (Some(url), Some(path)) = (a["url"].as_str(), a["path"].as_str()) {
                jobs.push(Job {
                    url: url.into(),
                    path: root.join("libraries").join(path),
                    size: a["size"].as_u64(),
                });
            }
        }
    }

    // Assets
    p.set("Lade Asset-Index …");
    let ai = &v["assetIndex"];
    let id = ai["id"].as_str().ok_or("assetIndex.id fehlt")?;
    let idx_path = root.join("assets").join("indexes").join(format!("{id}.json"));
    fs::create_dir_all(idx_path.parent().unwrap())?;
    let idx_text = c
        .get(ai["url"].as_str().ok_or("assetIndex.url fehlt")?)
        .send()?
        .error_for_status()?
        .text()?;
    fs::write(&idx_path, &idx_text)?;
    let idx: Value = serde_json::from_str(&idx_text)?;
    for (_, o) in idx["objects"].as_object().into_iter().flatten() {
        if let Some(h) = o["hash"].as_str() {
            if h.len() < 2 {
                continue;
            }
            jobs.push(Job {
                url: format!("{ASSET_URL}/{}/{h}", &h[..2]),
                path: root.join("assets").join("objects").join(&h[..2]).join(h),
                size: o["size"].as_u64(),
            });
        }
    }

    p.set(&format!("Lade {} Dateien (nur fehlende) …", jobs.len()));
    download_all(c, jobs, p)?;
    Ok(v)
}

// ---------- Starten ----------

pub struct LaunchOpts {
    pub java: String,
    pub username: String,
    pub ram_mb: u32,
}

fn offline_uuid(name: &str) -> String {
    let h = Sha1::digest(format!("OfflinePlayer:{name}").as_bytes());
    let hex: String = h[..16].iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

pub fn launch(root: &Path, v: &Value, o: &LaunchOpts) -> Res<Child> {
    let id = v["id"].as_str().ok_or("Versions-ID fehlt")?;
    let vdir = root.join("versions").join(id);
    let natives = vdir.join("natives");
    fs::create_dir_all(&natives)?;
    let game_dir = root.join("game");
    fs::create_dir_all(&game_dir)?;

    // Classpath
    let sep = if cfg!(windows) { ";" } else { ":" };
    let mut cp: Vec<String> = Vec::new();
    for lib in v["libraries"].as_array().into_iter().flatten() {
        if !rules_ok(lib.get("rules")) {
            continue;
        }
        if let Some(path) = lib.pointer("/downloads/artifact/path").and_then(|p| p.as_str()) {
            cp.push(root.join("libraries").join(path).to_string_lossy().into_owned());
        }
    }
    cp.push(vdir.join(format!("{id}.jar")).to_string_lossy().into_owned());
    let classpath = cp.join(sep);

    let mut vars: HashMap<&str, String> = HashMap::new();
    vars.insert("auth_player_name", o.username.clone());
    vars.insert("version_name", id.to_string());
    vars.insert("game_directory", game_dir.to_string_lossy().into_owned());
    vars.insert("assets_root", root.join("assets").to_string_lossy().into_owned());
    vars.insert("assets_index_name", v["assetIndex"]["id"].as_str().unwrap_or("").to_string());
    vars.insert("auth_uuid", offline_uuid(&o.username));
    vars.insert("auth_access_token", "0".into());
    vars.insert("clientid", "".into());
    vars.insert("auth_xuid", "".into());
    vars.insert("user_type", "legacy".into());
    vars.insert("user_properties", "{}".into());
    vars.insert("version_type", v["type"].as_str().unwrap_or("release").to_string());
    vars.insert("natives_directory", natives.to_string_lossy().into_owned());
    vars.insert("launcher_name", "RustLauncher".into());
    vars.insert("launcher_version", env!("CARGO_PKG_VERSION").into());
    vars.insert("classpath", classpath.clone());

    let mut jvm = collect_args(v, "jvm", &vars);
    if jvm.is_empty() {
        jvm = vec![
            format!("-Djava.library.path={}", natives.display()),
            "-cp".into(),
            classpath,
        ];
    }
    let mut game = collect_args(v, "game", &vars);
    if game.is_empty() {
        if let Some(s) = v["minecraftArguments"].as_str() {
            game = s.split_whitespace().map(|a| subst(a, &vars)).collect();
        }
    }
    let main_class = v["mainClass"].as_str().ok_or("mainClass fehlt")?;

    let log = fs::File::create(game_dir.join("launcher_log.txt"))?;
    let mut cmd = Command::new(&o.java);
    cmd.arg(format!("-Xmx{}M", o.ram_mb))
        .arg(format!("-Xms{}M", (o.ram_mb / 4).max(512)))
        .args(jvm)
        .arg(main_class)
        .args(game)
        .current_dir(&game_dir)
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // kein schwarzes Konsolenfenster
    }
    cmd.spawn().map_err(|e| {
        format!("Java konnte nicht gestartet werden ('{}'): {e}", o.java).into()
    })
}
