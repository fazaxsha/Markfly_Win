# Catatan Reverse Engineering & Porting — Markfly.md → Windows (Rust)

Dokumen ini mencatat hasil analisis proyek asli `../markfly.md` (Electron + Vue 3,
target macOS), keputusan porting ke **Rust + Tauri** agar dapat dikompilasi dan
dijalankan di **Windows**, struktur folder hasil, asumsi, serta kendala
kompatibilitas yang ditemukan.

## 1. Ringkasan analisis proyek asli

**Markfly.md v0.4.1** — aplikasi *markdown viewer* minimalis untuk macOS.

| Layer | Teknologi asli |
|---|---|
| Desktop shell | Electron 43 (`electron/main.cjs`) |
| UI | Vue 3 + TypeScript (SPA, satu root component `App.vue`) |
| Build | Vite 8 + Tailwind CSS v4 |
| Markdown | `marked` v18 + renderer override |
| Math | KaTeX via `marked-katex-extension` |
| Diagram | Mermaid v12 (sect. `.mermaid-block`, mode diagram/code/split) |
| Syntax highlight | highlight.js (20 bahasa) |
| Sanitizer | DOMPurify |
| File watcher | chokidar |
| Update | GitHub Releases API (`electron/update-service.cjs`) |
| Packaging | electron-builder (DMG/PKG/ZIP, `CFBundleDocumentTypes`) |

### Proses & IPC (tes)

Proses utama (`main.cjs`): window 900x700, menu (File/Edit/View/Help, Open
Recent dinamis), dialog, clipboard, chokidar watcher, prefs di
`userData/prefs.json`, update service, dan *authorized-files set* sebagai
pagar keamanan. Preload (`preload.cjs`) mengekspos `window.electronAPI`
(18 method). Renderer (`App.vue`): tab ganda, toggle source/preview, zoom
per-tab, find-in-document (CSS Custom Highlights), outline panel, tema
dark/light, drag & drop, import URL.

### Kebijakan keamanan yang direplikasi (`electron/security.cjs`)

- Hanya file berekstensi `.md/.markdown/.mdown/.markdn` yang boleh dibaca,
  dan hanya setelah "di-authorize" (dialog/CLI/drag-drop).
- Batas ukuran dokumen 5 MB (file lokal maupun unduhan).
- `isSafeImageHref`: hanya gambar inline `data:image/*;base64`.
- Mermaid: maks 24 diagram / 100 KB per diagram.
- Import URL: HTTPS only, tanpa kredensial, hanya IP publik (blok
  private/loopback/link-local/CGNAT/dokumentasi), maks 5 redirect,
  timeout 15 detik; GitHub blob → `raw.githubusercontent.com`.
- `openExternal`: hanya HTTPS ke host publik.

## 2. Pendekatan porting

**Arsitektur Tauri + Rust + workspace Cargo:**

- `crates/markfly-core` — **pure Rust**, semua logika domain: pipeline
  markdown (parse → render HTML aman), outline, prefs, otorisasi file,
  file watcher, URL import + guard SSRF.
- `src-tauri` — shell Tauri (Rust): command handlers, menu Windows native,
  state aplikasi, update service, drag & drop, file-association via CLI args.
- `src/` — renderer Vue 3 yang **dipertahankan dari aslinya** (agar tampilan
  dan fitur rendering tetap identik), diadaptasi agar memanggil command Rust
  lewat `@tauri-apps/api` (`src/markfly.ts`) alih-alih `window.electronAPI`.

Mengapa bukan rewrite GUI penuh (egui/iced)? Karena inti produk ini adalah
*fidelity rendering* (github-markdown-css, Mermaid, KaTeX, highlight.js).
Render HTML tetap berjalan di layer web, sedangkan **seluruh logika inti
(parse markdown, kebijakan keamanan, akses sistem) kini 100% Rust** —
memenuhi "Rust sebagai bahasa implementasi utama" untuk bagian yang
menentukan kompatibilitas platform.

### Pemetaan Electron API → Tauri command (Rust)

| `window.electronAPI` | Command Rust (`src-tauri/src/commands.rs`) |
|---|---|
| `readFile` | `read_file` |
| `watchFile` | `watch_file` (event `markfly://file-changed`) |
| `getArgs` | `get_args` (CLI args / file association) |
| `rendererReady` | `renderer_ready` (flush antrian open-file) |
| `openFileDialog` | `open_file_dialog` |
| `openFilePath` | `open_file_path` (+ rebuild menu recent) |
| `openDroppedFile` | *(native)* drag&drop → `open-file` event |
| `fetchMarkdownUrl` | `fetch_markdown_url` |
| `openExternal` | `open_external` |
| `copyText` | `copy_text` |
| `getTheme` | `get_theme` |
| `getRecentFiles` | `get_recent_files` |
| `loadPref` / `savePref` | `load_pref` / `save_pref` |
| `getUpdateState` & update-* | `get_update_state`, `update_check`, `update_download`, `update_open_installer`, `update_open_release` |
| `onFileChanged` / `onOpenFile` / `onUpdateState` | event listener `markfly://file-changed` / `markfly://open-file` / `markfly://update-state` |
| *(baru)* pipeline markdown | `render_preview`, `get_outline` |

### Perubahan teknologi dalam porting

| Asli | Port Rust | Alasan |
|---|---|---|
| `marked` + renderer override | `pulldown-cmark` (GFM: tables, strikethrough, task list, math) | pipeline Rust end-to-end |
| DOMPurify | `ammonia` (allow-list: tag/attr/scheme) | sanitizer Rust; `class`/`id`/`data-*`/`aria-*` di-allow agar scaffolding Mermaid & hljs tetap jalan |
| `markdown-security.ts` | `markdown.rs` + unit test | kebijakan (image/mermaid/escape) jadi Rust |
| `marked-katex-extension` | KaTeX `auto-render` di frontend | Rust mempertahankan delimiter `$…$`/`$$…$$` sebagai teks (`Event::InlineMath`) |
| highlight.js server-side | highlight.js `highlightElement` client-side | Rust meng-escape, hljs mewarnai setelah DOM masuk |
| chokidar | `notify` + debounce 150 ms | lintas-platform, termasuk save-via-rename |
| Custom `https` lookup + IP check | `reqwest` (rustls) + **DNS resolver kustom yang menolak IP non-publik** | mencegah TOCTOU/dns-rebinding, perbaikan dari aslinya |
| electron-builder (DMG/PKG) | Tauri bundle **MSI + NSIS** + file association `.md` | target Windows |
| GitHub updater (arm64 .pkg only) | updater umum (`.msi` diprioritaskan, fallback `.exe`), verifikasi sha256 | aslinya hard-fail di non-arm64 |
| macOS menu roles | menu Windows native: File (Open/Open Recent/Exit, Ctrl+O/Q), Edit, View (Reload/DevTools), Help (Check for Updates) | konvensi Windows |

## 3. Struktur folder hasil

```
markfly-rs/
├── Cargo.toml                  # Cargo workspace (2 member) + profile release
├── package.json                # frontend deps + script tauri
├── vite.config.ts              # Vite + Vue + Tailwind (base "./", strictPort)
├── tsconfig*.json              # konfigurasi TypeScript (dari aslinya)
├── index.html                  # entry HTML + CSP (ditambah ipc: http://ipc.localhost)
├── README.md                   # cara build & run di Windows
├── PORTING-NOTES.md            # dokumen ini
├── crates/
│   └── markfly-core/           # krate murni Rust (bisa dipakai ulang)
│       ├── Cargo.toml
│       ├── examples/dump.rs    # CLI kecil: markdown -> HTML tersanitasi
│       └── src/
│           ├── lib.rs          # re-export + 16 unit test
│           ├── config.rs       # konstanta kebijakan (5 MB, limit mermaid, dst.)
│           ├── documents.rs    # outline heading + penamaan dokumen (path Windows)
│           ├── error.rs        # MarkflyError (pesan identik dengan aslinya)
│           ├── fsops.rs        # otorisasi path, guard ukuran, baca file
│           ├── markdown.rs     # pipeline render + sanitizer (ammonia)
│           ├── netfetch.rs     # URL import + guard SSRF (resolver DNS tervalidasi)
│           ├── prefs.rs        # model/sanitasi preferensi (serde camelCase)
│           ├── recent.rs       # daftar file terbaru (maks 10)
│           └── watcher.rs      # notify + debounce
└── src-tauri/                  # shell desktop Windows (Rust)
    ├── Cargo.toml              # tauri + plugin dialog/opener/clipboard
    ├── build.rs                # tauri-build
    ├── tauri.conf.json         # window, CSP, bundle MSI/NSIS, file association
    ├── capabilities/default.json
    ├── gen_icons.py            # generator icon PNG/ICO (pure Python, tanpa PIL)
    ├── icons/                  # 32x32, 128x128, 128x128@2x, icon.ico
    └── src/
        ├── main.rs / lib.rs    # entry + builder (plugin, menu, drag&drop, IPC)
        ├── state.rs            # AppState (authorized, recent, watcher, pending, update)
        ├── commands.rs         # 20 #[tauri::command] = pengganti electronAPI
        ├── menu.rs             # menu Windows + Open Recent dinamis
        └── update.rs           # update checker GitHub + download + verifikasi
```

Frontend (`src/`): `App.vue`, `TabBar.vue`, `Sidebar.vue`,
`components/OutlinePanel.vue`, `components/UpdateStatus.vue`, `style.css`,
`main.ts`, `markfly.ts` (adapter Tauri baru), `katex-auto-render.d.ts`,
`assets/`.

## 4. Asumsi

1. **Repo update tetap `rizkidarmawan21/markfly.md`** (dari `update-service.cjs`).
   Karena release aslinya hanya memuat aset macOS (`*-arm64.pkg`), updater
   Windows akan melaporkan "No unique compatible installer" sampai ada rilis
   dengan aset Windows. Aturan seleksi aset: utamakan `.msi` yang menyebut
   versi, lalu `.exe`; URL harus berada di host GitHub dan path rilis repo ini.
2. **Preferensi lama tidak diimpor** dari `userData/prefs.json` versi Electron;
   aplikasi Windows memakai direktori data Tauri sendiri
   (`%APPDATA%\com.markfly.app`).
3. **File association** didaftarkan oleh installer (MSI/NSIS) untuk
   `.md/.markdown/.mdown/.markdn`; karena aslinya tidak memakai
   single-instance lock, perilakunya dipertahankan: peluncuran ganda membuka
   window baru. Jika ingin satu instance, aktifkan `tauri-plugin-single-instance`.
4. **CSP** diperluas dari aslinya (`ipc: http://ipc.localhost` +
   `ws://localhost:5173` untuk dev) karena Tauri memakai protokol IPC internal.
5. **Hal-hal yang tidak lagi berlaku di Windows:** event Finder `open-file`
   (mewakili double-click lewat file association + drag & drop via `tauri::DragDropEvent`),
   role menu macOS (`appMenu`), accelerator `Cmd` → `Ctrl`, dan dialog
   `webUtils.getPathForFile` (drag & drop kini native Rust, bukan renderer).
6. **Display path `~`-shortcut** kini juga mengenali `C:\Users\<nama>`.

## 5. Potensi kendala kompatibilitas (dan mitigasi)

1. **Perbedaan parser markdown.** `marked` vs `pulldown-cmark` bisa berbeda
   pada edge case (footnote, link title, autolink, garis pemisah tabel).
   GFM inti (tabel, strikethrough, task list, heading id, code fence) sudah
   diuji lewat unit test; kalau ada dokumen yang tampil berbeda, sesuaikan
   `markdown.rs` (paling sering: `ENABLE_*` options atau sanitizer allow-list).
2. **Sanitizer berbeda.** ammonia (allow-list ketat) menggantikan DOMPurify.
   Konsekuensi: skema URL terbatas pada `http/https/mailto/tel/data`
   (`data` hanya diperlukan gambar inline base64); link `data:text/html` tidak
   bisa dibuka karena klik link divalidasi ulang sebagai HTTPS oleh
   `open_external`. Atribut yang diizinkan: `class`, `id`, `data-*`, `aria-*`
   plus atribut per-tag; jika suatu fitur butuh atribut tambah, edit
   `sanitize()` di `markdown.rs`.
3. **KaTeX vs marked-katex.** Aslinya merender math saat parsing; port ini
   memakai KaTeX auto-render di DOM. Perilaku sedikit berbeda: teks seperti
   `$5 and $10` bisa ter-render sebagai math (sama seperti extension aslinya
   yang juga greedy pada pasangan `$`). Gunakan `\$` untuk escape.
4. **Mermaid & highlight.js tetap JS** (dipanggil setelah HTML Rust masuk ke
   DOM). Output SVG mermaid disisipkan setelah sanitasi, jadi tidak melewati
   ammonia — ini setara dengan alur aslinya (mermaid juga jalan setelah
   DOMPurify).
5. **Path Windows.** Semua operasi path memakai `std::path` (menerima `\` dan
   `/`); nama dokumen & tampilan sidebar sudah memisahkan kedua separator.
   Hindari path dengan spasi saat build dari命令行 — beberapa toolchain
   MinGW/GNU bermasalah dengan path ber-spasi (lihat catatan verifikasi).
6. **File watcher.** Editor yang menulis via rename (mis. VS Code) memicu
   beberapa event notify; debounce 150 ms meredam duplikat. Pada jaringan/OneDrive
   folder, event bisa lebih sering — jika perlu, naikkan `DEBOUNCE`.
7. **WebView2.** Aplikasi butuh runtime WebView2 (bawaannya ada di Windows 10/11);
   installer NSIS di-set `downloadBootstrapper` agar runtime terpasang bila belum.
8. **Toolchain Rust.** Target default `x86_64-pc-windows-msvc` butuh MSVC
   build tools. Alternatif tanpa Visual Studio:
   `rustup toolchain install stable-x86_64-pc-windows-gnu` + MinGW-w64, lalu
   `cargo +stable-x86_64-pc-windows-gnu build --target x86_64-pc-windows-gnu`.
9. **Tidak ada single-instance** (lihat asumsi 3) dan **tidak ada auto-replace
   update** — seperti aslinya, instalasi diselesaikan manual oleh pengguna.

## 6. Status verifikasi di Windows

| Langkah | Hasil |
|---|---|
| `cargo test -p markfly-core` (x86_64-pc-windows-gnu) | **16/16 lolos** (pipeline markdown, kebijakan gambar/mermaid, outline, IP publik, prefs, recent, path Windows) |
| `cargo check` + `cargo build -p markfly` (src-tauri, debug) | **lolos** — binary `markfly.exe` ter-link |
| `npm install` + `npm run build` (vue-tsc + vite) | **lolos** — type-check strict + bundle frontend |
| `npm run tauri build` (release + bundel) | **lolos** — `Markfly_0.4.1_x64_en-US.msi` (5,26 MiB) + `Markfly_0.4.1_x64-setup.exe` (4,49 MiB) di `src-tauri/target/release/bundle/` |

Catatan lingkungan verifikasi: mesin ini tidak memiliki MSVC build tools,
sehingga verifikasi memakai toolchain GNU (`stable-x86_64-pc-windows-gnu` +
MinGW-w64) — pada mesin dengan Visual Studio, target default
`x86_64-pc-windows-msvc` berjalan tanpa perubahan konfigurasi.

