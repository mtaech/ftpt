# AGENTS.md — Photo Tool

## Stack

- **GPUI** native desktop app (GPU-accelerated, by Zed Industries)
- **gpui-component** UI library (60+ shadcn/ui-style components)
- **photo-tool-core** pure Rust domain logic crate

## Workspace layout

```
photo-tool-core/          # Pure Rust library — all domain logic
├── Cargo.toml            # Dependencies: rawlib, kamadak-exif, image, trash, ...
photo-tool-gpui/          # GPUI desktop application
├── Cargo.toml            # Package: photo-tool-gpui; depends on photo-tool-core, gpui, gpui-component
├── src/
│   ├── main.rs           # App entry, state, rendering
│   └── texture_manager.rs # Thumbnail disk cache + lazy GPU texture loading
```

## Commands

| What | Command |
|------|---------|
| Rust build | `cargo build` |
| Build GPUI app | `cargo build -p photo-tool-gpui` |
| Rust tests (core) | `cargo test -p photo-tool-core` |
| Run single Rust test | `cargo test -p photo-tool-core -- <test_name>` |
| Rust Clippy (all) | `cargo clippy --all-targets` |

## Rust details

- **Workspace crates**: `photo-tool-core` (library) and `photo-tool-gpui` (binary).
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
- Key dependencies in `photo-tool-gpui`:
  - `gpui` (git: zed-industries/zed) — GPU-accelerated UI framework
  - `gpui_platform` (git, features: font-kit) — platform windowing
  - `gpui-component` (git: longbridge/gpui-component) — UI component library
  - `gpui-component-assets` (git) — bundled default assets
  - `rfd 0.17` — native file dialogs
  - `image 0.25` — image decoding
  - `dirs 6` — platform directories

## GPUI patterns

- `gpui_component::init(cx)` must be called early in `app.run` closure
- `Root::new(view, window, cx)` wraps every window's content
- `gpui_platform::application().with_assets(...)` initializes the app
- `cx.spawn(async {...}).detach()` wraps `cx.open_window`
- Background work: `std::thread::spawn` + `mpsc::channel` → poll in `render()`
- `Render` trait: `fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement`
- Theme colors: use `Rgba` from `rgb()`, not `Hsla`
