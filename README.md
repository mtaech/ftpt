# Photo Tool

一款基于 **GPUI** 的桌面照片管理和筛选（Culling）应用。

![截图](screenshot.png)

## 功能

- **目录扫描** — 打开文件夹自动扫描 JPEG + RAW + sidecar 配对
- **网格浏览** — 缩略图网格，支持 RAW 内嵌 JPEG 快速预览
- **标记筛选** — 评分（1-5★）、颜色标签、旗标（Pick/Reject）
- **预览模式** — 全尺寸查看，滚轮缩放，拖拽平移
- **文件操作** — 删除（回收站/永久）、重命名、多选批量操作
- **信息面板** — EXIF 元数据（相机/镜头/拍摄参数）
- **XMP 旁车** — PT 自定义命名空间，评分/标签/旗标存为 XMP
- **暗色主题** — 交易终端风格近黑 UI

## 截图

*(待补充)*

## 构建

### 前置条件

- Rust **nightly** 频道（edition 2024 需要）
- Linux：需安装 `libraw`（`libraw-dev` 或同目录 `local-lib/`）

### 命令

```bash
# 完整构建
cargo build

# 运行
cargo run -p photo-tool-app

# 测试
cargo test -p photo-tool-core
```

## 技术栈

| 层 | 技术 |
|---|---|
| 前端 | [GPUI](https://github.com/zed-industries/zed)（GPU 加速 UI 框架） |
| 组件库 | [gpui-component](https://github.com/longbridge/gpui-component) |
| RAW 解码 | [rawlib](https://crates.io/crates/rawlib)（封装 LibRaw） |
| EXIF | kamadak-exif（常规图）/ rawlib::exif（RAW） |
| 缩略图 | 磁盘缓存 + Lanczos3 缩放 |
| 配置 | TOML + 便携模式优先 |
| 许可 | MIT |

## 许可

[MIT](LICENSE)
