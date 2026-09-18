# 待确认问题 / 变更说明（2026-09 修复轮）

> ⚠️ **归档**：本文件记录的是 **Tauri v2 版前端**（`crates/photo-tauri`，2026-09-18 已删除）那一轮的
> 改动与验证，保留作历史记录——文中提到的 Vue/TS 文件、Tauri command 与插件都已不在仓库里。

> 由 AI 在独自处理你提出的 4 个问题时记录。你回来后逐条答复，答复结果已更新到下方【答案与落地】。

## 你已答复（答案与落地）

1. **不迁移**：保持配置优先，不改旧配置，只改「新默认」。—— 已按此处理（只改默认值，你的配置已是 None）。
2. **全尺寸**：复制图片用全尺寸原分辨率（不是 2048 缩放）。—— 已改：常规格式直接读原文件全解码；RAW/TIFF/HEIF 走缩略图缓存 full 母版（全分辨率 JPEG）再解码。
3. **胶片条缩略图要右键**：在预览中打开 / 复制图片 / 复制路径。—— 已加（filmstripMenuItems）。
4. **虚拟列表兜底**：胶片条用横向虚拟列表，大目录也能随意滚动。—— 已改：滚动容器为全内容宽度，缩略图按滚动位置绝对定位，只挂载可视区 ±6；5000+ 也流畅。
5. **复制给微信发图用**：全尺寸 RGBA 图片即可。—— 已按全尺寸处理（微信粘贴）。

## 已做的变更

### 1. 预览页复制图片（全尺寸）
- 后端命令 copy_image_to_clipboard（Tauri 版 `crates/photo-tauri/src-tauri/src/lib.rs`，该 crate 已删除；GPUI 版**尚未实现剪贴板复制**）：全尺寸 RGBA 写入系统剪贴板。
  - 常规可解码格式（JPG/PNG/WebP/BMP/GIF）：直接读原文件字节，image crate 全尺寸解码（不缩放）。
  - RAW/TIFF/HEIF：走缩略图缓存 get_or_generate_full（全分辨率母版）再解码。
- 新增 copy_text_to_clipboard（复制文件绝对路径用），ipc 封装 copyTextToClipboard。
- 引入 tauri-plugin-clipboard-manager（写图片 + 写文本），capability 放行 allow-write-image / allow-write-text（Tauri 版专有，已随 crate 删除）。
- 前端 PhotoPreview.vue 右键菜单「复制图片」。

### 2-3. 胶片条：虚拟列表 + 滚动条下方 + 右键菜单
- Filmstrip.vue 改为「横向虚拟列表」：滚动容器全内容宽度（scrollWidth 反映全部条目），缩略图按滚动位置绝对定位渲染（可视区 ±6 缓冲）。大目录也能随意滚到任意位置，不再渐进加载缺失。
- 横向滚动条独立放在缩略图下方（自定义 track + thumb，随滚动同步、可点击/拖拽），不嵌在图片条内。
- 缩略图右键菜单（filmstripMenuItems）：在预览中打开 / 复制图片 / 复制路径。

### 4. 默认图片不堆叠分组
- 堆叠模式默认从 ByTime 改为 None（每文件独立）。涉及 photo-config、CONTEXT.md（config.ts / mock.ts / bindings.ts 属 Tauri 版前端，已删除）。
- 你的 ~/.config/pt/config.toml 已是 stackMode = None。

## 验证
- （当时，Tauri 版）前端 vue-tsc -b --noEmit ✅、npm run build ✅、vitest 85 用例 ✅
- （当时）Rust cargo check -p photo-tauri ✅、photo-config 11 用例 ✅
  GPUI 版现行验证：`cargo check -p photo-ui` + `cargo test -p photo-ui`
- 说明：photo-recognize 的 ort::ep 未使用告警是既有告警，非本次引入。