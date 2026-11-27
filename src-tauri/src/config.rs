use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};

pub const DEFAULT_HOST: &str = "broker.hivemq.com";
pub const DEFAULT_PORT: u16 = 1883;
pub const DEFAULT_BASE_TOPIC: &str = "pens/uC2/smartergo";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct AppConfig {
    pub broker_host: String,
    pub broker_port: u16,
    pub base_topic: String,
    pub client_id: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            broker_host: DEFAULT_HOST.to_string(),
            broker_port: DEFAULT_PORT,
            base_topic: DEFAULT_BASE_TOPIC.to_string(),
            client_id: generate_client_id(),
        }
    }
}

pub fn generate_client_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    let suffix = ((nanos ^ pid.wrapping_mul(2654435761)) & 0xffff) as u16;
    format!("SmartErgoApp_{suffix:04x}")
}

pub fn validate_and_normalize(mut cfg: AppConfig) -> Result<AppConfig, String> {
    cfg.broker_host = cfg.broker_host.trim().to_string();
    if cfg.broker_host.is_empty() {
        return Err("Host broker tidak boleh kosong.".into());
    }
    if cfg.broker_host.contains("://") || cfg.broker_host.contains('/') {
        return Err("Host broker cukup nama domain/IP tanpa skema (contoh: broker.hivemq.com).".into());
    }
    if cfg.broker_host.chars().any(char::is_whitespace) {
        return Err("Host broker tidak boleh mengandung spasi.".into());
    }

    if cfg.broker_port == 0 {
        return Err("Port broker harus antara 1 sampai 65535.".into());
    }
    if matches!(cfg.broker_port, 8883 | 8884) {
        return Err("Port TLS/WebSocket (8883/8884) belum didukung; gunakan port TCP polos (mis. 1883).".into());
    }

    cfg.base_topic = cfg.base_topic.trim().trim_matches('/').to_string();
    if cfg.base_topic.is_empty() {
        return Err("Base topic tidak boleh kosong.".into());
    }
    if cfg.base_topic.chars().any(char::is_whitespace)
        || cfg.base_topic.contains('+')
        || cfg.base_topic.contains('#')
    {
        return Err("Base topic tidak boleh mengandung spasi atau wildcard (+/#).".into());
    }

    cfg.client_id = cfg.client_id.trim().to_string();
    if cfg.client_id.is_empty() {
        return Err("Client ID tidak boleh kosong.".into());
    }
    if cfg.client_id.len() > 64 || cfg.client_id.chars().any(char::is_whitespace) {
        return Err("Client ID maksimal 64 karakter dan tanpa spasi.".into());
    }

    Ok(cfg)
}

pub fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("gagal menentukan folder config: {e}"))?;
    Ok(dir.join("config.json"))
}

pub fn load_or_init(app: &AppHandle) -> AppConfig {
    let path = match config_path(app) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return AppConfig::default();
        }
    };

    if path.exists() {
        match fs::read_to_string(&path) {
            Ok(raw) => match serde_json::from_str::<AppConfig>(&raw) {
                Ok(parsed) => match validate_and_normalize(parsed) {
                    Ok(valid) => return valid,
                    Err(e) => eprintln!("config.json tidak valid ({e}), memakai default"),
                },
                Err(e) => eprintln!("gagal parse config.json ({e}), memakai default"),
            },
            Err(e) => eprintln!("gagal membaca config.json ({e}), memakai default"),
        }
        return AppConfig::default();
    }

    let def = AppConfig::default();
    if let Err(e) = write_file(&path, &def) {
        eprintln!("gagal menulis config.json awal: {e}");
    }
    def
}

pub fn save(app: &AppHandle, cfg: AppConfig) -> Result<AppConfig, String> {
    let valid = validate_and_normalize(cfg)?;
    let path = config_path(app)?;
    write_file(&path, &valid)?;
    Ok(valid)
}

fn write_file(path: &PathBuf, cfg: &AppConfig) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("gagal membuat folder config: {e}"))?;
    }
    let json = serde_json::to_string_pretty(cfg)
        .map_err(|e| format!("gagal serialisasi config: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    if fs::write(&tmp, &json).is_ok() && fs::rename(&tmp, path).is_ok() {
        return Ok(());
    }
    let _ = fs::remove_file(&tmp);
    fs::write(path, json).map_err(|e| format!("gagal menyimpan config.json: {e}"))
}
