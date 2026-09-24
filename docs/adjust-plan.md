# 预览「调整」功能完善方案

> 日期：2026-09-24 · 状态：待拍板 · 范围：右栏「调整」tab + 预览烘焙链路（ADR 0007 的续篇）
>
> 说明：本轮对话里**截图没有送达**（会话记录中该条用户消息只有纯文本，无 image 附件），
> 所以本方案按代码现状盘点，不针对某张具体截图。若截图指的是别的东西（例如外部软件的调整面板），
> 按第 7 节的「对照口径」再校准一次即可。

---

## 1. 现状盘点（已实现，勿重复造）

| 能力 | 落点 | 状态 |
|---|---|---|
| 三参数模型（曝光 ±2.0 EV/0.05、对比度 ±100、饱和度 ±100） | `crates/photo-domain/src/domain.rs:665` | 可用 |
| 量化 / 格式化 / 相对路径纯逻辑（含单测） | `crates/photo-ui/src/model/adjust.rs` | 可用 |
| 调整 tab：两行式滑杆 + 数值 chip + 单项重置 + 全部重置 +「已调整」徽标 | `crates/photo-ui/src/views/info_panel.rs:553-724` | 可用 |
| 持久化：350ms 去抖 → folder_db `adjustments` 表；切图 / Drop 强制 flush | `crates/photo-ui/src/state/app_state.rs:557-688` | 可用 |
| 预览烘焙：后台 executor，2560 母版 8-bit → `apply_tone8` → JPEG(90) | `crates/photo-ui/src/image/mod.rs:296-360` | 可用 |
| 世代序列 `adjust_render_seq` 丢弃过期帧、切图按 `adjust_path` 丢弃 | `crates/photo-ui/src/state/engine_ops.rs:414-441` | 可用 |
| 导出烘焙调整：RAW 走全尺寸 16-bit AHD | `crates/photo-engine/src/convert.rs:45-117` | 可用 |
| 16-bit 色调变换与 RAW 16-bit 预览路径（**引擎已就绪**） | `crates/photo-engine/src/adjustments.rs:131`、`convert.rs:141-196`、`rawlib::DecodeOptions::preview16()` | 未被 UI 使用 |
| 直方图 / 剪切掩码（**引擎已就绪**，6 个单测） | `crates/photo-engine/src/histogram.rs:67,125,133` | 无消费方 |
| 冒烟 `adjust_smoke` 16 项 | `crates/photo-ui/examples/adjust_smoke.rs` | 覆盖链路：装载 / 滑杆 / 更亮 / 落库 / 重置 |

---

## 2. 缺口清单

### P0 — 死控件与断链（引擎就绪、UI 零消费方）

**2.1 剪切警告按钮是死的。** `O` 键（`crates/photo-ui/src/app.rs:228`）只翻转
`state.show_clipping`（`app.rs:549`），全仓唯一读者是这个按钮自己的 `.selected()`
（`crates/photo-ui/src/views/preview.rs:697-706`）——叠加层从未渲染。
而 `engine::histogram::clipping_mask_png` 早已实现（红 = 高光溢出、蓝 = 死黑）。

**2.2 直方图完全没接线。** `engine::histogram::{compute_histogram, compute_histogram_from_file, HistogramData}`
有实现与单测，photo-ui **零调用**；手册 §7.4 / §9.7 写的是「Hero：直方图 + 剪切百分比」「预览剪切警告叠加」，
实际两处都不存在。调整时看不到「有没有拉溢出」，这是调整功能最关键的判据缺口。

### P1 — 精度与性能（ADR 0007 已承诺、未兑现）

**2.3 RAW 调整走 8-bit 母版 → 大范围拉曝光条带。** `image/mod.rs:299-300` 注释自认；
`docs/todo.md:457` 也记为遗留。ADR 设想的 half_size + 16-bit 母版未做——但
`rawlib::DecodeOptions::preview16()`、`apply_tone16`、`convert::render_adjusted` 的 16-bit 分支全都写好了，
差的只是 UI 侧改走这条路。

**2.4 有调整时 1:1 是放大母版（偏软）。** `preview.rs:181-194`：`adjust_active` 时
`one_to_one = false` 且 `need_full = false`，1:1 只能放大 2560 母版。ADR 的
「全分辨率重算在 slider 释放后异步进行」未做。

**2.5 每帧 JPEG 往返开销 + 二代损失。** 每次参数变化都把 2560 母版编码成 JPEG(90) 再交给渲染层
（`image/mod.rs:311-326`）。拖动链路上多了编码耗时，且调整效果叠在 JPEG 压缩产物上。
ADR 预算写的是「1600px tone 变换 ≤5 ms/帧」，实现是「2560 + JPEG 编码」——**没有实测数据**，
需要先量再改。

### P2 — 工作流缺口（culling 效率）

**2.6 没有批量套用。** 多选时调整 tab 仍只作用于主选中项（`info_panel.rs:573` 只拿 primary meta）。
同一光线下一整天逐张拉同一组参数是纯浪费。

**2.7 没有复制 / 粘贴调整，也没有「沿用上一张」。**

**2.8 网格 / 胶片条没有「已调整」标识。** `CaptureMeta` 不带调整信息
（`photo-domain/src/domain.rs` 的 `CaptureMeta` 无相关字段，`AdjustParams` 是独立类型），
扫完一眼看不出哪张已调。

**2.9 没有 before/after 对比。** 评估调整的基本动作，现在只能靠「全部重置 → 再拉回来」。

**2.10 无键盘操作。** 47 条 KeyBinding（`app.rs`）里没有任何调整相关项；滑杆只能鼠标拖，
没有方向键微调、双击归零、滚轮调节。

**2.11 调整不进撤销。** `Ctrl+Z` 只覆盖批量文件操作（`op_journal` / `UndoOp`），
参数改动无法回退。

**2.12 没有调整预设。** 导出侧已有 `exportPresets`（新建/保存/删除），调整侧没有对应物。

### P3 — 参数集扩展（拍板项）

**2.13 高光 / 阴影、色温 / 色调、白色 / 黑色色阶、清晰度。** 现在只有三参数，
「曝光拉完中间调发灰」没有对应手段。锐化 / 降噪不建议放进预览链路（逐帧代价高、判据弱）。

---

## 3. 逐项方案

### 3.1 剪切警告接线（P0）

- **做法**：引擎侧把 `clipping_mask_png` 拆出纯函数 `clipping_mask_rgba(&RgbImage) -> RgbaImage`，
  保留 `clipping_mask_png` 为薄包装（从文件加载的旧路径不动）；UI 侧在 toned 母版（已在内存）上
  生成掩码 → PNG 字节 → 作为叠加 Image 盖在预览主图上，`opacity 0.7`。掩码缓存键 = `(path, 参数)`，
  参数变化重算、切图丢弃；生成走后台 executor。
- **为什么按 toned 母版而不是原文件**：调整后的溢出区域才是用户要看的东西；用原文件会出现
  「加了 +1EV 但红区没变」的错觉。
- **验收**：单测（掩码在纯白/纯黑/中灰三档与 `clip_*_count` 一致）；`adjust_smoke` 加
  「`O` 键后叠加元素真的挂上，且 +1EV 后红色像素计数增加」。
- **估工**：0.5–1 d。

### 3.2 直方图接线（P0）

- **做法**：v1 不引入自定义绘制——把 256 桶降采样成 **64 桶柱状 div**（luma 面积 + RGB 三线可后置），
  数据来源就是 3.1 里那份 toned 母版：在烘焙线程顺手调 `compute_histogram(&toned)`，
  与图像一起回主线程写进 `AppState`（新增 `preview_histogram: Option<HistogramData>`）。
  同屏再给 **原图基线（幽灵柱）**，一眼看出「这一拉往哪边偏了」。
  剪切百分比（`clip_high_count / total_pixels`）直接放直方图右上角。
  计算在 worker（2560² 遍历，release 十几毫秒），主线程零像素工作。
- **位置**：调整 tab 顶部（调整时最需要），信息 tab Hero 复用同一组件。
- **验收**：引擎 6 个单测已有；`adjust_smoke` 加「直方图元素存在」「+1EV 后高亮侧计数增加」
  「全部重置后与基线一致」。
- **估工**：1–1.5 d（主要成本是柱状渲染与基线对比的视觉细节）。

### 3.3 RAW 16-bit 调整母版（P1，ADR 兑现）

- **做法**：`ImageManager` 增加 16-bit 母版缓存（`Rgb16Image`，只留焦点图一张，
  ≤40 MB/6MP，符合 ADR 预算）：RAW 用 `DecodeOptions::preview16()` half_size 解码 →
  缩到 2560（保持 16-bit）→ 缓存；参数变化只 `apply_tone16` + 降 8-bit，不重新解码。
  首次就绪走交互优先池，未就绪时沿用现有 8-bit 链路（渐进升级，不引入白屏）。
  非 RAW 保持 8-bit（直出即 8-bit，没有高位信息可救）。
- **验收**：`apply_tone16` 已有单测；新增「同一 RAW 上 +2EV 时 16-bit 路径的高光条带像素数 < 8-bit 路径」
  的对比单测（合成渐变素材即可）；`adjust_smoke` 断言 RAW 分支被走到（可加计数/日志断言）。
- **估工**：1.5–2 d。

### 3.4 全分辨率调整重算（P1）

- **做法**：滑杆拖动中仍用 2560 链路即时反馈；**滑杆释放**（`SliderEvent::Change` 带结束标志或
  去抖 200 ms）后异步跑全尺寸：RAW 走 `DecodeOptions::quality()` + `apply_tone16`，
  常规图直接读原文件 + `apply_tone8` → 与 `preview_full` 同槽位替换；世代序列沿用现有机制。
  取消语义用同一套 `AtomicBool`。
- **风险**：RAW 全尺寸 3–5 s、177 MB 缓冲——只在 1:1 且停手后触发，不需要时不跑。
- **验收**：冒烟加「释放后 1:1 图像长边 ≈ 原图长边（而非 2560）」，以及「拖动中不触发全尺寸」。
- **估工**：1–1.5 d。

### 3.5 拖动链路的编码开销 / 画质（P1，先量后改）

- **做法**：先加计时（复用 `master_bench` 或新增 `adjust_bench`）测三档：
  「2560 tone + JPEG 编码」「2560 tone 后直接喂 BGRA」「1600 链路」，拿到实测再决定。
  若编码是瓶颈，改成给渲染层直接提交 RGBA（GPUI `RenderImage`），省掉编解码与二代损失。
- **验收**：计时输出 + 拖动帧率目检；改路径后 `adjust_smoke` 全过。
- **估工**：0.5–1 d（含测量）。

### 3.6 批量套用（P2，需拍板语义）

- **做法**：调整 tab 顶部在「选中 > 1」时出现「套用到选中 N 张」；
  作用域 = 选中集 ∩ 当前筛选结果（与批量文件操作同一安全边界
  `FilterCriteria::has_active_filter` 的语义），并把参数写进 `op_journal` 以便 `Ctrl+Z` 回退。
- **验收**：`adjust_smoke` 加「3 张套用后 `adjustments` 表 3 行、撤销后回退」；
  纯逻辑单测覆盖目标集计算。
- **估工**：1 d（含撤销记录）。

### 3.7 复制 / 粘贴 / 沿用上一张（P2）

- **做法**：`Ctrl+Shift+C` 复制当前参数（内存 + 状态栏提示）、`Ctrl+Shift+V` 粘贴到选中集；
  「沿用上一张」按钮 = 把上一张的参数套到当前张（快捷键与 3.10 的曝光调节键位一起定，避免撞车）。剪贴板只走进程内状态
  （不占系统剪贴板，避免与 `Ctrl+C` 复制图片冲突）。
- **验收**：纯逻辑单测 + `adjust_smoke` 一条「复制 A → 切到 B → 粘贴 → B 与 A 参数相同」。
- **估工**：0.5 d。

### 3.8 网格 / 胶片条「已调整」标识（P2）

- **做法**：扫描回填阶段与 `enrich_meta_recognition` 同构，新增 `enrich_meta_adjustments`
  （`folder_db` 一次 `SELECT path` 拿全目录已调整集合）→ `CaptureMeta` 加 `has_adjustments: bool`
  （domain 字段，序列化门控沿用现有写法）→ 网格 cell / 胶片条右上角小徽标。
- **验收**：domain 单测（默认 false / 序列化往返）；`adjust_smoke` 加「调一张后重扫，网格徽标出现」。
- **估工**：1 d。

### 3.9 before/after 对比（P2）

- **做法**：按住 `\\`（或 `B`）时预览临时切回未调整母版（`adjust_active` 短路条件按「按住」状态
  反转），松开恢复；调整 tab 显示「按住 \\ 看原图」提示。渲染层条件判断，不引入状态恢复逻辑
  （与「调整视图隐藏识别叠加」同一套路）。
- **验收**：`adjust_smoke` 加「按住期间预览像素 ≈ 中性路径，松开后回到调整路径」。
- **估工**：0.5 d。

### 3.10 键盘与精细调节（P2）

- **做法**：新增 `AdjustExposureUp/Down` 等 action 与键位（`[` / `]` 或 `,` / `.` 调整曝光 0.05 EV，
  `Shift` 加速到 0.25）；滑杆双击标签归零；滑杆获得焦点时方向键微调（`Slider` 是否已支持需先验证）。
- **验收**：纯逻辑单测（步进映射）+ `adjust_smoke` 一条真实派发 action 改参数的用例。
- **估工**：0.5 d。

### 3.11 调整进撤销（P2）

- **做法**：撤销日志加一类 `UndoOp::Adjust { path, before, after }`（引擎 `undo.rs` 已有三类逆操作，
  外加一类成本可控）；`Ctrl+Z` 栈与批量文件操作共用同一条消息通道。
- **验收**：engine 单测 + `adjust_smoke` 一条「拉参数 → Ctrl+Z → 参数与库都复位」。
- **估工**：0.5–1 d。

### 3.12 调整预设（P2）

- **做法**：把导出预设的模式（`model/export.rs` 的 `preset_from_draft` / `preset_index_for_draft`）
  复刻到调整侧：`AppConfig.adjust_presets: Vec<AdjustParams>` + 设置页/调整 tab 的「保存为预设 / 套用 / 删除」。
- **验收**：config 单测（往返 + 钳制）+ `adjust_smoke` 一条。
- **估工**：1 d。

### 3.13 参数集扩展（P3，拍板后做）

- **建议范围**：先做**高光 / 阴影**（阴影提升是 culling 最常用的「救暗部」手段，与曝光同一次遍历可合成）
  与**色温 / 色调**（白平衡是第二高频）；色阶 / 清晰度再排。
- **注意**：新参数要同时改 domain、folder_db（`adjustments` 表加列、一次 migration）、
  `ToneParams`、8-bit/16-bit 两套实现与预览/导出两条链路——**改一处就是六处**，
  所以建议每加一个参数都配齐单测 + 冒烟。
- **估工**：每个参数 1–1.5 d。

---

## 4. 拍板项（需要你定的三件事）

1. **批量套用的作用域**：选中集 ∩ 筛选结果（推荐，与批量文件操作同边界）／仅选中集／整个目录。
2. **参数集扩展范围**：只补高光 + 阴影、还是连色温一起；锐化 / 降噪是否明确不做。
3. **是否重做裁切**：ADR 0007 修订记录里裁切是按你当时的要求整体移除的，本方案默认**不重做**。

---

## 5. 建议顺序（按价值 / 风险）

1. **3.1 剪切警告 + 3.2 直方图**（引擎已就绪、零风险，调整判据立刻可用）
2. **3.9 before/after + 3.10 键位**（小、高频）
3. **3.8 网格「已调整」标识**（culling 可见性）
4. **3.3 16-bit 母版**（RAW 用户核心价值）
5. **3.6 批量套用 + 3.7 复制粘贴**（效率翻倍，依赖拍板项 1）
6. **3.5 实测 + 3.4 全分辨率收尾**
7. **3.11 撤销 / 3.12 预设**
8. **3.13 参数扩展**（拍板后）

---

## 6. 回归网（本方案的验收清单）

- **单测**：`photo-engine`（histogram 掩码纯函数、`apply_tone16` 对比、撤销逆操作）、
  `photo-ui`（批量目标集、步进映射、before/after 状态机）、`photo-config`（预设往返）。
- **冒烟**：`adjust_smoke` 由 16 项扩到约 30 项，新覆盖——直方图元素与随参数变化、
  剪切叠加真的挂上、before/after 像素对比、批量套用落库 + 撤销回退、RAW 16-bit 分支、
  1:1 全分辨率替换、网格徽标。
- **性能**：新增计时（2560 tone / 编码 / 16-bit 路径各一档），把 ADR 的性能预算表更新成实测值。
- **文档**：手册 §7.3 / §7.4 / §9.7 与 `docs/todo.md` #17 同步（现在这三处都写着未实现）。

---

## 7. 明确不做 / 暂不做

- 裁切（已按用户要求移除，除非重新拍板）。
- 锐化、降噪（预览链路代价高、判据弱）。
- 直方图的网格与 0/255 刻度等视觉细节：v1 用 64 桶柱状，够用再说。
- GPU shader：ADR 已否决，维持 CPU 管线。

---

## 8. 实施状态

### ✅ 第 1 批（2026-09-24）：3.1 剪切警告 + 3.2 直方图

- **引擎**：`histogram.rs` 拆出纯函数 `clipping_mask_rgba(&RgbImage)`（文件路径版保留为薄包装），
  新增单测断言「掩码红/蓝像素数 == 直方图剪切计数」。
- **预览管线**：`ImageManager::render_adjusted_master` → `build_preview_frame`，**同一次烘焙**
  产出「显示图 + 直方图 + 原图基线直方图 + 可选剪切掩码 PNG」。直方图与掩码都算在
  **烘焙后的像素**上（+1 EV 后溢出变多必须看得见）；原图基线直方图按图片缓存（每张只算一次）。
- **状态与渲染**：`AppState` 新增 `preview_histogram` / `preview_base_histogram` / `preview_clip_mask`；
  新增 `model/histogram.rs`（降采样 64 桶 / sqrt 归一化 / 剪切百分比，5 个单测）与
  `views/histogram.rs`（实柱 + 基线幽灵柱 + 「高光 x% · 死黑 y%」），调整 tab 与信息 tab Hero 共用。
  预览叠加层按 0.7 不透明度叠在主图上；`O` 键打开时强制重算一次（关闭立刻撤掉叠加）。
- **验证**：`cargo check --workspace --all-targets` 0 warning；photo-engine 199 → **200**、
  photo-ui 96 → **101**；`adjust_smoke` 16 → **26 项**全过——含「直方图非空」「无调整时当前=基线」
  「+1 EV 后 clip_high 0 → 98100」「基线不随调整变化」「掩码与主图同尺寸」「+1 EV 后红色掩码 98100 像素」
  「再按 O 掩码撤除」；`lease_smoke` 15/15 无回归。目检截图（+1 EV + 剪切警告打开）：
  右栏「直方图 / 高光 10.2% · 死黑 0.0%」正常，预览红色高光带与画面条纹对齐。
- **已知取舍**：信息 tab 的直方图只在「预览里当前显示的正是这张」时渲染（网格模式不额外发起统计），
  避免把上一张的统计画到当前图上。
- **下一批**：3.9 before/after → 3.10 键位 → 3.8 网格「已调整」标识 → 3.3 RAW 16-bit 母版。

### ✅ 第 2 批（2026-09-24）：3.9 before/after + 3.10 键位与双击归零

- **before/after**：新增独立槽位 `preview_base_image`（与 `preview_image` 分开——按住 / 松开互不覆盖，
  不会「松开后闪一下旧图」）；按住反斜杠时预览渲染未调整母版，未就绪时保持调整图不闪白。
  剪切掩码算在调整后的像素上，按住看原图时**一并收起**。首次按住才后台加载一次，之后同图命中缓存。
- **键位微调**：新增 4 个 action（`[` / `]` = ±0.05 EV、`Shift+[` / `Shift+]` = ±0.25 EV），
  `AppState::nudge_adjust` 走与滑杆**完全相同**的量化 / 钳制 / 落盘 / 预览重算路径（不另开写入逻辑）。
- **双击归零**：调整行的标签挂双击（与「重置」按钮同一条路径）+ tooltip「双击归零；[ / ] 微调曝光」；
  底部说明补「按住 \ 看原图」。
- **验证**：`cargo check --workspace --all-targets` 0 warning；`adjust_smoke` 26 → **33 项**全过——
  含「] 微调 1 → 1.05」「Shift+] 粗调 1.05 → 1.3」「微调同步回滑杆」「按住后未调整母版已就绪」
  「before/after 用的是原图（149.0）」「调整帧仍留在 preview_image（两槽位互不覆盖）」「松开后标志复位」；
  `lease_smoke` 15/15、engine 200 / photo-ui 101 单测全绿。
- **目检**（新增第二个 `PHOTO_SMOKE_HOLD_MS` 钩子）：同一位置截两帧，左 = 调整后（+1.3 EV + 剪切警告），
  右 = 按住反斜杠（原图 + 无红色掩码），画面明暗与掩码消失都符合预期。
- **未覆盖**：反斜杠的**真实键盘事件**没有注入（Xvfb 无 WM 下按键投递不可靠），冒烟是直接调
  `set_before_after` 验证数据通路；app.rs 的六行键处理靠代码评审 + 编译期类型检查。
  双击归零同理（测试窗口不支持模拟带 click_count 的点击）。
- **下一批**：3.8 网格 / 胶片条「已调整」标识 → 3.3 RAW 16-bit 母版 → 3.6/3.7 批量套用与复制粘贴。

### ✅ 第 3 批（2026-09-24）：3.8 网格 / 胶片条「已调整」标识

- **模型**：`CaptureMeta.has_adjustments`（domain 新增字段，序列化门控沿用现有写法）——
  只认**非中性**参数，显式写全零行 = 已复位，不算「已调整」。
- **查询**：`folder_db::all_adjustments()` 一次 SQL 拿全集，扫描期按 rel_path 回填；
  逐张 get 会是 N 次加锁 + N 次查询，重扫时没必要。
- **即时性**：改参数后 `on_adjust_changed` 直接更新内存里的 meta，徽标立刻出现（不必等重扫）；
  重扫 / 打开新目录时从库里回填。
- **渲染**：网格 cell 左上角「格式胶囊 + 已调整胶囊」并列（原来是单独的格式胶囊，改成一行 flex，
  避免两个绝对定位元素互相压住）；胶片条右上角「调整」小徽标（96×72 里只放一个词）；
  a11y 名称同步加「已调整」（`grid_cell_label` / `filmstrip_item_label` 各加一个参数 + 单测）。
- **验证**：`cargo check --workspace --all-targets` 0 warning；photo-engine 200 → **201**；
  `adjust_smoke` 33 → **37 项**全过（含「标识立即点亮」「重置后熄灭」「重扫后从 folder_db 回填」）；
  `a11y_smoke`、`grid_scroll_smoke` 无回归。
- **目检**：网格 cell 上 `JPEG` + `已调整` 两个胶囊并排显示（第三个 `PHOTO_SMOKE_HOLD_MS` 钩子）。
- **下一批**：3.3 RAW 16-bit 母版。

### ✅ 第 4 批（2026-09-24）：3.3 RAW 16-bit 调整母版（ADR 0007 兑现）

- **引擎**：`convert::decode_raw16_master`（preview16 = half_size + 16bit + 自动亮度 + sRGB + 相机白平衡，
  长边缩到 2560 且**保持 16-bit 精度**）+ `fit_long_edge16`（纯缩放，单独可测）；`rgb16_to_rgb8` 公开给 UI。
- **像素源**：`ImageManager.base16_cache`（只留 1 张，6MP ≈ 36MB，符合 ADR 的 ≤40MB 预算）+
  `has_raw16_master` / `ensure_raw16_master`；`build_preview_frame` 的 RAW 分支优先用 16-bit 母版，
  未就绪时**先用 8-bit 出帧**并把 `used_raw16` 标成 false。
- **首帧不卡**：`render_adjust_preview` 不等解码——同时起一个升级任务（`ensure_raw16_master`），
  母版就绪后对同一张图重出一次帧（这次命中 16-bit）。`AppState.adjust_preview_16bit` 记录当前帧来源。
- **验证**：
  - engine 单测 202 → **204**：`test_tone16_keeps_shadow_levels_that_8bit_tone_loses`
    （同一段暗部 1000..1199 经 +2 EV：8-bit 路径只剩 **2** 级、16-bit 路径保留 **62** 级——色带的直接来源）、
    `test_fit_long_edge16_scales_down_only`。
  - 新增 **`raw_adjust_smoke`**（需要真实 RAW 目录，仓库内没有素材，故按参数运行）：
    真实 RW2（59MB Panasonic）实测——**首帧 622ms**（8-bit 兜底，不白屏）、
    **16-bit 升级累计 1639ms**（ADR 预算 ≤2s）、**复用 16-bit 母版再改参数 691ms**（debug，无重新解码）。
  - `adjust_smoke`（JPEG 路径）37 项无回归。
- **已知（下一批处理）**：debug 下 2560 链路单帧 ≈0.7s（含 tone + 降位深 + JPEG 编码，且冒烟轮询粒度 50ms），
  滑杆手感需要 release 实测数据再决定是否换成更快的提交路径（3.5）。
- **下一批**：3.6 批量套用 + 3.7 复制 / 粘贴 / 沿用上一张。

**第 4 批补充验证（release 实测 + 一处冒烟修复）**

- 真实 RW2 在 **release** 下的三段计时：**首帧 155ms**（8-bit 兜底）、**升级到 16-bit 773ms**、
  **复用 16-bit 母版改参数 156ms**——ADR 的「首次就绪 ≤2s」「slider 不重复解码」两条都达标。
  debug 下同一路径是 622/1639/691ms（debug 本就慢 3~4 倍，符合预期）。
- `raw_adjust_smoke` 改为**把参数指向的 RAW 复制进自己的临时目录**再扫描：这次踩到了真实教训——
  debug 跑过一次后，素材目录里已存了 +1.75 EV，release 复跑时「初始未就绪」「调整生效」两条断言
  全部失去意义（实测 3 项假失败）。冒烟自己造素材是仓库惯例，RAW 这条也一样。
- **顺手修掉一个真 bug**：`preview_full_smoke` 成功路径**没有 std::process::exit(0)**
  （只有失败路径 exit(1)），于是它一跑通就挂在 GPUI 事件循环里、永远不返回——
  以前「preview_full_smoke 无法运行」的部分原因是这个。修完在真实 RAW 上 5/5 通过
  （1:1 全分辨率图源 10099 KB），确认本批没有破坏全分辨率链路。

### ✅ 第 5 批（2026-09-24）：3.6 批量套用 + 3.7 复制 / 粘贴 / 沿用上一张

- **纯逻辑**（model/adjust.rs）：`adjust_targets`（**选中集 ∩ 筛选结果**，无选区返回空——
  绝不退化成「整个目录」，套错一整目录的代价远大于收益）、`previous_in_order`、
  `describe_adjust`；3 个单测（含「被筛掉的选中项不参与」「第一张没有上一张」）。
- **撤销**：新增 `UndoOp::Adjust { rel_path, before, after }`（一条 op 一张图，撤销报告才能逐张说清）
  + `undo_ops_with_db` / `OpJournal::undo_last_with_db`；`UndoOp` 去掉 Eq（带 f32 参数）、
  `target()`/`origin()` 改 `Option<&Path>`（本来就没有调用方，调整 op 没有文件路径）。
  Ctrl+Z 改走 `AppState::undo_last_op`（带 folder_db）——文件类 op 用不到库，行为不变
  （`batch_ops_smoke` 全过即回归证据）。2 个单测：写回 before、无库时明确报「跳过」。
- **UI**：调整 tab 新增一行按钮——复制参数 / 粘贴 / 沿用上一张 / **套用到选中 N 张**（N > 1 才出现）；
  快捷键 `Ctrl+Shift+C` / `Ctrl+Shift+V`（进程内剪贴板，不占系统剪贴板：Ctrl+C 已经是复制图片）；
  粘贴目标 = 选中集 ∩ 筛选结果，无选中 = 焦点图。
- **落库路径**：`engine_ops::apply_adjustments_batch` 逐张 upsert + 徽标即时更新 + 产出撤销记录；
  刻意**不开后台线程**——N 行 SQLite upsert 是毫秒级，走后台反而要处理「写库完成但 UI 已切图」的竞态。
- **验证**：`cargo check --workspace --all-targets` 0 warning；photo-engine 203 → **205**、
  photo-ui 101 → **104**；`adjust_smoke` 37 → **44 项**（素材从 1 张变 3 张，新增 7 项：
  批量套用三张落库 / 徽标全亮 / 复制进剪贴板 / 粘贴回三张 / 沿用上一张 0.5 EV / Ctrl+Z 撤销沿用）；
  `batch_ops_smoke` 全过（文件类撤销无回归）。
- **下一批**：3.5 性能实测 + 3.4 全分辨率重算。

### ✅ 第 6 批（2026-09-24）：3.5 性能实测 + 3.4 全分辨率重算

**3.5 先量后改**：新增 `adjust_bench`（release，中位数，各 6 次）。

| 链路 | tone | JPEG 编码 | 合计 | 编码占比 |
|---|---|---|---|---|
| 1600px (1.7MP) | 4.7ms | 23.1ms | **27.6ms** | 84% |
| 2560px (4.4MP) | 11.3ms | 58.9ms | **69.8ms** | 84% |

端到端（热缓存）：JPEG 84ms / RAW 98ms；打开剪切掩码 +20~35ms；
RAW 16-bit 母版解码 776ms（一次性）、命中后帧时 126ms。

**结论**：ADR 的性能预算（tone ≤5ms/帧）本身达标（4.7ms@1600 / 11.3ms@2560），
瓶颈是 **JPEG 编码（84%）**，不是色调变换。

**落地改动（回到 ADR 的原始设计）**：调整链路的像素源从预览母版 2560 改成
`ADJUST_SOURCE_SIZE = 1600`（ADR 原文就是「显示链路恒为 1600px」，一直没照做）——
单帧 69.8 → **27.6ms（2.5×）**。1600 在「适应窗口」下已 ≥ 视口；放大档由全分辨率链路兜底。

**3.4 全分辨率重算**：
- 引擎：`convert::render_adjusted_full`（全尺寸解码 → tone → JPEG 92；RAW 走全尺寸 AHD+16bit），
  复用导出同一条 `bake_rgb8`，单测断言「输出不被缩放」。
- UI：`AppState` 停手闸门（400ms 去抖）+ 同 (path, 参数) 去重 + 渲染世代；
  渲染层在**显示尺寸超过 1600 链路**时请求，未就绪先沿用放大后的母版（不闪白）；
  before/after 按住时不上全分辨率（那一眼看的是原图，不值得付 3 秒）。
- 结果：真实 RW2 实测 **3265ms** 得到 **8152×5432（44MP）** 的调整后全分辨率帧。

**验证**：`cargo check --workspace --all-targets` 0 warning；photo-engine 205 → **206**；
`raw_adjust_smoke` 新增「1:1 全分辨率调整帧就绪（3265ms）」「尺寸远超 1600 链路（8152×5432）」；
`adjust_smoke` 44 项、`lease_smoke` 15 项无回归。

**如实留下的两件事**：
1. 编码占 84% 意味着「直接提交 BGRA（GPUI RenderImage）」还能再快约 2 倍且免掉一次 JPEG 世代损失——
   但要把 `preview_image` 的槽位类型从 `Arc<Image>` 改成枚举，涉及渲染、before/after、冒烟多处，
   本批没做（实测数据已记录，要做时有据可依）。
2. 中段放大（约 1.5–3×）现在显示 1600 上采样：1:1 与高倍由全分辨率帧兜底，
   这是 ADR 的原始设计取舍，不是退化。

- **下一批**：3.12 调整预设（3.11 调整进撤销已在第 5 批覆盖批量与粘贴路径）。

### ✅ 第 7 批（2026-09-24）：3.12 调整预设

- **配置**：新增 `AdjustPreset`（name / exposure / contrast / saturation）与
  `AppConfig.adjust_presets`（`#[serde(default)]`，默认**空列表**——调整没有「原图预设」这种有意义的东西）；
  `clamped()` 与滑杆同口径（0.05 量化、±2.0 / ±100、NaN/inf → 0，否则会往 TOML 写出读不回来的 nan）；
  手改配置塞太多时读入截到 **20 条**。
- **纯逻辑**（model/adjust.rs）：`preset_from_params` / `preset_index_for_params` / `preset_chip_label`。
  `preset_index_for_params` **比较前把两边都钳制**（与导出预设的 `preset_index_for_draft` 同口径）——
  第一版只钳制预设侧，被单测抓到：库里一个 0.37 的旧值会永远对不上按 0.35 存的预设（chip 选不中、删不掉）。
- **UI**（调整 tab）：预设行——chip 点一下套用（选中态 = 当前参数命中该预设）、「保存当前参数」、
  「删除当前预设」（按当前参数命中判定，避免删错）。
- **语义**：套用走与批量/粘贴**完全同一条** `apply_adjustments_to`（作用域 = 选中集 ∩ 筛选结果，
  无选中 = 焦点图），因此**同样可 Ctrl+Z 撤销**，不需要单独的撤销路径。
- **验证**：`cargo check --workspace --all-targets` 0 warning；photo-config 20 → **22**、photo-ui 104 → **106**；
  `adjust_smoke` 新增 4 项（保存 0.85 EV / 套用只动目标张 / chip 选中态命中 / 删除后清空）；
  `lease_smoke` 15 项无回归。
- **下一批（最后一批）**：3.13 高光/阴影 + 色温/色调——要动 domain 字段、folder_db 迁移、
  8-bit/16-bit 两套色调查表与预览/导出两条链路。

### ✅ 第 8 批（2026-09-24）：3.13 高光/阴影 + 色温/色调 + 3.11 收尾

**参数集 3 → 7**：新增阴影 / 高光 / 色温 / 色调。

- **domain**：4 个字段 + Default + is_neutral。
- **folder_db**：**一次 migration 补 4 列**（末尾 ALTER；老库与新建库走同一条链，schema 一致）+
  put/get/all/copy/解析全部升到 7 列；顺手把「非有限曝光」在解析层归零（手改库塞 nan 不再传染进查表）。
- **色调实现（关键设计）**：四条件**全部折进查表**——每通道各一张 R/G/B 表：
  曝光/对比度仍走旧路径（**逐位不变**，既有曝光/对比度/饱和度测试就是回归网），
  阴影/高光在编码域叠加（pivot 0.5、权重线性、幅度 = k×权重×0.5），色温/色调是通道增益
  （±25% R/B 反向；色调 ±10% R/B、∓20% G）。**每像素成本不变**：仍是一次查表（+可选饱和度定点），
  多出来的只是每帧 3×65536 次的表构建。
- **UI**：`AdjustField` 3 → 7（新增 ALL / index / range / label / format_value）；滑杆实体改成
  **与 ALL 同序的数组**、循环创建与订阅；调整 tab 的七行也改成循环——**以后加参数只需改枚举**。
- **预设**同步扩到 7 个字段（配置 + 钳制 + chip 文案）。
- **3.11 收尾**：单张滑杆的改动也进撤销——一次连续拖动**只记一条**（改前值在改动起点快照，
  落盘时写 `UndoOp::Adjust`），与批量/粘贴/预设共用同一条 Ctrl+Z 通道。

**验证**：`cargo check --workspace --all-targets` 0 warning；photo-engine 206 → **211**
（阴影提亮暗部且中灰不动 / 高光回收亮部 / 色温暖冷方向 / 色调品红绿方向 / 8-bit 与 16-bit 一致），
且**既有曝光·对比度·饱和度测试逐位未变**；photo-config 22（新字段钳制）；photo-ui 106；
`adjust_smoke` 全过——新增的像素级断言：**阴影 +100 最暗 99 → 127**、**高光 -100 均值 161 → 127**、
**色温 +100 R 161 → 201 / B 161 → 121**、**单张滑杆改动 Ctrl+Z 色温 -100 → 0**；
`batch_ops_smoke` / `lease_smoke` / `raw_adjust_smoke`（release，真实 RW2，含 16-bit 新曲线与
1:1 全分辨率 8152×5432）全过。

---

## 9. 全部批次完成（2026-09-24）

| 批次 | 内容 | 状态 |
|---|---|---|
| 1 | 3.1 剪切警告 + 3.2 直方图 | ✅ |
| 2 | 3.9 before/after + 3.10 键位与双击归零 | ✅ |
| 3 | 3.8 网格 / 胶片条「已调整」标识 | ✅ |
| 4 | 3.3 RAW 16-bit 调整母版 | ✅ |
| 5 | 3.6 批量套用 + 3.7 复制/粘贴/沿用 + 3.11（批量路径） | ✅ |
| 6 | 3.5 性能实测 + 3.4 全分辨率重算 | ✅ |
| 7 | 3.12 调整预设 | ✅ |
| 8 | 3.13 高光/阴影 + 色温/色调 + 3.11（单张路径） | ✅ |

**交付后的能力**：七个调整参数（曝光/对比度/饱和度/阴影/高光/色温/色调）+ 实时直方图（含原图基线）+
剪切警告叠加 + 按住 \ 看原图 + `[` `]` 微调与双击归零 + 网格/胶片条「已调整」标识 +
RAW 16-bit 高质量链路 + 1:1 全分辨率重算 + 批量套用/复制粘贴/沿用上一张 + 全链路 Ctrl+Z +
调整预设（配置文件持久化）。

### 修订（2026-09-24，用户拍板）：曝光范围与步进

曝光 **±2.0 → ±3.0 EV**、步进 **0.05 → 0.3 EV**（≈1/3 档）；键盘微调「[」「]」= ±0.3、
Shift = ±0.9；AdjustPreset::clamped 与 ToneParams::from 的钳制同步。
本文前面各批次记录里的「±2.0 / 0.05」是当时状态，不再适用（ADR 0007 已追加同一条修订）。

**仍明确不做 / 留待**：
1. **BGRA 直提交**（去掉 84% 的 JPEG 编码成本，估计再快约 2 倍且无二代损失）——实测数据已在第 6 批记录；
2. 中段放大（约 1.5–3×）显示的是 1600 上采样（1:1 与高倍有全分辨率帧兜底，属 ADR 原始设计取舍）；
3. 锐化 / 降噪：明确不做（逐帧代价高、判据弱）；
4. 裁切：按用户要求保持移除。
