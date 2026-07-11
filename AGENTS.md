# AGENTS.md — Photo Tool

## Stack

- **Tauri v2** desktop app (config at `src-tauri/tauri.conf.json`)
- **Frontend**: Vue 3 + Pinia + Vue Router (hash history) + Reka UI + Lucide icons
- **Backend**: Rust, Cargo workspace (`resolver = "2"`, edition `"2024"`) with two crates

## Workspace layout

```
photo-tool-core/          # Pure Rust library — all domain logic
├── Cargo.toml            # Dependencies: rawlib, kamadak-exif, image, trash, ...
src-tauri/                # Tauri v2 shell — commands/ delegate to photo-tool-core
├── Cargo.toml            # Package: photo-tool-tauri; depends on photo-tool-core, tauri v2 + plugins
├── tauri.conf.json       # Tauri app config
├── capabilities/
│   └── default.json      # Permission grants (core, dialog, fs, ...)
src/                      # Vue 3 frontend — @ = ./src
├── App.vue
├── main.ts
├── router/index.ts       # Hash-based routing (createWebHashHistory)
├── stores/               # Pinia stores (browse, config, ui)
├── views/                # BrowseView, CompareView
├── components/           # Layout, Toolbar, DirectoryTree, ThumbnailGrid, dialogs, ...
├── composables/          # useKeyboard, useThumbnail
└── types/                # index.ts (domain types), tauri.ts (IPC command types)
```

## Commands

| What | Command |
| ------ | --------- |
| Tauri dev (full app) | `pnpm tauri` |
| Frontend-only dev | `pnpm dev` (Vite on :1420, strict port) |
| Frontend build | `pnpm build` (vue-tsc --noEmit → vite build) |
| Rust build | `cargo build` |
| Rust tests (core) | `cargo test -p photo-tool-core` |
| Run single Rust test | `cargo test -p photo-tool-core -- <test_name>` |
| Rust Clippy (all) | `cargo clippy --all-targets` |
| Check Tauri Rust code | `cargo check -p photo-tool-tauri` |

## Frontend details

- **Package manager**: `pnpm` — never use npm or yarn.
- **Tauri dev** (`pnpm tauri`) auto-starts the Vite dev server via `tauri.conf.json` `beforeDevCommand`. Do not run `pnpm dev` and `pnpm tauri` simultaneously — they will fight over port 1420.
- **Routing**: Hash-based (`createWebHashHistory`), because Tauri loads from `http://localhost:1420`.
- **Alias**: `@` → `./src`.
- **Dev server**: port `1420`, strict port (fail if taken).
- **Build target**: `chrome105` (Windows), `safari14` (other).
- **Icons**: `lucide-vue-next` for UI icons.
- **No JS/TS test runner** is configured.
- **No linter/formatter** is configured for frontend code.

## Rust details

- **Workspace crates**: `photo-tool-core` (library) and `src-tauri` (binary, package name `photo-tool-tauri`).
- **Tests** live inline in `photo-tool-core/src/*.rs` (no `tests/` dir).
- **No explicit rustfmt.toml or clippy.toml** — use defaults.
- `libraw.so` is gitignored — the `rawlib` crate requires a system-level libraw installation for RAW image support.
- Key dependencies in `photo-tool-core`:
  - `rawlib 0.3` — RAW image decoding
  - `kamadak-exif 0.6` — EXIF reading
  - `image 0.25` (jpeg/png/tiff/webp/bmp/gif) — thumbnail & format conversion
  - `quick-xml 0.37` — XMP parsing
  - `trash 4` — move to trash instead of permanent delete
  - `walkdir 2` — recursive directory scanning
  - `chrono 0.4` — date/time handling
  - `toml 0.8` — config serialization
- Key dependencies in `src-tauri`:
  - `tauri 2` with `devtools` feature
  - `tauri-plugin-dialog 2` — native open/save/message dialogs
  - `tauri-plugin-fs 2` — filesystem access
  - `tauri-plugin-log 2` — logging
  - `tokio 1` (full features) — async runtime

## Tauri capabilities

Permissions are declared in `src-tauri/capabilities/default.json`:

- `core:default` — core Tauri functionality
- `dialog:allow-open`, `dialog:allow-ask`, `dialog:allow-message` — native dialog prompts
- `fs:allow-read`, `fs:allow-exists` — filesystem read access

Adding new Tauri plugins or commands may require updating this file.
