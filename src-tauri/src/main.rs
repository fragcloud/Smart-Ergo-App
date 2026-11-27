#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod mqtt;

use config::AppConfig;
use mqtt::{AlarmPayload, MqttManager, MqttStatusSnapshot, TrayUi};
use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, LogicalSize, Manager, State, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};

const MAIN_WINDOW: &str = "main";
const SETTINGS_WINDOW: &str = "settings";
const ALARM_WINDOW: &str = "alarm";

struct AppState {
    mqtt: MqttManager,
    _tray_ui: TrayUi,
}

fn show_window(app: &AppHandle, label: &str) {
    if let Some(w) = app.get_webview_window(label) {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn open_settings(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(SETTINGS_WINDOW) {
        show_window(app, SETTINGS_WINDOW);
        let _ = w.emit("settings-refresh", ());
        return;
    }
    // Fallback bila window belum ada: bangun di thread terpisah agar tidak deadlock WebView2.
    let handle = app.clone();
    std::thread::spawn(move || {
        let win = WebviewWindowBuilder::new(
            &handle,
            SETTINGS_WINDOW,
            WebviewUrl::App("settings.html".into()),
        )
        .title("Pengaturan Smart Ergo")
        .inner_size(520.0, 620.0)
        .min_inner_size(480.0, 540.0)
        .resizable(true)
        .center()
        .build();
        if let Err(e) = win {
            eprintln!("gagal membuat window pengaturan: {e}");
        }
    });
}

#[tauri::command]
async fn get_config(state: State<'_, AppState>) -> Result<AppConfig, String> {
    Ok(state.mqtt.get_config().await)
}

#[tauri::command]
async fn save_config(
    app: AppHandle,
    state: State<'_, AppState>,
    cfg: AppConfig,
) -> Result<AppConfig, String> {
    let saved = config::save(&app, cfg)?;
    state.mqtt.apply_config(saved.clone()).await;
    Ok(saved)
}

#[tauri::command]
async fn reset_config(app: AppHandle, state: State<'_, AppState>) -> Result<AppConfig, String> {
    let saved = config::save(&app, AppConfig::default())?;
    state.mqtt.apply_config(saved.clone()).await;
    Ok(saved)
}

#[tauri::command]
async fn get_mqtt_status(state: State<'_, AppState>) -> Result<MqttStatusSnapshot, String> {
    Ok(state.mqtt.get_status().await)
}

#[tauri::command]
async fn reconnect_mqtt(state: State<'_, AppState>) -> Result<(), String> {
    state.mqtt.trigger_reconnect();
    Ok(())
}

#[tauri::command]
async fn publish_cmd(
    state: State<'_, AppState>,
    cmd: String,
    val: serde_json::Value,
) -> Result<(), String> {
    state.mqtt.publish_cmd(&cmd, val).await
}

#[tauri::command]
async fn get_alarm_state(state: State<'_, AppState>) -> Result<AlarmPayload, String> {
    Ok(state.mqtt.get_alarm_state().await)
}

#[tauri::command]
async fn ack_alarm(app: AppHandle, state: State<'_, AppState>) -> Result<bool, String> {
    state.mqtt.ack_alarm(&app).await
}

#[tauri::command]
async fn open_settings_window(app: AppHandle) {
    open_settings(&app);
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_window(app, MAIN_WINDOW);
        }))
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![
            get_config,
            save_config,
            reset_config,
            get_mqtt_status,
            reconnect_mqtt,
            publish_cmd,
            get_alarm_state,
            ack_alarm,
            open_settings_window,
        ])
        .setup(|app| {
            // Dashboard default 1440x900 (logis), tetapi jangan melebihi work area monitor.
            if let Some(w) = app.get_webview_window(MAIN_WINDOW) {
                if let Ok(Some(mon)) = w.current_monitor() {
                    let area = mon.work_area().size.to_logical::<u32>(mon.scale_factor());
                    if area.width < 1440 || area.height < 900 {
                        let width = area.width.saturating_sub(48).max(1024).min(1440);
                        let height = area.height.saturating_sub(48).max(700).min(900);
                        let _ = w.set_size(LogicalSize::new(width, height));
                        let _ = w.center();
                    }
                }
            }

            let item_dashboard = MenuItemBuilder::with_id("open_dashboard", "Buka Dashboard").build(app)?;
            let item_settings = MenuItemBuilder::with_id("open_settings", "Pengaturan").build(app)?;
            let item_alarm = MenuItemBuilder::with_id("ui_alarm", "Uji Alarm").build(app)?;
            let item_status = MenuItemBuilder::with_id("status", "Status: menghubungkan ke broker...")
                .enabled(false)
                .build(app)?;
            let item_quit = MenuItemBuilder::with_id("quit", "Keluar").build(app)?;

            let menu = MenuBuilder::new(app)
                .items(&[&item_dashboard, &item_settings, &item_alarm, &item_status, &item_quit])
                .build()?;

            let default_icon = app
                .default_window_icon()
                .expect("ikon default belum tersedia — pastikan icons/ lengkap")
                .clone();

            let tray = TrayIconBuilder::with_id("main")
                .menu(&menu)
                .tooltip("Smart Ergo — Menghubungkan ke broker...")
                .show_menu_on_left_click(false)
                .icon(default_icon)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "open_dashboard" => show_window(app, MAIN_WINDOW),
                    "open_settings" => open_settings(app),
                    "ui_alarm" => {
                        let handle = app.clone();
                        let mgr = app.state::<AppState>().mqtt.clone();
                        tauri::async_runtime::spawn(async move {
                            mgr.trigger_test_alarm(&handle).await;
                        });
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_window(tray.app_handle(), MAIN_WINDOW);
                    }
                })
                .build(app)?;

            let initial_cfg = config::load_or_init(app.handle());
            let mqtt_mgr = MqttManager::new(initial_cfg);
            let tray_ui = TrayUi {
                tray,
                status_item: item_status,
            };

            app.manage(AppState {
                mqtt: mqtt_mgr.clone(),
                _tray_ui: tray_ui.clone(),
            });

            mqtt::spawn_worker(app.handle().clone(), mqtt_mgr, tray_ui);

            Ok(())
        })
        .on_window_event(|window, event| {
            // Menutup dashboard/pengaturan = sembunyikan; alarm hanya ditutup lewat tombol ACK.
            if let WindowEvent::CloseRequested { api, .. } = event {
                if matches!(window.label(), MAIN_WINDOW | SETTINGS_WINDOW) {
                    api.prevent_close();
                    let _ = window.hide();
                } else if window.label() == ALARM_WINDOW {
                    api.prevent_close();
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("gagal membangun aplikasi Tauri")
        .run(|_app, _event| {});
}
