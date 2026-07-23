# 采用 GPUI + gpui-component 构建桌面前端

Photo Tool 原规划 Flutter 前端（`photo_tool_flutter/`），但 Flutter 代码从未落地，仓库中仅有纯 Rust 核心库 `photo-tool-core`。我们决定用 GPUI（Zed Industries 的 GPU 加速 Rust UI 框架）+ gpui-component（Longbridge 的 60+ 组件库）构建桌面前端，实现 Rust 全栈。

## 考虑过的替代方案

| 方案 | 否决原因 |
|---|---|
| **Flutter** | 需要 Dart 生态，与 Rust core 通过 FFI 桥接，类型系统断裂，调试困难 |
| **Tauri** | Web 前端（JS/TS），照片网格滚动性能受 DOM 限制，Rust 后端优势未充分发挥 |
| **egui** | 即时模式框架，适合工具/调试面板，难以实现精美三栏布局和复杂交互 |
| **iced** | Elm-like 架构，类型安全但生态组件少，照片工具所需的虚拟化列表、图片渲染支持不够成熟 |

## 关键权衡

**选 GPUI 的理由：**
- Rust 全栈：前后端共享类型系统，直接调用 core 模块，无 FFI 开销
- GPU 加速渲染：缩略图网格虚拟化滚动、大图预览缩放都由 GPU 处理
- gpui-component 提供开箱即用的桌面组件（Button、Input、Select、Slider、List 等）
- Zed 编辑器证明了 GPUI 在复杂桌面应用中的可行性

**主要风险：**
- GPUI 目前 pre-1.0，版本间有 breaking changes
- 2026 年 Zed 团队传闻暂停 GPUI 投入以聚焦核心业务
- Windows 平台支持可能不如 macOS/Linux 成熟（但这是我们的主要开发平台）
- 社区较小，遇到问题需要读源码

## 后果

- 新建 `photo-tool-app` crate（binary），与 `photo-tool-core`（lib）组成 Cargo workspace
- 前端直接调用 core 各模块，废弃原计划的 `api.rs` FFI 层
- 采用 rayon 线程池 + GPUI AsyncWindowContext 桥接同步 core 调用
- 三栏布局（目录/筛选 | 网格/预览 | 元数据），暗色主题专用
- 线程模型：核心操作 spawn 到 rayon，结果通过 oneshot channel + cx.update_model() 回主线程
- 目标平台：Windows + Linux（macOS 不做）
- 依赖锁定：gpui 和 gpui-component 均以 git HEAD 引入
