# ADR-001: 视觉设计系统 — 基于 Zed GPUI 的设计语言

## 状态

提议

## 上下文

photo-tool-app 使用 GPUI 框架（Zed 编辑器同一框架），但当前视觉设计严重落后于框架能力：

- 纯白底色（`#ffffff`），扁平而不分层
- 颜色是裸值常量，无语义层级
- 无阴影、无 Elevation
- 无暗色主题
- 间距硬编码 5 级（4/8/12/16/24）
- 字体硬编码为 Microsoft YaHei UI
- 状态色（danger/success/warning）直接用纯色（`#ef4444`），无背景/边框三层派生
- 元素状态（hover/active/selected）各组件各自维护，不是系统级数据

Zed 本身（GPUI 的创建者）有一套经过工业级验证的设计体系——主题 JSON schema v0.2.0、语义化颜色分层、三层 Elevation、动态间距密度。本 ADR 将其适配到 photo-tool 领域。

## 决策

采用 Zed 风格的设计体系，每个方面按 3 个阶段实施：

### 1. 色彩体系 (Phase 1)

将 `theme.rs` 从扁平常量重构为语义化 `ThemeColors` 结构体，提供 `light`/`dark` 两个变体。

```rust
#[derive(Clone, Debug)]
pub struct ThemeColors {
    // ── 边框 ──
    pub border: Hsla,
    pub border_variant: Hsla,      // 次要边框/分隔线
    pub border_focused: Hsla,      // focus ring
    pub border_selected: Hsla,     // 选中态
    pub border_transparent: Hsla,
    pub border_disabled: Hsla,

    // ── 表面层级 ──
    pub elevated_surface_background: Hsla,  // 模态框/弹窗
    pub surface_background: Hsla,           // 面板/容器
    pub background: Hsla,                   // 根背景（surface 之下）

    // ── 元素状态 ──
    pub element_background: Hsla,
    pub element_hover: Hsla,
    pub element_active: Hsla,
    pub element_selected: Hsla,
    pub element_disabled: Hsla,

    // ── 文字 ──
    pub text: Hsla,
    pub text_muted: Hsla,
    pub text_placeholder: Hsla,
    pub text_disabled: Hsla,
    pub text_accent: Hsla,

    // ── 图标 ──
    pub icon: Hsla,
    pub icon_muted: Hsla,
    pub icon_disabled: Hsla,
    pub icon_placeholder: Hsla,
    pub icon_accent: Hsla,
}
```

#### Light 调色盘（基于 One Light 适配）

```
border:              #c9c9ca   (暖灰)
border.variant:      #dfdfe0   (更淡)
border.focused:      #7d82e8   (蓝紫)
border.selected:     #cbcdf6   (淡蓝紫)

elevated_surface:    #ebebec   (弹窗表面)
surface:             #ebebec   (面板)
background:          #dcdcdd   (根背景，比 surface 略深)

element.bg:          #ebebec
element.hover:       #dfdfe0
element.active:      #cacaca
element.selected:    #cacaca

text:                #242529   (近黑暖)
text.muted:          #58585a   (中灰)
text.placeholder:    #7e8086   (浅灰)
text.disabled:       #7e8086
text.accent:         #5c78e2   (蓝紫)

icon:                #242529
icon.muted:          #58585a

status colors (三层: base / .bg / .border):
  error:     #d36151 / #fbdfd9 / #f6c6bd
  warning:   #a48819 / #faf2e6 / #f4e7d1
  success:   #669f59 / #dfeadb / #c8dcc1
  info:      #5c78e2 / #e2e2fa / #cbcdf6
```

#### Dark 调色盘（基于 One Dark 适配）

```
border:              #464b57
border.variant:      #363c46
border.focused:      #47679e
border.selected:     #293b5b

elevated_surface:    #2f343e
surface:             #2f343e
background:          #3b414d   (比 surface 亮，形成反直觉的视觉深度)

element.bg:          #2e343e
element.hover:       #363c46
element.active:      #454a56

text:                #dce0e5
text.muted:          #a9afbc
text.placeholder:    #878a98
text.accent:         #74ade8

status:
  error:     #d07277 / #d072771a / #4c2b2c
  warning:   #dec184 / #dec1841a / #5d4c2f
  success:   #a1c181 / #a1c1811a / #38482f
  info:      #74ade8 / #74ade81a / #293b5b
```

### 2. 封装 API

```rust
// 全局主题访问
impl ThemeColors {
    pub fn light() -> Self { ... }
    pub fn dark() -> Self { ... }
}

// 应用全局
pub fn set_theme(mode: ThemeMode, ...);

// 组件内使用
let colors = theme::colors();  // 返回当前主题的 &ThemeColors
div().bg(colors.surface_background)
    .text_color(colors.text)
```

### 3. Elevation 系统 (Phase 1)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElevationIndex {
    Background,       // #0: 根背景层
    Surface,          // #1: 面板/容器
    ElevatedSurface,  // #2: 弹窗/模态
    ModalSurface,     // #3: 最高层
}

impl ElevationIndex {
    /// 每层的阴影
    pub fn shadow(self) -> Vec<gpui::BoxShadow>;

    /// 每层的背景色
    pub fn bg(&self, colors: &ThemeColors) -> Hsla;
}
```

阴影定义：

| 层级 | 阴影 |
|------|------|
| Background | 无 |
| Surface | `0 1px 2px rgba(0,0,0,0.06)` |
| ElevatedSurface | `0 4px 12px rgba(0,0,0,0.10)` |
| ModalSurface | `0 8px 32px rgba(0,0,0,0.15)` |

### 4. 间距系统 (Phase 1)

从 5 个硬编码常量 → 8 级间距映射：

```rust
pub fn spacing(level: Spacing) -> Pixels;
pub enum Spacing {
    Xs = 0,  // 4px
    Sm = 1,  // 8px
    Md = 2,  // 12px
    Lg = 3,  // 16px
    Xl = 4,  // 24px
    Xxl = 5, // 32px
    Xxxl = 6,// 40px
    Xxxl2 = 7,// 48px
}
```

另：所有 `px(40.)` / `px(36.)` 等硬编码数值在 UI 组件中改用命名字段。

### 5. 字体体系 (Phase 2)

```rust
pub const UI_FONT_FAMILY: &str = "Inter, Microsoft YaHei UI, sans-serif";
pub const MONO_FONT_FAMILY: &str = "JetBrains Mono, Consolas, monospace";

pub enum FontSize {
    Xs = 11,
    Sm = 12,
    Md = 14,
    Lg = 16,
    Xl = 18,
    Heading = 22,
}
```

### 6. 暗色主题切换 (Phase 2)

```rust
pub enum ThemeMode {
    Light,
    Dark,
    System,  // 跟随系统
}

pub fn toggle_theme(cx: &mut Context<...>);
```

系统跟随通过检测 `WindowAppearance` 实现。

### 7. 组件样式重设计 (Phase 2–3)

#### 工具栏 (toolbar.rs)

| 变更 | 当前 | 改进 |
|------|------|------|
| 背景 | `panel_bg()` = `#f8f9fa` | `toolbar_background` 主题色 |
| 底部边框 | `BG_SURFACE` = `#e5e7eb` 灰 | `border_variant` 更淡 |
| 高度 | `h(px(40.))` | 用间距系统: `spacing(Spacing::Toolbar)` |
| 按钮样式 | 依赖 gpui_component Button | 统一用主题色 |

#### 侧边栏 (sidebar.rs)

| 变更 | 当前 | 改进 |
|------|------|------|
| 背景 | `panel_bg()` | `surface_background` |
| 分割线 | `BG_SURFACE` 灰 | `border_variant` |
| 选中行 | 自实现 | `element_selected` 主题色 |
| 间距 | 硬编码 px_2 | `spacing(Sm/Md)` |

#### 缩略图网格 (grid.rs + grid_cell.rs)

| 变更 | 当前 | 改进 |
|------|------|------|
| 网格背景 | `GRID_BG` = `#f3f4f6` | `background` 主题色 |
| 单元格背景 | `GRID_CELL_BG` = `#ffffff` | `surface_background` |
| 单元格边框 | `GRID_CELL_BORDER` = `#e5e7eb` | `border_variant` |
| 选中态 | 手动切换 `ACCENT` / `2px` | `border_selected` + `element_selected` bg |
| 阴影 | 无 | `ElevationIndex::Surface` 细微阴影 |
| 文件名字体 | `text_xs()` | `FontSize::Xs` 主题字号 |

#### 预览面板 (preview.rs)

| 变更 | 当前 | 改进 |
|------|------|------|
| 缩放按钮 | 36x36 硬编码 | `spacing(Lg) * 2.25` |
| 按钮背景 | `BG_SURFACE` | `element_background` + hover 变体 |
| 文字颜色 | `TEXT_PRIMARY` | `text` 主题色 |

#### 信息面板 (info_panel.rs)

| 变更 | 当前 | 改进 |
|------|------|------|
| 背景 | `panel_bg()` | `surface_background` |
| 卡片 section | 无阴影 | `ElevationIndex::Surface` 阴影 + `element_bg` 背景 |
| 分隔线 | 灰色边框 | `border_variant` |
| info_row 标签色 | `TEXT_SECONDARY` | `text_muted` |
| info_row 值色 | `TEXT_PRIMARY` | `text` |

#### 状态栏 (status_bar.rs)

| 变更 | 当前 | 改进 |
|------|------|------|
| 背景 | StatusBar 默认 | `status_bar_background` 主题色 |
| 文字色 | `TEXT_MUTED` / `TEXT_SECONDARY` | `text_muted` / `text` |
| "就绪" 状态 | `SUCCESS` = `#22c55e` | `success` 主题色 |

#### 导入向导 (import_wizard.rs)

| 变更 | 当前 | 改进 |
|------|------|------|
| 模态遮罩 | `rgba(0x00000088)` | 半透明模态背景 + `ElevationIndex::ModalSurface` 阴影 |
| 弹窗背景 | `panel_bg()` | `elevated_surface_background` |
| 弹窗边框 | `BG_SURFACE` | `border` |
| 圆角 | `rounded_md()` | `BORDER_RADIUS_LG` (10px) |

### 8. 圆角与聚焦环统一 (Phase 1)

```rust
pub const BORDER_RADIUS: f32 = 6.0;     // 通用元素
pub const BORDER_RADIUS_LG: f32 = 10.0; // 模态框/大卡片
pub const BORDER_RADIUS_SM: f32 = 4.0;  // 标签/徽标
pub const FOCUS_RING_WIDTH: f32 = 2.0;
```

## 实施计划

### Phase 1 (基础颜色 + Elevation + 间距)

文件：`photo-tool-app/src/ui/theme.rs`

1. 重构 colors 常量区为 `ThemeColors` 结构体 + `light()` / `dark()` 构造器
2. 添加 `ElevationIndex` 枚举 + 阴影表
3. 添加 `ElevationBgExt` trait（`fn elevation_bg(self, index: ElevationIndex)`）
4. 添加 `Spacing` 枚举 + `fn spacing()`
5. 添加 `BORDER_RADIUS_*` + `FOCUS_RING_*` 常量
6. 添加全局 `AppTheme` 数据（当前主题模式 + 当前 ThemeColors）
7. 创建 `AppTheme::set_theme(mode)` 切换函数
8. 编写测试：验证 light/dark 颜色非零，Shadow 尺寸正确

### Phase 2 (组件迁移 + 暗色主题切换)

1. 逐个组件迁移（layout → toolbar → sidebar → grid → grid_cell → status_bar → info_panel → preview → import_wizard）
2. 每个迁移：
   - 引入主题色替代裸值
   - 用 Elevation 替代无阴影
   - 用 spacing() 替代硬编码
3. 添加暗色主题切换 UI 入口（快捷键 Ctrl+Shift+T 或工具栏按钮）
4. 跟随系统暗色模式
5. 测试：所有组件在 light/dark 下可读

### Phase 3 (字体 + 细节打磨)

1. 字体层级 `FontSize` / `FontWeight`
2. 按钮/输入框聚焦环
3. 动画过渡（hover、选择切换）
4. 无障碍颜色对比度检查

## 拒绝的方案

- **Bootstrap 式扁平主题** — 当前方案，无深度/层级感
- **Material Design 3** — 与 GPUI 框架风格不匹配
- **自定义主题 JSON 格式** — 过于重量级，当前阶段不需要 Zed 式完整主题系统
- **CSS 变量式主题** — GPUI 不支持 CSS 变量替换，需要 Rust 结构体

## 开放性

- Phase 1 独立可验证，不影响现有 UI 逻辑
- Phase 2 与现有组件并存（改一个测一个）
- 未来可扩展：用户自定义颜色、第三方主题导入
