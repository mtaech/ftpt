# 0001 — 分层 crate 结构（Layered Crates Over Zed-Style 1:1 Splits）

## 状态

已实施

## 背景

photo_tool 最初是 2-crate workspace（`photo-tool-core` + `photo-tool-app`）。随着功能增长，core 内部出现 10 个模块平铺，其中 `domain ↔ exif/xmp` 存在跨模块依赖。按 Zed 的惯例（239 个 crate，一 crate 一关注点）重构的提议被提出。

## 决策

拆为 4 个 crate，按依赖层级组织而不是按关注点一一对应：

```
crates/
├── photo-domain/      ← 类型叶子（Capture, ExifMetadata, XmpMetadata…）
├── photo-engine/      ← 文件机械（scanner, exif 提取, xmp 读写, thumbnail…）
├── photo-config/      ← 配置（TOML 读写 + SQLite 持久化）
└── photo-tool-app/    ← GPUI 前端（状态 + UI + worker）
```

依赖方向：`app → engine → domain`，`app → config`。

所有依赖版本 `[workspace.dependencies]` 集中管理。

## 被否决的选项

### 选项 A：全量 Zed 式（10 crate，1:1 映射）

每个模块拆为独立 crate。利弊：

- **优点**：最大的编译器强制力，growth 时加 crate 不加摩擦
- **缺点**：`config.rs`（150 行）、`xmp.rs`（291 行）单独一个 crate 的仪式成本大于收益；10 个 Cargo.toml 的维护负担

### 选项 B：只拆叶子（只拆 domain）

- **优点**：最小改动，保住了"人人依赖 domain、无人被 domain 依赖"的核心不变量
- **缺点**：services 层内（scanner/ops/thumbnail…）仍可互相耦合，crate 边界不提供额外保护

## 理由

1. **DAG 编译器强制**：在单 crate 内，模块间可以互相 `use`（Rust 允许同 crate 循环引用）。拆 crate 后 `engine → domain` 的单向性由 Cargo 保证，任何人无法反向写 `domain → engine` 的 import

2. **增长骨架（D 动机）**：三层加 config 的结构容忍未来的增长——受信任的第三方库可以只依赖 `photo-domain`。新功能在 engine 层加模块即可，不影响已有 crate

3. **类型下沉而非复制**：`exif` 和 `xmp` 都发现了"类型定义在机械层、但领域层消费者也需要"的跨层矛盾。统一的解法是把类型（`ExifMetadata`、`XmpMetadata`、`CameraInfo` 等）移到 `photo-domain`，机械（提取/读写）留在 `photo-engine`

## 影响

- 正：编译粒度更细——改 domain 只重编 2 个 crate，改 engine 重编 3 个
- 正：import 路径从 `photo_tool_core::domain::X` 变成 `photo_domain::X`——路径即契约
- 负：3 份 Cargo.toml 的版本同步成本
- 负：跨 crate 方法调用不能走 orphan rules（`impl ExifMetadata` 必须在定义 crate 内）
