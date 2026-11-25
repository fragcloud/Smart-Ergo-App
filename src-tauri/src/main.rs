#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, PhysicalSize, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};

const MAIN_WINDOW: &str = "main";
const SETTINGS_WINDOW: &str = "settings";

/// Fase 1: simpan objek tray supaya tetap hidup selama aplikasi berjalan.
#[derive(Clone)]
struct TrayHandle(tauri::tray::TrayIcon);

fn show_window(app: &AppHandle, label: &str) {
    if let Some(w) = app.get_webview_window(label) {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn open_settings(app: &AppHandle) {
    if app.get_webview_window(SETTINGS_WINDOW).is_some() {
        show_window(app, SETTINGS_WINDOW);
        return;
    }
    let win = WebviewWindowBuilder::new(app, SETTINGS_WINDOW, WebviewUrl::App("settings.html".into()))
        .title("Pengaturan Smart Ergo")
        .inner_size(520.0, 560.0)
        .min_inner_size(480.0, 520.0)
        .resizable(true)
        .build();
    if let Err(e) = win {
        eprintln!("gagal membuat window pengaturan: {e}");
    }
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // Panggilan kedua pada .exe: jangan buat proses baru, cukup tampilkan
            // window utama dari instance yang sudah berjalan.
            show_window(app, MAIN_WINDOW);
        }))
        .setup(|app| {
            // Dashboard default 1440x900, tetapi jangan melebihi layar monitor.
            if let Some(w) = app.get_webview_window(MAIN_WINDOW) {
                if let Ok(Some(mon)) = w.current_monitor() {
                    let size = mon.size();
                    if size.width < 1440 || size.height < 900 {
                        let width = size.width.saturating_sub(48).max(1024);
                        let height = size.height.saturating_sub(72).max(700);
                        let _ = w.set_size(PhysicalSize::new(width, height));
                    }
                }
            }

            let open_dashboard = MenuItemBuilder::with_id("open_dashboard", "Buka Dashboard").build(app)?;
            let open_settings = MenuItemBuilder::with_id("open_settings", "Pengaturan").build(app)?;
            let ui_alarm = MenuItemBuilder::with_id("ui_alarm", "Uji Alarm").build(app)?;
            let status = MenuItemBuilder::with_id("status", "Status: app berjalan").build(app)?;
            let quit = MenuItemBuilder::with_id("quit", "Keluar").build(app)?;

            let menu = MenuBuilder::new(app)
                .items(&[&open_dashboard, &open_settings, &ui_alarm, &status, &quit])
                .build()?;

            let default_icon = app
                .default_window_icon()
                .expect("ikon default belum tersedia — pastikan icons/ lengkap")
                .clone();

            let tray = TrayIconBuilder::with_id("main")
                .menu(&menu)
                .tooltip("Smart Ergo — Kendali & Telemetri")
                .show_menu_on_left_click(false)
                .icon(default_icon)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "open_dashboard" => show_window(app, MAIN_WINDOW),
                    "open_settings" => open_settings(app),
                    "ui_alarm" => {
                        let _ = app.tray_by_id("main").map(|t| {
                            let _ = t.set_tooltip(Some("Uji Alarm aktif di rilis berikutnya".into()));
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

            app.manage(TrayHandle(tray));

            Ok(())
        })
        .on_window_event(|window, event| {
            // Menutup dashboard = menyembunyikan window, bukan berhenti.
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == MAIN_WINDOW {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("gagal membangun aplikasi Tauri")
        .run(|_app, _event| {});
}