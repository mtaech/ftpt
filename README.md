# Photo Tool

一款基于 **GPUI**（[gpui-kit](https://gpui-kit.com) 0.6.1）的桌面照片管理和筛选（Culling）应用。

![截图](screenshot.png)

## 功能

- **目录扫描** — 打开文件夹自动扫描 JPEG + RAW（单层/递归可配）
- **网格浏览** — 缩略图虚拟网格，堆叠分组（同画面多格式/连拍），RAW 内嵌预览快速加载
- **标记筛选** — 评分（1-5★）、颜色标签、旗标（Pick/Reject），多维筛选 + 排序
- **预览模式** — 全尺寸查看，滚轮缩放，拖拽平移，1:1 全分辨率
- **物种识别** — org_det 通用检测 → BioCLIP 全物种零样本分类 → 名录映射（单张/批量）
- **文件操作** — 删除（回收站/永久）、移动、复制、批量重命名（可撤销）
- **格式转换** — RAW 内嵌预览→JPEG、缩放导出、调整参数烘焙
- **SD 卡导入** — 可移动设备检测，按拍摄日期归档去重
- **信息面板** — EXIF 元数据（相机/镜头/拍摄参数）、全局物种统计
- **双主题** — 亮色 / 交易终端风格近黑暗色

## 截图

*(待补充)*

## 构建

### 前置条件

- Rust **nightly** 频道（edition 2024 需要）
- Linux：需安装 `libraw`（`libraw-dev` 或同目录 `local-lib/`）；运行 GPUI 需要 X11/Wayland（无头环境用 `xvfb-run`）
- Windows/Linux：需 ExifTool 运行时（`local-lib/exiftool/`，见 `docs/exiftool-update.md`）

### 命令

```bash
# Rust 全量构建
cargo build

# 前端检查 / 单测（GPUI 版，crates/photo-ui）
cargo check -p photo-ui
cargo test -p photo-ui

# 核心测试
cargo test -p photo-engine -p photo-recognize -p photo-domain -p photo-config

# 开发运行（GPUI 桌面窗口）
cargo run -p photo-ui

# 无头冒烟（无需真实显示器）
XDG_CONFIG_HOME=/tmp/ptlease-config xvfb-run -a cargo run -p photo-ui --example lease_smoke
```

## 技术栈

| 层 | 技术 |
|---|---|
| 前端 | [GPUI](https://gpui.rs/)（经 [gpui-kit](https://gpui-kit.com) 0.6.1；`crates/photo-ui`，Dock 三栏 + Material You 主题） |
| 后端 | Rust（`photo-engine` / `photo-recognize` / `photo-config` / `photo-domain`） |
| RAW 解码 | [rawlib](https://crates.io/crates/rawlib)（封装 LibRaw） |
| EXIF | exiftool 主后端 + rawlib 回退后端 |
| 图片管线 | 进程内解码：缩略图 / 2560 母版 / 1:1 全分辨率（`ImageManager`，无 IPC） |
| 缩略图 | 磁盘缓存 + Lanczos3 缩放 |
| 配置 | TOML + 便携模式优先 |
| 许可 | MIT |

## 许可

本程序：[MIT](LICENSE)。

随发布包分发的模型与生物名录资产来自第三方，各自许可与署名要求见 [NOTICE](NOTICE)
（BioCLIP 2 = MIT；TreeOfLife-200M = CC0-1.0；Catalogue of Life China = CC BY）。
⚠️ `models/org_det.onnx` 基于 Ultralytics YOLOE，为 **AGPL-3.0**（copyleft，与收不收费无关）：
包含该权重的发布包整体按 AGPL-3.0 分发（文本见 [LICENSE-AGPL-3.0.txt](LICENSE-AGPL-3.0.txt)），
本仓库源码仍为 MIT；只是想搬进闭源产品的人需要先换模型或买商业许可。
