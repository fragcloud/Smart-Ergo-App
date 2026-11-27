# Smart Ergo App

GUI kendali & telemetri **Smart Ergo** (Paket 4 — Workshop Mikrokontroler Lanjut, PENS).
Aplikasi desktop Windows (Tauri 2) yang berjalan di latar dengan tray icon, dashboard
monitoring & kendali dua node ESP32, window pengaturan terpisah, notifikasi toast Windows,
serta pop-up alarm *always-on-top*.

Kelompok 2 · 2026

## Struktur

```
src/                 frontend statis offline-safe (tanpa bundler & tanpa CDN)
  index.html         dashboard kendali & telemetri (Chart.js + font lokal)
  settings.html      window pengaturan broker MQTT
  alarm.html         pop-up alarm frameless always-on-top
  vendor/            Chart.js 4.4.4 UMD + font Inter & IBM Plex Mono (.woff2)
  assets/icon.png    sumber ikon 1024×1024
src-tauri/           backend Rust
  src/main.rs        tray icon, lifecycle window, single-instance, command IPC
  src/config.rs      persistensi & validasi %APPDATA%\id.pens.smartergo\config.json
  src/mqtt.rs        worker MQTT rumqttc latar, watchdog node, logika alarm & toast
.github/workflows/   check.yml (cargo check) + release.yml (installer NSIS)
```

Konfigurasi aplikasi: `src-tauri/tauri.conf.json`.
Installer dihasilkan di GitHub Actions (NSIS, per-user, tanpa hak admin).

## Build

- **Cek cepat (compile saja):** push ke `main` → workflow `Check`.
- **Installer:** push tag `v*` → workflow `Release` → unduh dari halaman GitHub
  Releases. Contoh: `git tag v1.0.0 && git push origin v1.0.0`.
- Installer unsigned akan memunculkan "Windows protected your PC — Unknown
  publisher". Klik *More info → Run anyway*. Verifikasi integritas lewat hash
  SHA-256 di `checksums.txt` pada halaman rilis.

## Menjalankan

Install `.exe`, lalu jalankan. Aplikasi muncul di tray:

- **Buka Dashboard** — tampilkan window utama (menutupnya hanya menyembunyikan ke tray).
- **Pengaturan** — ubah host broker, port TCP, base topic, dan client ID (`config.json`).
- **Uji Alarm** — tampilkan pop-up alarm uji dan notifikasi toast Windows.
- **Status** — status koneksi broker dan keaktifan node Vital/Ergo secara langsung.
- **Keluar** — satu-satunya cara menghentikan aplikasi.

## Perangkat demo (virtual device)

Berada di `workshop mikro lanjut dan interface\Smart Ergo Simulasi\` — halaman HTML
terpisah (tidak ikut dibundel) yang mensimulasikan dua node ESP32 di atas broker
MQTT sungguhan. Jalankan lewat server HTTP lokal (`python -m http.server`) untuk
menguji seluruh alur telemetri, kendali, ACK, dan pop-up alarm tanpa hardware.
