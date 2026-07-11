# Task for worker

You are a delegated subagent running from a fork of the parent session. Treat the inherited conversation as reference-only context, not a live thread to continue. Do not continue or answer prior messages as if they are waiting for a reply. Your sole job is to execute the task below and return a focused result for that task using your tools.

Task:
## 任务：补全剩余功能

当前 `photo-tool-gpui` crate 编译通过、可运行。需要补全剩余功能：

### 1. 搜索输入框

当前 `button::Button::new("search-btn")` 是占位符。改为 gpui-component 的 `TextInput` 或 `Input` 组件。
检查 `use gpui_component::input::*;` 或 `use gpui_component::prelude::*;` 看有什么可以用的输入组件。

要求：输入文字后实时过滤 `self.search_text` → `self.apply_filters()` → `cx.notify()`

### 2. 右键菜单 — "Copy Path" 和 "Open in Explorer"

在 `context_menu` 函数中。
- "Copy Path": 用 `cx.write_to_clipboard(ClipboardItem::new_string(cap.primary_path.clone()))` (GPUI API)
- "Open in Explorer": 用 `std::process::Command::new("explorer").arg("/select,").arg(&path).spawn();` (Windows) 或 `open` / `xdg-open` 跨平台

文件路径来自 `this.menu_target` → `this.captures[idx].primary_path.clone()`

### 3. 对话框：SettingsDialog

在 `src/main.rs` 末尾新加一个渲染函数 `settings_dialog(&mut self, surface, border, cx)`。
当 `self.show_settings = true` 时覆盖在主视图上。

对话框内容：
- 缩略图尺寸滑块（120-400px，步长20）
- Light/Dark 主题切换
- 关闭按钮

在工具栏加一个齿轮按钮打开设置对话框。

### 4. 对话框：ImportDialog / RenameDialog

ImportDialog：点击时调用 `rfd::FileDialog::new().pick_folder()`，然后打印路径（将来对接 `photo_tool_core::import`）。

RenameDialog：输入新文件名前缀 + 起始编号，预览结果列表。

### 5. 目录树展开/折叠

当前 sidebar 的目录列表只有一级。改为可展开：点击条目左侧的 ▶/▼ 符号切换展开状态，展开后显示子目录列表（用 `std::fs::read_dir` 读取）。

---

### 构建和测试

所有改动在 `photo-tool-gpui/src/main.rs`（如果是新函数也在同一个文件中，或在 `src/` 下加新文件）。
运行 `cargo build -p photo-tool-gpui` 验证编译通过。
运行 `cargo test -p photo-tool-core` 验证 64 个测试全部通过。

### 重要兼容性

- GPUI 的 `Render` trait: `fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement`
- gpui-component 的 `input` 模块可能在 `gpui_component::input` 或 `gpui_component::forms`
- 对话框用 `div().absolute().inset_0().bg(...).child(...)` 实现模态覆盖
- 所有颜色用 `rgb(0x...)` 而非 `Hsla`
- `cx.listener(|this, event: &EventType, window, cx| { ... })` 处理事件

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