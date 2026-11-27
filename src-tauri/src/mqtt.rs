use crate::config::AppConfig;
use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, QoS, SubscribeFilter};
use serde::Serialize;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::menu::MenuItem;
use tauri::tray::TrayIcon;
use tauri::{AppHandle, Emitter, Manager, Wry};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::{watch, RwLock};

const NODE_TIMEOUT_MS: u64 = 5_000;
const ALARM_COOLDOWN_MS: u64 = 3_000;
const RECONNECT_DELAY: Duration = Duration::from_secs(3);
const WATCHDOG_INTERVAL: Duration = Duration::from_secs(2);
const ALARM_WINDOW: &str = "alarm";

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

#[derive(Debug, Clone, Serialize)]
pub struct AlarmPayload {
    pub active: bool,
    pub is_test: bool,
    pub ergo_online: bool,
    pub broker_connected: bool,
    pub device: String,
    pub jarak: Option<f64>,
    pub accel_x: Option<f64>,
    pub reason: String,
    pub triggered_ms: u64,
}

impl Default for AlarmPayload {
    fn default() -> Self {
        Self {
            active: false,
            is_test: false,
            ergo_online: false,
            broker_connected: false,
            device: "Node Ergo".to_string(),
            jarak: None,
            accel_x: None,
            reason: "Parameter ergonomi melewati ambang batas.".to_string(),
            triggered_ms: 0,
        }
    }
}

struct AlarmTracker {
    popup_open: bool,
    is_test: bool,
    last_ack_ms: u64,
    toast_sent_episode: bool,
    current: AlarmPayload,
}

impl Default for AlarmTracker {
    fn default() -> Self {
        Self {
            popup_open: false,
            is_test: false,
            last_ack_ms: 0,
            toast_sent_episode: false,
            current: AlarmPayload::default(),
        }
    }
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
    alarm: Arc<RwLock<AlarmTracker>>,
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
            alarm: Arc::new(RwLock::new(AlarmTracker::default())),
            reload_tx,
        }
    }

    pub async fn get_config(&self) -> AppConfig {
        self.config.read().await.clone()
    }

    pub async fn get_status(&self) -> MqttStatusSnapshot {
        self.snapshot.read().await.clone()
    }

    pub async fn get_alarm_state(&self) -> AlarmPayload {
        self.alarm.read().await.current.clone()
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

    pub async fn ack_alarm(&self, app: &AppHandle) -> Result<bool, String> {
        let (was_test, payload_after) = {
            let mut al = self.alarm.write().await;
            let was_test = al.is_test;
            al.popup_open = false;
            al.is_test = false;
            al.current.active = false;
            al.current.is_test = false;
            (was_test, al.current.clone())
        };

        hide_alarm_window(app, &payload_after);

        if was_test {
            return Ok(false);
        }

        let (ergo_online, broker_connected) = {
            let s = self.snapshot.read().await;
            (s.ergo_online, s.state == BrokerState::Connected)
        };

        if ergo_online && broker_connected {
            {
                let mut al = self.alarm.write().await;
                al.last_ack_ms = now_ms();
                al.toast_sent_episode = false;
            }
            let _ = app.emit(
                "mqtt-cmd-sent",
                serde_json::json!({ "cmd": "ack_alarm", "val": 1 }),
            );
            let _ = self.publish_cmd("ack_alarm", serde_json::json!(1)).await;
            return Ok(true);
        }

        Ok(false)
    }

    pub async fn trigger_test_alarm(&self, app: &AppHandle) {
        let (ergo_online, broker_connected) = {
            let s = self.snapshot.read().await;
            (s.ergo_online, s.state == BrokerState::Connected)
        };

        let payload = {
            let mut al = self.alarm.write().await;
            al.popup_open = true;
            al.is_test = true;
            al.current = AlarmPayload {
                active: true,
                is_test: true,
                ergo_online,
                broker_connected,
                device: "Uji Tray".to_string(),
                jarak: Some(18.5),
                accel_x: Some(2.45),
                reason: "Simulasi uji pop-up alarm dari menu tray.".to_string(),
                triggered_ms: now_ms(),
            };
            al.current.clone()
        };

        show_alarm_window(app, &payload);
        send_toast(
            app,
            "Uji Alarm Smart Ergo",
            "Pop-up alarm dan notifikasi Windows aktif.",
        );
    }

    async fn handle_ergo_alarm(
        &self,
        app: &AppHandle,
        raw: &str,
        ts: u64,
        broker_connected: bool,
    ) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) else {
            return;
        };
        let alarm_flag = v.get("alarm").and_then(|x| x.as_bool()).unwrap_or(false);
        let device = v
            .get("device")
            .and_then(|x| x.as_str())
            .unwrap_or("Node Ergo")
            .to_string();
        let jarak = v.get("jarak").and_then(|x| x.as_f64());
        let accel_x = v.get("accel_x").and_then(|x| x.as_f64());

        let mut parts = Vec::new();
        if let Some(j) = jarak {
            if (0.0..30.0).contains(&j) {
                parts.push(format!("jarak {j:.1} cm (< 30 cm)"));
            }
        }
        if let Some(ax) = accel_x {
            if ax.abs() >= 2.0 {
                parts.push(format!("kemiringan {:.2} m/s² (≥ 2,0)", ax.abs()));
            }
        }
        let reason = if parts.is_empty() {
            "Parameter ergonomi melewati ambang batas.".to_string()
        } else {
            format!("Terdeteksi {}.", parts.join(" dan "))
        };

        let mut al = self.alarm.write().await;
        if alarm_flag {
            if !al.popup_open && ts.saturating_sub(al.last_ack_ms) >= ALARM_COOLDOWN_MS {
                al.popup_open = true;
                al.is_test = false;
                al.current = AlarmPayload {
                    active: true,
                    is_test: false,
                    ergo_online: true,
                    broker_connected,
                    device,
                    jarak,
                    accel_x,
                    reason: reason.clone(),
                    triggered_ms: ts,
                };
                show_alarm_window(app, &al.current);
                if !al.toast_sent_episode {
                    al.toast_sent_episode = true;
                    send_toast(app, "Alarm Smart Ergo", &reason);
                }
            } else if al.popup_open && !al.is_test {
                al.current.ergo_online = true;
                al.current.broker_connected = broker_connected;
                al.current.device = device;
                al.current.jarak = jarak;
                al.current.accel_x = accel_x;
                al.current.reason = reason;
                emit_alarm_state(app, &al.current);
            }
        } else {
            al.toast_sent_episode = false;
            if al.popup_open && !al.is_test {
                al.popup_open = false;
                al.current.active = false;
                hide_alarm_window(app, &al.current);
            }
        }
    }

    async fn sync_alarm_connectivity(
        &self,
        app: &AppHandle,
        ergo_online: bool,
        broker_connected: bool,
    ) {
        let mut al = self.alarm.write().await;
        if al.popup_open && !al.is_test {
            al.current.ergo_online = ergo_online;
            al.current.broker_connected = broker_connected;
            emit_alarm_state(app, &al.current);
        }
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

fn emit_alarm_state(app: &AppHandle, payload: &AlarmPayload) {
    if let Some(w) = app.get_webview_window(ALARM_WINDOW) {
        let _ = w.emit("alarm-state", payload);
    }
}

fn show_alarm_window(app: &AppHandle, payload: &AlarmPayload) {
    if let Some(w) = app.get_webview_window(ALARM_WINDOW) {
        let _ = w.set_always_on_top(true);
        let _ = w.center();
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
        let _ = w.emit("alarm-state", payload);
    }
}

fn hide_alarm_window(app: &AppHandle, payload: &AlarmPayload) {
    if let Some(w) = app.get_webview_window(ALARM_WINDOW) {
        let _ = w.hide();
        let _ = w.emit("alarm-state", payload);
    }
}

fn send_toast(app: &AppHandle, title: &str, body: &str) {
    let _ = app.notification().builder().title(title).body(body).show();
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
                        let (dirty, ergo_online, broker_connected) = {
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
                            (dirty, s.ergo_online, s.state == BrokerState::Connected)
                        };
                        if dirty {
                            mgr.sync_alarm_connectivity(&app, ergo_online, broker_connected).await;
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

                                let broker_connected = {
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
                                    s.state == BrokerState::Connected
                                };

                                if kind == "ergo" {
                                    mgr.handle_ergo_alarm(&app, &payload, ts, broker_connected).await;
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
                                {
                                    let mut s = mgr.snapshot.write().await;
                                    s.state = BrokerState::Error;
                                    s.error = Some(e.to_string());
                                    s.vital_online = false;
                                    s.ergo_online = false;
                                    emit_status(&app, &tray_ui, &s);
                                }
                                mgr.sync_alarm_connectivity(&app, false, false).await;
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
