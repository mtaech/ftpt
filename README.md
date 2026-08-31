# Photo Tool

一款基于 **Tauri v2** 的桌面照片管理和筛选（Culling）应用。

![截图](screenshot.png)

## 功能

- **目录扫描** — 打开文件夹自动扫描 JPEG + RAW（单层/递归可配）
- **网格浏览** — 缩略图虚拟网格，堆叠分组（同画面多格式/连拍），RAW 内嵌预览快速加载
- **标记筛选** — 评分（1-5★）、颜色标签、旗标（Pick/Reject），多维筛选 + 排序
- **预览模式** — 全尺寸查看，滚轮缩放，拖拽平移，1:1 全分辨率
- **鸟类识别** — YOLO 检测 → 鸟种分类 → 名录映射 → 鸟眼锐度评分（单张/批量）
- **文件操作** — 删除（回收站/永久）、移动、复制、批量重命名（可撤销）
- **格式转换** — RAW 内嵌预览→JPEG、缩放导出、调整参数烘焙
- **SD 卡导入** — 可移动设备检测，按拍摄日期归档去重
- **信息面板** — EXIF 元数据（相机/镜头/拍摄参数）、全局鸟种统计
- **双主题** — 亮色 / 交易终端风格近黑暗色

## 截图

*(待补充)*

## 构建

### 前置条件

- Rust **nightly** 频道（edition 2024 需要）
- Node.js + pnpm（前端构建）
- Linux：需安装 `libraw`（`libraw-dev` 或同目录 `local-lib/`）
- Windows/Linux：需 ExifTool 运行时（`local-lib/exiftool/`，见 `docs/exiftool-update.md`）

### 命令

```bash
# Rust 全量构建
cargo build

# Tauri 后端检查
cargo check -p photo-tauri

# 核心测试
cargo test -p photo-engine -p photo-recognize -p photo-domain -p photo-config

# 开发运行（tauri dev，vite 1420 端口 + WebView2）
cd crates/photo-tauri && npm run tauri dev

# 浏览器 mock 模式（无 Tauri 后端）
cd crates/photo-tauri && npm run dev  # 浏览器开 localhost:1420

# 前端类型检查（必须带 -b）
cd crates/photo-tauri && npx vue-tsc -b --noEmit

# 前端单测
cd crates/photo-tauri && npx vitest run

# 前端生产构建
cd crates/photo-tauri && npm run build
```

## 技术栈

| 层 | 技术 |
|---|---|
| 前端 | [Tauri v2](https://tauri.app/) + Vue 3 + Pinia + Tailwind v4 + shadcn-vue |
| 后端 | Rust（`photo-engine` / `photo-recognize` / `photo-config` / `photo-domain`） |
| 类型共享 | specta + tauri-specta（Rust serde 类型 → TS 绑定） |
| RAW 解码 | [rawlib](https://crates.io/crates/rawlib)（封装 LibRaw） |
| EXIF | exiftool 主后端 + rawlib 回退后端 |
| 图片协议 | `ptimg://` 自定义协议流式 serve（缩略图/预览/全尺寸） |
| 缩略图 | 磁盘缓存 + Lanczos3 缩放 |
| 配置 | TOML + 便携模式优先 |
| 许可 | MIT |

## 许可

[MIT](LICENSE)
