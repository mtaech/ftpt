# Task for worker

You are a delegated subagent running from a fork of the parent session. Treat the inherited conversation as reference-only context, not a live thread to continue. Do not continue or answer prior messages as if they are waiting for a reply. Your sole job is to execute the task below and return a focused result for that task using your tools.

Task:
## 任务 1：搭建 workspace 和空窗口

创建 `photo-tool-gpui` crate，加入 workspace，写一个能编译的 GPUI 空窗口。

### 步骤

1. **创建 Cargo.toml**：`photo-tool-gpui/Cargo.toml`

```toml
[package]
name = "photo-tool-gpui"
version.workspace = true
edition.workspace = true

[dependencies]
gpui = { git = "https://github.com/zed-industries/zed" }
gpui_platform = { git = "https://github.com/zed-industries/zed", features = ["font-kit"] }
gpui-component = { git = "https://github.com/longbridge/gpui-component" }
gpui-component-assets = { git = "https://github.com/longbridge/gpui-component" }
rfd = "0.17"
photo-tool-core = { path = "../photo-tool-core" }
anyhow = "1.0"
```

2. **创建 main.rs**：`photo-tool-gpui/src/main.rs`

```rust
use gpui::*;
use gpui_component::Root;

struct AppState;

impl Render for AppState {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        Root::new(
            div().size_full().child("Photo Tool"),
            _window,
            _cx,
        )
    }
}

fn main() {
    let app = gpui_platform::application()
        .with_assets(gpui_component_assets::Assets);

    app.run(move |cx| {
        gpui_component::init(cx);

        cx.spawn(async move |cx| {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(
                        Bounds::centered(None, size(px(1400.), px(900.)), &cx),
                    )),
                    ..Default::default()
                },
                |window, cx| {
                    let view = cx.new(|_cx| AppState);
                    cx.new(|cx| Root::new(view, window, cx))
                },
            )
            .expect("Failed to open window");
        })
        .detach();
    });
}
```

3. **修改 workspace Cargo.toml**：根目录 `Cargo.toml` 的 members 数组添加 `"photo-tool-gpui"`：
当前：
```toml
members = ["photo-tool-core", "src-tauri"]
```
改为：
```toml
members = ["photo-tool-core", "src-tauri", "photo-tool-gpui"]
```

4. **编译验证**：运行 `cargo build -p photo-tool-gpui`

### 关键要点
- `gpui_component::init(cx)` 在 `app.run` 闭包中尽早调用
- `Root::new(view, window, cx)` 接收三个参数
- `gpui_platform::application().with_assets(...)` 而非 `Application::new()`
- `cx.spawn(async {...}).detach()` 包裹 `open_window`

## Acceptance Contract
Acceptance level: verified
Completion is not accepted from prose alone. End with a structured acceptance report.

Criteria:
- criterion-1: Implement the requested change without widening scope

Required evidence: changed-files, tests-added, commands-run, validation-output, residual-risks, no-staged-files

Finish with a fenced JSON block tagged `acceptance-report` in this shape:
Use empty arrays when no items apply; array fields contain strings unless object entries are shown.
```acceptance-report
{
  "criteriaSatisfied": [
    {
      "id": "criterion-1",
      "status": "satisfied",
      "evidence": "specific proof"
    }
  ],
  "changedFiles": [
    "src/file.ts"
  ],
  "testsAddedOrUpdated": [
    "test/file.test.ts"
  ],
  "commandsRun": [
    {
      "command": "command",
      "result": "passed",
      "summary": "short result"
    }
  ],
  "validationOutput": [
    "validation output or concise summary"
  ],
  "residualRisks": [
    "none"
  ],
  "noStagedFiles": true,
  "diffSummary": "short description of the diff",
  "reviewFindings": [
    "blocker: file.ts:12 - issue found, or no blockers"
  ],
  "manualNotes": "anything else the parent should know"
}
```