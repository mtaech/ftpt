# Task for worker

You are a delegated subagent running from a fork of the parent session. Treat the inherited conversation as reference-only context, not a live thread to continue. Do not continue or answer prior messages as if they are waiting for a reply. Your sole job is to execute the task below and return a focused result for that task using your tools.

Task:
## 任务：全局状态管理 + 三栏布局 + 后台扫描通信

当前 `photo-tool-gpui/src/main.rs` 只有一个空窗口（`AppState` 结构体，`Render` 显示 "Photo Tool"）。现在实现核心功能。

### 前置知识
- GPUI Render trait: `fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement`
- gpui-component 提供 `Resizable` 组件做分栏布局
- `photo-tool-core` 已提供 `scanner::scan_directory()`、`CaptureMeta`、`TreeNode` 等类型
- 后台扫描用 `std::thread::spawn` + `std::sync::mpsc::channel` 通信，主线程通过 GPUI 的 `cx.spawn` 轮询通道

### 步骤

**1. 扩展 AppState（`photo-tool-gpui/src/main.rs`）**

在 `main.rs` 中扩展 `AppState`，添加字段：

```rust
use std::path::PathBuf;
use std::sync::mpsc;
use photo_tool_core::domain::{CaptureMeta, TreeNode, SortBy, SortDirection};

struct AppState {
    captures: Vec<CaptureMeta>,
    filtered_indices: Vec<usize>,
    directory_tree: Vec<TreeNode>,
    current_path: Option<PathBuf>,
    selected_indices: Vec<usize>,
    sort_by: SortBy,
    sort_direction: SortDirection,
    search_text: String,
    is_scanning: bool,
    scan_progress: Option<ScanProgress>,
    scan_rx: Option<mpsc::Receiver<ScanEvent>>,
    thumbnail_size: u32,
}

#[derive(Clone)]
struct ScanProgress {
    percent: u32,
    path: String,
    phase: String,
}

enum ScanEvent {
    Progress { percent: u32, path: String, phase: String },
    Complete { captures: Vec<CaptureMeta>, tree: Vec<TreeNode> },
    Error(String),
}
```

**2. 实现扫描触发和通道轮询**

在 `AppState` 上添加方法：

```rust
impl AppState {
    fn new() -> Self {
        Self {
            captures: vec![],
            filtered_indices: vec![],
            directory_tree: vec![],
            current_path: None,
            selected_indices: vec![],
            sort_by: SortBy::FileName,
            sort_direction: SortDirection::Ascending,
            search_text: String::new(),
            is_scanning: false,
            scan_progress: None,
            scan_rx: None,
            thumbnail_size: 220,
        }
    }

    fn start_scan(&mut self, path: PathBuf) {
        let (tx, rx) = mpsc::channel();
        self.scan_rx = Some(rx);
        self.is_scanning = true;
        self.scan_progress = Some(ScanProgress {
            percent: 0,
            path: path.to_string_lossy().to_string(),
            phase: "scanning".into(),
        });

        let report_tx = tx.clone();
        let report_path = path.to_string_lossy().to_string();
        let on_progress: Box<dyn Fn(u32) + Send> = Box::new(move |pct| {
            let _ = report_tx.send(ScanEvent::Progress {
                percent: pct,
                path: report_path.clone(),
                phase: "scanning".into(),
            });
        });

        std::thread::spawn(move || {
            let sidecar = vec!["xmp".to_string()];
            match photo_tool_core::scanner::scan_directory(
                &path, &sidecar, &Default::default(), Some(on_progress),
            ) {
                Ok(captures) => {
                    let metas: Vec<CaptureMeta> = captures.iter()
                        .map(|c| c.into()).collect();
                    let _ = tx.send(ScanEvent::Complete {
                        captures: metas, tree: vec![],
                    });
                }
                Err(e) => {
                    let _ = tx.send(ScanEvent::Error(e.to_string()));
                }
            }
        });
    }

    fn poll_scan(&mut self) {
        if let Some(ref rx) = self.scan_rx {
            while let Ok(event) = rx.try_recv() {
                match event {
                    ScanEvent::Progress { percent, path, phase } => {
                        self.scan_progress = Some(ScanProgress { percent, path, phase });
                    }
                    ScanEvent::Complete { captures, tree } => {
                        self.captures = captures;
                        self.directory_tree = tree;
                        self.apply_filters();
                        self.is_scanning = false;
                        self.scan_progress = None;
                        self.scan_rx = None;
                    }
                    ScanEvent::Error(_) => {
                        self.is_scanning = false;
                        self.scan_progress = None;
                        self.scan_rx = None;
                    }
                }
            }
        }
    }

    fn apply_filters(&mut self) {
        let mut indices: Vec<usize> = (0..self.captures.len()).collect();
        if !self.search_text.is_empty() {
            let q = self.search_text.to_lowercase();
            indices.retain(|&i| self.captures[i].base_name.to_lowercase().contains(&q));
        }
        indices.sort_by(|&a, &b| {
            let ca = &self.captures[a];
            let cb = &self.captures[b];
            let cmp = match self.sort_by {
                SortBy::FileName => ca.base_name.cmp(&cb.base_name),
                SortBy::FileSize => ca.file_size.unwrap_or(0).cmp(&cb.file_size.unwrap_or(0)),
                SortBy::DateTaken => ca.date_taken.cmp(&cb.date_taken),
            };
            match self.sort_direction {
                SortDirection::Ascending => cmp,
                SortDirection::Descending => cmp.reverse(),
            }
        });
        self.filtered_indices = indices;
    }
}
```

**3. 在 Render 中实现三栏布局（gpui-component Resizable）**

```rust
impl Render for AppState {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 轮询扫描通道
        self.poll_scan();

        Root::new(
            div().size_full().flex_row()
                // Left: Directory sidebar
                .child(
                    div()
                        .w(px(260.))
                        .h_full()
                        .border_r_1()
                        .border_color(gpui::rgb(0xe0e0e0))
                        .child("目录树")
                )
                // Center: Toolbar + Grid + StatusBar
                .child(
                    div().flex_1().h_full().flex_col()
                        .child(
                            div().h(px(40.)).border_b_1().border_color(gpui::rgb(0xe0e0e0))
                                .flex_row().items_center().px_2()
                                .child(format!("{} files", self.captures.len()))
                        )
                        .child(
                            div().flex_1().child("缩略图网格")
                        )
                        .child(
                            div().h(px(24.)).border_t_1().border_color(gpui::rgb(0xe0e0e0))
                                .child(format!("{} 张 |", self.filtered_indices.len()))
                        )
                )
                // Right: Preview panel
                .child(
                    div()
                        .w(px(320.))
                        .h_full()
                        .border_l_1()
                        .border_color(gpui::rgb(0xe0e0e0))
                        .child("预览")
                ),
            _window,
            cx,
        )
    }
}
```

**4. 在 Render 开头添加 `self.poll_scan()` 调用**（已在第 3 步中包含）

**5. 为测试：在 main 函数中给 AppState 添加临时按钮来触发扫描（用于验证）**

在 AppState render 中加一个按钮调用 `start_scan` 来测试：

```rust
.child(
    Button::new("open-test")
        .label("打开目录")
        .on_click(cx.listener(|this, _, _, cx| {
            // 使用 rfd 打开文件夹选择对话框
            // 暂时先硬编码测试路径
            let path = std::path::PathBuf::from("C:/Users/huang/Pictures");
            this.start_scan(path);
            cx.notify();
        }))
)
```

需要添加 `use gpui_component::button::Button;`。

还需要添加 `use gpui_component::*;` 因为用了 Root。

### 编译验证
运行 `cargo build -p photo-tool-gpui`

### 注意
- `CaptureMeta` 必须实现 `From<&Capture>` ——检查 `photo-tool-core/src/domain.rs` 中是否已有此 impl。如果 `captures.iter().map(|c| c.into()).collect()` 报错，改用 `captures.iter().map(CaptureMeta::from).collect()`
- `gpui_component::init(cx)` 已在 main 中调用
- 注意 `CaptureMeta` 的字段名是 camelCase（因为有 `#[serde(rename_all = "camelCase")]`）——如 `base_name` 在 Rust 端就是 `base_name`（snake_case），Serialize 时才转

## Acceptance Contract
Acceptance level: checked
Completion is not accepted from prose alone. End with a structured acceptance report.

Criteria:
- criterion-1: Implement the requested change without widening scope

Required evidence: changed-files, tests-added, commands-run, residual-risks, no-staged-files

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