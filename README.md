# Smart Ergo App

GUI kendali & telemetri **Smart Ergo** (Paket 4 — Workshop Mikrokontroler Lanjut, PENS).
Aplikasi desktop Windows (Tauri 2) yang berjalan di latar dengan tray icon; dashboard
hanyalah window yang boleh dibuka-tutup tanpa mematikan aplikasi.

Kelompok 2 · 2026

## Struktur

```
src/                 frontend statis (di-serve langsung, tanpa bundler)
  index.html         dashboard (shell Fase 1; port penuh di Fase 3)
  settings.html      window pengaturan (shell Fase 1; penuh di Fase 2)
  assets/icon.png    sumber ikon 1024×1024
src-tauri/           backend Rust (tray, lifecycle window, MQTT di fase berikutnya)
.github/workflows/   check.yml (cargo check) + release.yml (installer NSIS)
```

Konfigurasi aplikasi: `src-tauri/tauri.conf.json`.
Installer dihasilkan di GitHub Actions (NSIS, per-user, tanpa hak admin).

## Build

- **Cek cepat (compile saja):** push ke `main` → workflow `Check`.
- **Installer:** push tag `v*` → workflow `Release` → unduh dari halaman GitHub
  Releases. Contoh: `git tag v0.1.0 && git push origin v0.1.0`.
- Installer unsigned akan memunculkan "Windows protected your PC — Unknown
  publisher". Klik *More info → Run anyway*. Verifikasi integritas lewat hash
  SHA-256 di `checksums.txt` pada halaman rilis.

## Menjalankan

Install `.exe`, lalu jalankan. Aplikasi muncul di tray:

- **Buka Dashboard** — tampilkan window utama.
- **Pengaturan** — window konfigurasi.
- **Uji Alarm** — aktif di rilis lanjutan (Fase 4).
- **Status** — status aplikasi.
- **Keluar** — satu-satunya cara menghentikan aplikasi.

Menutup dashboard hanya menyembunyikannya; proses tetap berjalan di latar.

## Perangkat demo (virtual device)

Berada di `workshop mikro lanjut dan interface\Smart Ergo Simulasi\` — halaman HTML
terpisah (tidak ikut dibundel) yang mensimulasikan dua node ESP32 di atas broker
MQTT sungguhan. Tidak ada hardware yang dibutuhkan untuk menguji aplikasi ini.