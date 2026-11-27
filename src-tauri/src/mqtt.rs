use crate::config::AppConfig;
use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, QoS, SubscribeFilter};
use serde::Serialize;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::menu::MenuItem;
use tauri::tray::TrayIcon;
use tauri::{AppHandle, Emitter, Wry};
use tokio::sync::{watch, RwLock};

const NODE_TIMEOUT_MS: u64 = 5_000;
const RECONNECT_DELAY: Duration = Duration::from_secs(3);
const WATCHDOG_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BrokerState {
    Connecting,
    Connected,
    Reconnecting,
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct MqttStatusSnapshot {
    pub state: BrokerState,
    pub host: String,
    pub port: u16,
    pub base_topic: String,
    pub client_id: String,
    pub error: Option<String>,
    pub vital_online: bool,
    pub ergo_online: bool,
    pub vital_count: u64,
    pub ergo_count: u64,
    pub ack_count: u64,
    pub last_vital_ms: Option<u64>,
    pub last_ergo_ms: Option<u64>,
    pub last_vital_raw: Option<String>,
    pub last_ergo_raw: Option<String>,
    pub last_ack_raw: Option<String>,
}

impl MqttStatusSnapshot {
    fn from_config(cfg: &AppConfig) -> Self {
        Self {
            state: BrokerState::Connecting,
            host: cfg.broker_host.clone(),
            port: cfg.broker_port,
            base_topic: cfg.base_topic.clone(),
            client_id: cfg.client_id.clone(),
            error: None,
            vital_online: false,
            ergo_online: false,
            vital_count: 0,
            ergo_count: 0,
            ack_count: 0,
            last_vital_ms: None,
            last_ergo_ms: None,
            last_vital_raw: None,
            last_ergo_raw: None,
            last_ack_raw: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MqttMessageEvent {
    pub topic: String,
    pub kind: String,
    pub payload: String,
    pub ts_ms: u64,
}

#[derive(Clone)]
pub struct TrayUi {
    pub tray: TrayIcon,
    pub status_item: MenuItem<Wry>,
}

impl TrayUi {
    pub fn sync(&self, snap: &MqttStatusSnapshot) {
        let (menu_text, tooltip) = match snap.state {
            BrokerState::Connecting => (
                "Status: menghubungkan ke broker...".to_string(),
                format!("Smart Ergo — Menghubungkan ke {}:{}", snap.host, snap.port),
            ),
            BrokerState::Reconnecting => (
                "Status: menyambung ulang...".to_string(),
                format!("Smart Ergo — Menyambung ulang ke {}:{}", snap.host, snap.port),
            ),
            BrokerState::Error => (
                "Status: broker terputus".to_string(),
                format!(
                    "Smart Ergo — Terputus ({})",
                    snap.error.as_deref().unwrap_or("galat koneksi")
                ),
            ),
            BrokerState::Connected => match (snap.vital_online, snap.ergo_online) {
                (true, true) => (
                    "Status: terhubung · Vital & Ergo aktif".to_string(),
                    format!("Smart Ergo — Terhubung ({}:{}) · Vital & Ergo aktif", snap.host, snap.port),
                ),
                (true, false) => (
                    "Status: terhubung · Vital aktif".to_string(),
                    format!("Smart Ergo — Terhubung ({}:{}) · Hanya Vital aktif", snap.host, snap.port),
                ),
                (false, true) => (
                    "Status: terhubung · Ergo aktif".to_string(),
                    format!("Smart Ergo — Terhubung ({}:{}) · Hanya Ergo aktif", snap.host, snap.port),
                ),
                (false, false) => (
                    "Status: broker terhubung (menunggu node)".to_string(),
                    format!("Smart Ergo — Broker terhubung ({}:{}), menunggu node", snap.host, snap.port),
                ),
            },
        };
        let _ = self.status_item.set_text(menu_text);
        let _ = self.tray.set_tooltip(Some(tooltip.as_str()));
    }
}

#[derive(Clone)]
pub struct MqttManager {
    config: Arc<RwLock<AppConfig>>,
    snapshot: Arc<RwLock<MqttStatusSnapshot>>,
    client: Arc<RwLock<Option<AsyncClient>>>,
    reload_tx: watch::Sender<u64>,
}

impl MqttManager {
    pub fn new(initial: AppConfig) -> Self {
        let snap = MqttStatusSnapshot::from_config(&initial);
        let (reload_tx, _) = watch::channel(0u64);
        Self {
            config: Arc::new(RwLock::new(initial)),
            snapshot: Arc::new(RwLock::new(snap)),
            client: Arc::new(RwLock::new(None)),
            reload_tx,
        }
    }

    pub async fn get_config(&self) -> AppConfig {
        self.config.read().await.clone()
    }

    pub async fn get_status(&self) -> MqttStatusSnapshot {
        self.snapshot.read().await.clone()
    }

    pub async fn apply_config(&self, cfg: AppConfig) {
        {
            let mut w = self.config.write().await;
            *w = cfg.clone();
        }
        {
            let mut s = self.snapshot.write().await;
            *s = MqttStatusSnapshot::from_config(&cfg);
        }
        self.trigger_reconnect();
    }

    pub fn trigger_reconnect(&self) {
        let next = self.reload_tx.borrow().wrapping_add(1);
        let _ = self.reload_tx.send(next);
    }

    pub async fn publish_cmd(&self, cmd: &str, val: serde_json::Value) -> Result<(), String> {
        validate_cmd(cmd, &val)?;

        let (client, base_topic, connected) = {
            let c = self.client.read().await.clone();
            let s = self.snapshot.read().await;
            (c, s.base_topic.clone(), s.state == BrokerState::Connected)
        };

        let client = match (client, connected) {
            (Some(c), true) => c,
            _ => return Err("Broker MQTT belum terhubung.".into()),
        };

        let payload = serde_json::json!({ "cmd": cmd, "val": val }).to_string();
        let topic = format!("{base_topic}/cmd");
        client
            .publish(topic, QoS::AtMostOnce, false, payload.into_bytes())
            .await
            .map_err(|e| format!("gagal mengirim perintah MQTT: {e}"))
    }
}

fn validate_cmd(cmd: &str, val: &serde_json::Value) -> Result<(), String> {
    let n = val
        .as_i64()
        .ok_or_else(|| format!("nilai untuk '{cmd}' harus bilangan bulat"))?;
    match cmd {
        "led" | "buzzer" => {
            if n == 0 || n == 1 {
                Ok(())
            } else {
                Err(format!("nilai '{cmd}' harus 0 atau 1"))
            }
        }
        "thr_bpm" => {
            if (40..=200).contains(&n) {
                Ok(())
            } else {
                Err("ambang BPM harus antara 40 sampai 200".into())
            }
        }
        "interval" => {
            if (100..=2000).contains(&n) {
                Ok(())
            } else {
                Err("periode pengiriman harus antara 100 sampai 2000 ms".into())
            }
        }
        "ack_alarm" => {
            if n == 1 {
                Ok(())
            } else {
                Err("nilai 'ack_alarm' harus 1".into())
            }
        }
        _ => Err(format!("perintah '{cmd}' tidak dikenal")),
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn emit_status(app: &AppHandle, tray_ui: &TrayUi, snap: &MqttStatusSnapshot) {
    tray_ui.sync(snap);
    let _ = app.emit("mqtt-status", snap);
}

pub fn spawn_worker(app: AppHandle, mgr: MqttManager, tray_ui: TrayUi) {
    tauri::async_runtime::spawn(async move {
        let mut reload_rx = mgr.reload_tx.subscribe();

        loop {
            let cfg = mgr.get_config().await;
            {
                let mut s = mgr.snapshot.write().await;
                s.state = BrokerState::Connecting;
                s.host = cfg.broker_host.clone();
                s.port = cfg.broker_port;
                s.base_topic = cfg.base_topic.clone();
                s.client_id = cfg.client_id.clone();
                s.error = None;
                emit_status(&app, &tray_ui, &s);
            }

            let mut opts = MqttOptions::new(
                cfg.client_id.clone(),
                cfg.broker_host.clone(),
                cfg.broker_port,
            );
            opts.set_keep_alive(Duration::from_secs(20));
            opts.set_clean_session(true);

            let (client, mut eventloop) = AsyncClient::new(opts, 64);
            {
                let mut w = mgr.client.write().await;
                *w = Some(client.clone());
            }

            let mut watchdog = tokio::time::interval(WATCHDOG_INTERVAL);
            watchdog.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            let mut should_delay = false;

            loop {
                tokio::select! {
                    changed = reload_rx.changed() => {
                        if changed.is_ok() {
                            let _ = client.disconnect().await;
                            break;
                        }
                        return;
                    }
                    _ = watchdog.tick() => {
                        let now = now_ms();
                        let mut s = mgr.snapshot.write().await;
                        let mut dirty = false;
                        if s.vital_online && s.last_vital_ms.is_some_and(|t| now.saturating_sub(t) > NODE_TIMEOUT_MS) {
                            s.vital_online = false;
                            dirty = true;
                        }
                        if s.ergo_online && s.last_ergo_ms.is_some_and(|t| now.saturating_sub(t) > NODE_TIMEOUT_MS) {
                            s.ergo_online = false;
                            dirty = true;
                        }
                        if dirty {
                            emit_status(&app, &tray_ui, &s);
                        }
                    }
                    ev = eventloop.poll() => {
                        match ev {
                            Ok(Event::Incoming(Incoming::ConnAck(_))) => {
                                let filters = [
                                    SubscribeFilter::new(format!("{}/vital", cfg.base_topic), QoS::AtMostOnce),
                                    SubscribeFilter::new(format!("{}/ergo", cfg.base_topic), QoS::AtMostOnce),
                                    SubscribeFilter::new(format!("{}/ack", cfg.base_topic), QoS::AtMostOnce),
                                ];
                                if let Err(e) = client.subscribe_many(filters).await {
                                    let mut s = mgr.snapshot.write().await;
                                    s.state = BrokerState::Error;
                                    s.error = Some(format!("gagal subscribe: {e}"));
                                    emit_status(&app, &tray_ui, &s);
                                    should_delay = true;
                                    break;
                                }
                                let mut s = mgr.snapshot.write().await;
                                s.state = BrokerState::Connected;
                                s.error = None;
                                emit_status(&app, &tray_ui, &s);
                            }
                            Ok(Event::Incoming(Incoming::Publish(p))) => {
                                let ts = now_ms();
                                let payload = String::from_utf8_lossy(&p.payload).into_owned();
                                let kind = if p.topic.ends_with("/vital") {
                                    "vital"
                                } else if p.topic.ends_with("/ergo") {
                                    "ergo"
                                } else if p.topic.ends_with("/ack") {
                                    "ack"
                                } else {
                                    "other"
                                };

                                {
                                    let mut s = mgr.snapshot.write().await;
                                    let mut status_changed = false;
                                    match kind {
                                        "vital" => {
                                            s.vital_count += 1;
                                            s.last_vital_ms = Some(ts);
                                            s.last_vital_raw = Some(payload.clone());
                                            if !s.vital_online {
                                                s.vital_online = true;
                                                status_changed = true;
                                            }
                                        }
                                        "ergo" => {
                                            s.ergo_count += 1;
                                            s.last_ergo_ms = Some(ts);
                                            s.last_ergo_raw = Some(payload.clone());
                                            if !s.ergo_online {
                                                s.ergo_online = true;
                                                status_changed = true;
                                            }
                                        }
                                        "ack" => {
                                            s.ack_count += 1;
                                            s.last_ack_raw = Some(payload.clone());
                                        }
                                        _ => {}
                                    }
                                    if status_changed {
                                        emit_status(&app, &tray_ui, &s);
                                    }
                                }

                                let _ = app.emit(
                                    "mqtt-message",
                                    MqttMessageEvent {
                                        topic: p.topic,
                                        kind: kind.to_string(),
                                        payload,
                                        ts_ms: ts,
                                    },
                                );
                            }
                            Ok(_) => {}
                            Err(e) => {
                                let mut s = mgr.snapshot.write().await;
                                s.state = BrokerState::Error;
                                s.error = Some(e.to_string());
                                s.vital_online = false;
                                s.ergo_online = false;
                                emit_status(&app, &tray_ui, &s);
                                should_delay = true;
                                break;
                            }
                        }
                    }
                }
            }

            {
                let mut w = mgr.client.write().await;
                *w = None;
            }

            if should_delay {
                {
                    let mut s = mgr.snapshot.write().await;
                    s.state = BrokerState::Reconnecting;
                    emit_status(&app, &tray_ui, &s);
                }
                tokio::select! {
                    _ = tokio::time::sleep(RECONNECT_DELAY) => {}
                    changed = reload_rx.changed() => {
                        if changed.is_err() {
                            return;
                        }
                    }
                }
            }
        }
    });
}
