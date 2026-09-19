# Photo Tool GUI 设计手册（框架无关）

> 用途：把 ftpt 的界面**照着重做一遍**——用 GPUI / Qt / egui / 任何 GUI 框架。
>
> 本文只描述**与框架无关的契约**：结构、状态模型、算法公式、交互语义、视觉令牌、异步与事件协议。
> Vue/DOM/CSS 的实现细节（组件名、Tailwind 类、reka-ui 行为）一律剥离，只保留"必须成立的行为"。
>
> 三份文档的分工：
>
> | 文档 | 内容 |
> |---|---|
> | `CONTEXT.md` | 领域术语（Capture / 堆叠 / 画面 …） |
> | `docs/adr/` | 架构决策 |
> | **本文** | **界面与交互的可移植规格** |
>
> 基线：本手册成文时的参考实现是 Tauri v2 版（`crates/photo-tauri/`，**已于 2026-09-18 删除**）；
> 当前实现是 GPUI 版 `crates/photo-ui/`。行为契约以本文为准，实现与本文冲突时改实现，或顺手改本文。

---

## 0. 怎么用这本手册

移植按这个顺序读，每一步都能独立验收：

| 步骤 | 读 | 交付物 |
|---|---|---|
| 1. 定骨架 | §1 §2 §6 | 窗口外壳 + 五列布局 + 主题令牌跑起来 |
| 2. 定状态 | §3 §4 | 纯函数层（筛选/排序/分组/选优/缩放数学）+ 可测 |
| 3. 接后端 | §7 §8 附录 A | 扫描 → 网格显示照片，图片管线通 |
| 4. 接交互 | §5 §9 §10 | 键位、选择、标记、视图切换全通 |
| 5. 补齐 | §9.9–§9.11 §12 | 弹窗、覆盖层、右键菜单、平台能力 |

**唯一不可省略的验收**：§5.4 的 `Esc` 优先级链、§5.3 的标记作用域、§8.3 的"生成号 + 取消"三件套。
这三处错了会**误删用户照片**或**把旧目录的元数据写到新目录**，其余都可以后补。

---

## 1. 设计立场

一句话：**键盘优先、高密度、照片是屏幕上唯一高亮的照片筛选（culling）工具。**

| 立场 | 具体约束（可验收） |
|---|---|
| **键盘优先** | 每一条鼠标路径都有一个等价键位，且两者调用**同一个动作函数**，不允许两套代码 |
| **照片是唯一高亮** | 表面为校准级中性灰；全界面只有一个 accent 色；网格与预览中只有照片和标记色是彩色的 |
| **高密度、零装饰** | 基准字号 15px 等比缩放；行高紧凑（顶栏 44 / 状态栏 24；标题栏默认交给系统，§2.5）；动效预算只花在三处（空态、菜单、瞬态提示） |
| **筛选驱动操作** | 一切批量动作（文件操作/重命名/导出/识别范围）的对象 = **当前筛选结果**；无激活筛选时批量操作**直接禁用** |
| **单一事实 + 乐观更新** | 扫描结果一次下推为前端权威副本；标记先改本地、失败回滚；后端只推事件 |
| **不重复造原生** | 窗口控制、缩放热区、系统目录对话框、剪贴板、打开外链一律走平台 API；日期用原生日期控件、滑杆用原生 range |

**世界隐喻**：Material You · 墨白（§6）——中性灰底衬照片，表面全黑白灰，
彩色只出现在图标与边框（accent 一族）上。

---

## 2. 界面结构

### 2.1 解剖图

```
┌──────────────────────────────────────────────────────────────────────┐
│ Header    44px   左:目录名+项数 │ 中:网格/预览/统计 tab │ 右:进度+刷新+设置│
│ （标题栏默认由系统提供，不占应用布局；仅客户端装饰时自绘 36px，见 §2.5）│
├────┬─────────────┬───────────────────────────────┬───────────┬───────┤
│ L  │ LeftPanel   │  FilterBar (仅网格态, ≥36px)   │ InfoPanel │  R    │
│rail│ 文件树/文件操作│ ─────────────────────────────  │ 信息/调整 │ rail  │
│48px│ 200–480px   │  PhotoGrid / PhotoPreview /    │ 200–480px │ 48px  │
│    │ 可拖宽       │  Slideshow / Stats             │ 可拖宽    │       │
├────┴─────────────┴───────────────────────────────┴───────────┴───────┤
│ StatusBar 24px   目录 · 项数/已选 · 扫描/识别状态 · 瞬态提示            │
└──────────────────────────────────────────────────────────────────────┘
```

根容器：占满屏幕、纵向 flex、`overflow: hidden`、1px 窗口描边、圆角 8px；
**最大化时去圆角去边框**（否则贴边露缝）。其上有窗口投影（亮/暗两套）。

### 2.2 尺寸表

| 区域 | 尺寸 | 备注 |
|---|---|---|
| 标题栏 | 系统提供 | 服务端装饰时不占应用布局；客户端装饰时自绘 36px（窗口按钮 44×36 满高） |
| 顶栏 | 44px | 三组 flex-1，中组固定宽 |
| 左/右活动栏 | 48px | 竖排图标按钮，间距 4 |
| 左栏 | 200–480，默认 220 | 拖把手在**右缘** 4px，`col-resize` |
| 右栏 | 200–480，默认 200 | 拖把手在**左缘** |
| 筛选栏 | 折叠 36px / 展开自适应 | 仅网格视图 + 已打开目录时渲染 |
| 状态栏 | 24px | 三段式，数字用等宽（`tabular-nums`） |
| 网格行高 | `cellW + 56`，下限 80 | 行距 8，容器内边距 4 |
| 胶片条 | 80px + 6px 自绘滚动条 | 仅预览态 |

### 2.3 面板宽度持久化

宽度写配置 AppConfig.leftPanelWidth（默认 200）/ rightPanelWidth（默认 200），读取时钳制 200–480。
GPUI 版由 Dock 停靠区尺寸承担（DockPlacement::Left/Right）：拖宽实时改内存、**350ms 去抖**后落盘。

> ⚠️ 历史：Tauri 版前端走本地存储与后端配置字段双轨；GPUI 版已统一为**只跟随配置文件**。
> `right_panel_visible` 仍未使用。

### 2.4 显示 / 隐藏

| 操作 | 键位 | 入口 |
|---|---|---|
| 左栏 | `Ctrl+[` | 左活动栏首按钮（图标随状态切换，`aria-pressed` 语义） |
| 右栏 | `Ctrl+]` | 右活动栏按钮 |

隐藏用**保留状态**的方式（不销毁实例）：目录列表、批量表单、滚动位置、tab 选择都不丢。

### 2.5 窗口装饰与标题栏

默认**请求系统装饰**（`WindowDecorations::Server` + `titlebar.title`）：合成器支持时
标题栏由系统绘制，应用不再占 36px、也不重复画一遍窗口按钮。
合成器不支持时 gpui 会回落客户端装饰——此时才自绘 36px 标题栏，下表能力必须全部具备。

| 能力 | 做法 |
|---|---|
| 拖拽窗口 | 标题栏整体作为拖拽区（子元素也可拖，按钮除外） |
| 双击最大化 | 标题栏双击 → 切换最大化 |
| 窗口按钮 | 最小化 / 最大化⇄还原 / 关闭；关闭按钮 hover 用 destructive 底色 |
| 最大化状态同步 | 查询一次 + 订阅 resize 事件双向同步 |
| 八向缩放热区 | 4 边 + 4 角；边 6px 厚（两端各留 12px 避开圆角），角 8px 见方；**最大化时整体隐藏** |
| 首次渲染兜底 | 主题接管前按**亮色**渲染（内联底色），避免白屏闪烁 |

---

## 3. 状态模型

### 3.1 唯一事实

**后端权威副本**（打开目录后一次性下推，前端持有全量）：

```
CaptureMeta {                        // 每图片文件一条；扫描不配对
  index, baseName, primaryPath, primaryFormat,
  fileSize?, dateTaken?, extensions[],
  cameraMake?, cameraModel?, lens?, exposureTime?, fNumber?, iso?, focalLength?,
  imageWidth?, imageHeight?, gpsLat?, gpsLon?, focusPoint?,
  rating, colorLabel, flag?, keywords[],
  birdName?, birdConfidence?, recognitionStatus?, birdBbox?, eyeSharpness?
}
```

**前端状态分三类**，移植时严格区分，否则会写出互相打架的双份状态：

| 类别 | 内容 | 变更方式 |
|---|---|---|
| 权威副本 | `items: CaptureMeta[]`、`directory`、缩略图版本号 | 扫描/重扫/回滚时整体替换 |
| 纯派生（零 IPC） | 筛选结果下标、排序、堆叠组、连拍组、选中集、缩放平移、可见行 | 由权威副本 + 用户输入同步算出 |
| 任务状态机 | 扫描 / 识别 / 批量 / 导出 / 导入 / 重复检测 / 技术分 | 事件驱动 + 哨兵位 |

### 3.2 派生管线（顺序不可换）

```
items
  └─ filterCaptures(criteria)          → 通过筛选的下标（升序）
       └─ sortBy + sortDirection       → display_order（稳定排序）
            └─ groupStacks / groupByTime / singles  → StackGroup[]（网格显示项）
            └─ computeBurstGroups       → 连拍组映射（仅 size ≥ 2 登记）
            └─ pickBestFrame（每组）     → 最优帧路径集合
```

**关键不变式**：
- 方向键、`Shift` 范围选择、`Ctrl+A` 全选、批量操作的对象**全部基于 display_order**，不是原始 `items` 顺序。
- 堆叠只影响**显示层**，不影响选中集：选中集存的是 `items` 下标。

### 3.3 视图状态机

`view ∈ { grid, preview, slideshow, stats }`，另有一个独立的 `mapOverlay: bool`。

| 视图 | 渲染 | 说明 |
|---|---|---|
| `grid` | 网格 + 筛选栏 | 默认态、空态落点 |
| `preview` | 单图 + 胶片条 | 缩放/平移/叠加层 |
| `slideshow` | 3s 自动播放 | 进入前视图记在 `viewBeforeSlideshow` |
| `stats` | 全局鸟种索引 | 退出固定回网格 |

**渲染用显式条件链而非 else 链**（空态包裹在过渡里会打断链式渲染）：

```
Stats → 空态(无目录) → Slideshow → Preview → Grid
```

优先级：统计与空态最高；无目录时非统计视图一律落到空态（居中图标 + "打开目录"按钮）。

**转移表**：

| 从 → 到 | 入口 |
|---|---|
| grid → preview | `G` / 双击 cell / 左活动栏图片按钮 / 顶栏 tab / 右键"在预览中打开" |
| preview → grid | `G` / `Esc` / 工具条"网格" / 顶栏 tab |
| grid·preview → slideshow | `S`（从当前选中张在筛选序中的位置开始） |
| slideshow → 来源视图 | `Esc` / `G`（还原 `viewBeforeSlideshow`） |
| 任意 → stats | `T` |
| stats → grid | `Esc` / `G` / "退出"（清空缓存，下次进入重拉） |
| 任意 → 地图 | `M`（全屏浮层，**不改变 view**） |

`G` 是**唯一"切换"语义键**：网格/预览间切换；在幻灯片/统计态则退回网格。

### 3.4 缩放入口状态

| 状态 | 值 | 含义 |
|---|---|---|
| `zoom` | `1` | 适应窗口（fit） |
| `zoom` | `0` | 1:1（显示尺寸 = 原图像素，**图源切全分辨率**） |
| `zoom` | 其他 | 相对 fit 的倍率 |

滚轮/±键步进 **×1.25**。百分比显示相对**原图自然尺寸**（不是相对 fit），
否则"适应"与"1:1"都会显示 100%。

---

## 4. 纯算法规范

这一层是**移植的第一优先**：无 IO、无 UI 依赖、可单元测试。
下面给出语义与边界；实现语言不限，但边界必须逐条对齐。

### 4.1 筛选 `filterCaptures(items, criteria) → number[]`

`FilterCriteria` 字段与语义（全部为"未设 = 不限制"）：

| 字段 | 语义 | 边界 |
|---|---|---|
| `formatFilter: ImageFormat?` | 格式精确匹配（大小写归一） | **RAW 是通配语义**：真实数据 `primaryFormat` 存的是扩展名（`NEF`/`CR3`/`ARW`），不是字符串 `RAW`；芯片值为 `RAW` 时匹配任意非标准格式，带具体扩展名时精确匹配 |
| `birdNames: string[]` | 命中任一即保留 | 空 = 不限 |
| `dateFrom` / `dateTo` | 闭区间，比较 `YYYY-MM-DD` | `dateTaken` 为 null 且设了区间 → **排除**；有值但解析失败 → **保留**（历史实现无 else 分支，保持一致） |
| `minRating: Rating?` | **≥ N** 语义 | `None` = 0，不满足 ≥1 |
| `colorLabel: ColorLabel?` | 精确匹配 | — |
| `flagFilter: Flag?` | 精确匹配 Pick/Reject | — |
| `unflaggedFilter: bool` | 只显示无旗标 | 与 `flagFilter` 互斥 |
| `recognitionFilter` | All / Confirmed / NeedsReview / Unrecognized / **NotRecognized** | `NotRecognized` = **无识别记录**（`recognitionStatus == null`），不是 Unrecognized |
| `isoMin` / `isoMax` | 闭区间 | 无 ISO 且设了区间 → 排除 |
| `focalMin` / `focalMax` | 闭区间，mm | 解析失败 → 排除 |
| `lensFilter: string[]` | 精确匹配 EXIF lens 串 | 空 = 不限 |
| `keywordFilter: string[]` | 命中任一即中 | 空 = 不限 |

**焦距解析**：取字符串中**第一个**数值（`"70-200mm"` → 70，广角端）。对鸟类拍摄场景保守且可预期。

**`hasActiveFilters(criteria)`**：任一字段非默认即为真。
**这是批量文件操作的安全边界**——无筛选时批量操作按钮禁用，防全目录误操作。

### 4.2 排序 `compareCaptures(sortBy, a, b) → -1|0|1`

| 键 | 比较 | null 语义 |
|---|---|---|
| `FileName` | `baseName` 小写字典序 | — |
| `DateTaken` | `dateTaken` 字符串序（null → `""`） | 排最前 |
| `FileSize` | 数值 | null → 0 |
| `Rating` | 0–5 数值 | None = 0 |
| `Modified` | 用 `dateTaken` 代理 mtime（桌面端可换真实文件 mtime） | null 排最前 |
| `EyeSharpness` | 数值升序 | **null 排最前** |
| `Quality` | 数值升序（技术分来自内存表） | **null 排最后**（未评分不参与机筛） |

降序 = 外层整体反转比较结果（不是各自写一遍降序逻辑）。
排序必须**稳定**（同键保持 display 原序）。

### 4.3 堆叠分组

三种模式，键与组位置规则：

| 模式 | 分组键 | 组位置 | 说明 |
|---|---|---|---|
| `None` | `i-<下标>` | 自身 | 每成员独立成组（渲染路径统一，单成员组无堆叠 UI） |
| `ByFileName` | `<stem>`，跨目录时 `<stem>@<dir>` | 组内成员在显示序中的**最小位置** | 同 stem 因筛选/排序被打散时聚合到最先出现的位置；跨目录同 stem **必须分组**（不是同画面） |
| `ByTime` | `t-<组内最早时间戳>` | 同上 | 相邻时间戳差 ≤ **2000ms** 聚类（与连拍阈值一致）；无时间/解析失败 → 独立单成员组 `x-<下标>` |

**激活成员**（网格显示哪一张）：按主路径**真实扩展名**在优先级表中取最优：

```
jpg > jpeg > png > tif > tiff > gif > webp > bmp > heic > heif > avif
```

全部未命中（纯 RAW 堆叠）→ 回退组内首个成员。用户手动切换后由激活覆盖值接管。

**组键稳定性**：`ByTime` 用最早时间戳而非组序号——筛选变化时若最早帧仍在组内，组 id 不变，激活覆盖不失效。

### 4.4 连拍分组 `computeBurstGroups(items, gapMs = 2000) → Map<下标, {groupId, size, pos}>`

- 按传入顺序（**显示序**）遍历，相邻两项拍摄时间差 ≤ 2000ms 归同组。
- 时间为 null / 解析失败 = 独立单张，**且切断组链**（前后两段即使时间接近也不合并）。
- 只登记 `size ≥ 2` 的组；空输入返回空。

**EXIF 时间解析**：兼容 `2026:06:28 10:15:30` 与 `2026-08-01T10:15:30`；
分隔符不限定；字段越界（13 月）必须**先拦截**——`Date.UTC` 会静默进位。

### 4.5 最优帧 `pickBestFrame(members) → path | null`

确定性全序（并列全部打破，不依赖输入顺序）：

```
1. eyeSharpness 降序（null 视为 -1，垫底）
2. fileSize 降序（null 视为 0）
3. primaryPath 字典序升序
```

`size < 2` 返回 null。`nonBestPaths(group)` = 组内非最优帧路径（保持组序），供"保留最优"批量标 Reject。

### 4.6 缩放 / 平移数学

```
centerOffset(disp, container) = (container - disp) / 2

clampPanAxis(disp, container, pan):
  if disp <= container: return 0            // 小图不允许平移
  center = centerOffset(disp, container)
  return clamp(pan, container - disp - center, -center)

fitScale(cw, ch, iw, ih) = min(cw/iw, ch/ih, 1)   // 小图不放大

panAfterCursorZoom(oldDisp, newDisp, container, pan, cursor):
  // 保持光标下的图像点不动
  oldOrigin = centerOffset(oldDisp, container) + pan
  r = newDisp / oldDisp
  newOrigin = cursor - (cursor - oldOrigin) * r
  return newOrigin - centerOffset(newDisp, container)   // 逐轴独立
```

锚点约定：
- 滚轮缩放 → **光标位置**为锚点
- 工具条 ± 与 `=`/`-` 键 → **容器中心**为锚点
- 平移按显示尺寸钳制（越界回弹）

### 4.7 命名模板 `renderNameTemplate(template, ctx) → string`

占位符 `{name} {species} {date} {seq} {camera}`；未知占位符**原样保留**；
`{date}` 归一为 `YYYYMMDD`（格式不符 → 空串）；`{seq}` 补零 3 位。
结果做文件名清洗：去掉 `/ \ : * ? " < > |` 与控制字符、连续空白折叠、去首尾空白与尾部句点；
渲染后为空 → 回退原名。

### 4.8 网格虚拟化

```
COLS   = gridColumns (2–5)
cellW  = (gridW - PAD*2 - (COLS-1)*GAP) / COLS          // PAD=4, GAP=8
rowH   = max(80, round(cellW) + 56)
rowStep= rowH + GAP
spacer = PAD*2 + rowCount*rowStep - GAP                  // 撑高容器
可见行 = [floor(scrollTop/rowStep) - 2, ceil((scrollTop+vh)/rowStep) + 2] ∩ [0, rowCount)
cell 绝对定位：x = PAD + col*(cellW+GAP)，y = PAD + row*rowStep
```

行高含 56px 信息区：文件名 / 文件大小 + 五星 + 鸟种状态行（**常驻固定高度 18px**，避免高度跳动）。

### 4.9 胶片条虚拟化

```
ITEM_W = 96, GAP = 6, BUFFER = 6
total  = n*ITEM_W + (n-1)*GAP
可见区 = [floor(scrollLeft/step) - 6, ceil((scrollLeft+vw)/step) + 6]
滚动条：thumbW = max(client/total * trackW, 24)     // 自绘 track + thumb，可点击可拖拽
```

---

## 5. 交互契约

### 5.1 选择语义

选中集是**有序、去重**的 `items` 下标数组，外加一个 `anchorIndex`（范围基准 + 方向键焦点）。

| 操作 | 行为 |
|---|---|
| 单击 | 替换为 `[i]`，anchor = i |
| `Ctrl`/`Cmd` + 单击 | 切换 i；插入时保持网格顺序；移除锚点时锚点挪到选中集末尾 |
| `Shift` + 单击 | 以**显示序区间**替换选中集，anchor 不变（锚点不在筛选结果内则退化为单击） |
| `Ctrl+A` | 全选**当前筛选结果**（不是全部 items） |
| `Ctrl+D` | 清空 |
| `←` / `→` | 移动一个**堆叠组**（折叠为单选），`Home`/`End` 首尾组 |
| 主选中项 | `anchorIndex ?? 选中集末尾`（右栏与预览读它） |

`Shift` 范围**必须走显示序**：直接填 `items` 下标区间会把中间被筛掉的隐藏项一起选中，
之后 Delete / 标记会误伤它们。

### 5.2 标记写入（乐观更新）

```
mutateOptimistic(paths, apply, remote):
  prev = 快照受影响项
  items = items.map(受影响项 → apply)      // 先改本地，UI 立刻响应
  try { await remote() }                   // 调后端
  catch { items = prev }                   // 失败回滚
```

评分 / 色标 / 旗标 / 关键词四类共用同一骨架。
删除（回收站，**无确认**）走删除命令 + 全量重扫 + 清空选中；若删的是当前预览图则一并退出预览。

### 5.3 标记键作用域 `markPaths()`

评分/色标/旗标/删除**共用同一目标解析**，避免幻灯片模式误伤未显示的项：

```
幻灯片态 → 当前显示张
其余     → 选中集
```

判定依据是**视图状态**（而非某个 active 字段），退出后立即恢复选中集语义。

### 5.4 `Esc` 优先级链（命中即返回，一次 Esc 只关一层）

```
1. 地图浮层打开        → 关地图
2. 设置弹窗打开        → 关设置
3. 识别进行中          → 取消批量识别
4. 幻灯片态            → 退出幻灯片
5. 统计态              → 退出统计
6. 有框选待确认框      → 清除框选
7. 预览态              → 返回网格
```

两个例外必须保留：
- 右键菜单的 `Esc` 由菜单自身**先**吃掉（菜单比全局键位先注册），不会顺带触发"退出预览"。
- **导入对话框点遮罩/Esc 不关闭**（有状态的扫描/计划流程，误触代价大），只允许右上角 ×。

### 5.5 键位解析规则

1. 焦点在文本输入/可编辑区时，**全局键位不触发**（用户输入优先）。
2. 修饰键**精确匹配**：绑定未声明的修饰键不检查；`Ctrl+B` 与 `Ctrl+Shift+B` 用 `shift:false/true` 区分三态。
3. 键名规范化：方向键 → `left/right/up/down`，`Escape` → `escape`，`F5` → `f5`，字母统一小写。
4. 命中即**吞掉事件**（阻止默认行为：滚动/全选/刷新）；无匹配不拦截。

### 5.6 键位总表

**导航与视图**

| 键 | 动作 | 生效范围 |
|---|---|---|
| `←` / `→` | 上一个 / 下一个 | 网格 = 堆叠组间；幻灯片 = 切张（重置计时） |
| `Home` / `End` | 首 / 末 | 堆叠组 |
| `Q` / `E` | 堆叠内上一个 / 下一个成员 | 仅网格态 |
| `G` | 网格 ⇄ 预览 | 幻灯片/统计态 → 退回网格 |
| `S` | 进入幻灯片 | 从当前选中张开始 |
| `空格` | 幻灯片暂停 / 继续 | 仅幻灯片态 |
| `T` | 统计视图 进入 / 退出 | 全局 |
| `M` | GPS 地图 进入 / 退出 | 全屏浮层 |
| `=` / `-` | 放大 / 缩小 | 预览 |
| `Esc` | 取消 / 关闭 | 按 §5.4 优先级链 |

**标记（作用于 `markPaths()`）**

| 键 | 动作 |
|---|---|
| `1`–`5` / `0` | 评分 1–5 星 / 清除评分 |
| `6` `7` `8` `9` / `Ctrl+6` | 色标 红 / 黄 / 绿 / 蓝 / 紫 |
| `P` / `X` / `U` | 旗标 入选 / 淘汰 / 清除 |
| `K` | 连拍选优：非最优帧标 Reject（带确认弹窗） |
| `Delete` | 删除到回收站（不区分修饰键，无确认） |

**识别与叠加**

| 键 | 动作 |
|---|---|
| `B` / `Ctrl+B` / `Ctrl+Shift+B` | 识别当前所选 / 识别全部未识别 / 重新识别全部 |
| `V` | 检测框（含鸟眼角标） |
| `F` | 对焦点叠加（仅预览态） |
| `O` | 剪切警告叠加（仅预览态） |

**选择 / 面板 / 文件**

| 键 | 动作 |
|---|---|
| `Ctrl+A` / `Ctrl+D` | 全选当前筛选结果 / 取消选择 |
| `Ctrl+Z` | 撤销最近一次批量操作（移动/复制/重命名） |
| `Ctrl+[` / `Ctrl+]` | 切换左 / 右栏 |
| `F5` | 重扫当前目录 |

**设计约定**：单字母动作 `G/C/S/T/M/V/F/O/K`；`B` 是唯一三态修饰键；
方向键语义随视图变化但**始终是"当前位置的相邻"**；`Delete` 与 `Esc` 是最常按的两个键。

---

## 6. 视觉系统（Material You · 墨白）

> 本节取代 2026-09 的「接触印相」亮色改版与更早的「观片灯箱 / 暗室」两套隐喻。
> 落点：`crates/photo-ui/src/theme/`（`material.rs` 色彩引擎、`scheme.rs` 方案与预设、
> `mod.rs` gpui 令牌映射）。

### 6.1 立场

Material You（Material 3）的色彩体系，加一条本产品的硬约束：**黑白为底，彩色只上图标与边框**。

- **表面**全部来自 seed 色相的**零彩度**中性色调板——真正的黑、白、灰，既不偏蓝也不偏暖；
  照片是界面上唯一的大块色彩。
- **彩色**只出现在 accent 一族：主按钮、焦点环、选中描边、活动 tab 下划线、进度条、
  活动栏高亮图标。这是「允许彩色图标和边框」的落点——是状态表达，不是装饰。
- 领域语义色（危险 / 成功 / 警告 / 信息 / 色标 / 旗标 / 星级）不随 seed 变。
- 界面上**不再有第二个强调色**：一个 seed 决定 accent，其余全部灰阶。

生成管线：seed `#RRGGBB` → HCT（CAM16 色相 / 彩度 + CIE L\* 明度）→ 各 tone →
亮/暗两套 gpui-component `ThemeConfig`，运行时注册进 `Theme` 并 `Theme::change` 切换。

### 6.2 生成规则

| 色调板 | 色相 | chroma | 说明 |
|---|---|---|---|
| neutral | seed 色相 | **0** | 所有表面；tone 即 CIE L\*，纯灰阶 |
| accent | seed 色相 | seed.chroma < 8 → 0；否则 clamp(24, 48) | accent 一族 |

- **单色退化**：seed 近乎无彩（chroma < 8，含默认 `#111111`）→ accent 直接用中性板，
  亮色主色取 N12（近黑）、暗色主色取 N95（近白）。这就是默认的「墨白」。
- **彩色**：主色取 M3 标准 tone（亮 P40 / 暗 P80），hover/active 向亮暗各走一档。
- 中性 tone 与 M3 一致 = CIE L\*：`N0 #000000`、`N40 #5e5e5e`、`N50 #777777`、
  `N92 #e8e8e8`、`N100 #ffffff`。任意 tone 都可由 HCT 求解，表内取值即由此生成。

### 6.3 令牌映射（tone 取自 §6.2 的板）

| gpui-component key | 亮 | 暗 | 用途 |
|---|---|---|---|
| `background` | N92 | N5 | 网格画布 |
| `foreground` | N10 | N92 | 正文 |
| `sidebar` / `title_bar` / `status_bar` | N100 | N11 | 常驻 chrome |
| `popover` | N100 | N17 | 浮层 / 顶栏 |
| `list` / `table` / `button` / `accordion` | N100 | N13 | 内容面 |
| `group_box` / `muted` / `secondary` / `skeleton` / `tiles` | N95 | N17 | 次级面 |
| `muted_foreground` / `tab_foreground` | N45 | N64 | 弱化文字 |
| `accent`（hover 底）/ `list_hover` / `table_hover` | N94 | N21 | 悬浮底（保持中性） |
| `secondary_hover` / `secondary_active` | N92 / N89 | N22 / N27 | 次级按钮状态 |
| `border` / `table_row_border` | N88 / N92 | N25 / N20 | 发丝分隔 |
| `input` | N60 | N32 | 输入描边 |
| `window_border` | N72 | N30 | 窗口描边 |
| `list_head` / `table_head` / `description_list_label` | N96 | N15 | 表头 |
| `list_even` / `table_even` | N98 | N14 | 斑马纹 |
| `slider_bar` | N85 | N30 | 滑杆轨 |
| `switch` / `switch_thumb` | N80 / N100 | N32 / N90 | 开关 |
| `scrollbar_thumb` / `hover` | N70 / N60 | N35 / N45 | 滚动条（默认半透明） |
| `primary` / `ring` / `caret` / `link` / `progress_bar` / `drag_border` / `slider_thumb` | accent | accent | **彩色落点** |
| `primary_foreground` | N100 | P20 或 N14 | accent 上的字 |
| `primary_hover` / `primary_active` | 见 §6.4 | 见 §6.4 | 主按钮状态 |
| `selection` / `list_active` / `table_active` / `drop_target` | P90 | P30 | 选中底（accent 容器） |
| `list_active_border` / `table_active_border` | P40 | P80 | 选中描边 |
| `tab` / `tab_active` | 透明 / N100 | 透明 / N17 | tab |
| `tab_bar` / `tab_bar_segmented` | N95 / N100 | N15 / N17 | tab 轨 |
| `overlay` | N0 @ 40% | N0 @ 60% | 弹窗遮罩 |
| `danger` / `success` / `warning` / `info` | 见 §6.7 | 见 §6.7 | 语义状态 |

### 6.4 accent 一族（seed 驱动的全部彩色）

| 角色 | 单色（墨白）亮 / 暗 | 彩色亮 / 暗 |
|---|---|---|
| `primary` | N12 / N95 | P40 / P80 |
| `primary_foreground` | N100 / N14 | N100 / P20 |
| `primary_hover` | N26 / N100 | P32 / P88 |
| `primary_active` | N6 / N86 | P22 / P70 |
| `primary_container`（选中底） | N90 / N26 | P90 / P30 |
| `on_primary_container` | N12 / N92 | P12 / P90 |

**只重写 accent 一族**：表面与中性色永远不随 seed 变。这就是「固定黑白底 + accent 由 seed 驱动」
世界的核心。

### 6.5 圆角 / 密度 / 投影

- 基准字号 `font.size = 15`；全 `rem` 等比缩放（GPUI `text_*` / `gap_*` / `p_*` helper）。
- 圆角**只有 4px 一档**：`radius = 4`（输入框 / 卡片 / 按钮 / 网格 cell / 弹窗 / 浮层）、
  `radius_lg = 4`。**不使用 `Theme::radius_full()`**——色片、旗标角标、色标、HUD 条、
  活动栏竖条一律 4px 圆角，不做胶囊与圆形。
  **视图里禁止固定值圆角**（`.rounded(px(4.))` / `.rounded_md()`），一律走 `cx.theme().radius`。
- 表面一律**平**（`shadow: false`）：分层靠发丝边框与底色差，不靠投影堆叠。
  只有最顶层浮层保留一层软投影——`theme::overlay_shadow()`（弹窗 7 例 + 预览缩放 HUD）。
- 间距基数 4：`gap` 用 8 / 6 / 4 / 3 / 2，容器内边距 4 / 3。
- 滚动条 8px 常驻轨道，悬停/滚动才显 thumb。
- 全局禁止文本选择（工具型应用防误选），输入框与可编辑区例外；图片禁止拖拽。

### 6.6 设置里的主题（§9.10 通用设置）

- **明暗**：浅色 / 深色两个分段按钮，立即生效并写回 `config.toml` 的 `theme`。
- **主题色**：12 个预设色块（第一个「墨白」为默认）+ 自定义 `#RRGGBB` 输入框，
  立即重建两套 `ThemeConfig` 并写回 `accentColor`。
- 持久化：`AppConfig.theme`（Light / Dark）与 `AppConfig.accentColor`（`#rrggbb`，
  读入即归一，非法值回落 `None` → 默认墨白）。
- 切换语义：只改 accent 一族与明暗底，**不动任何布局与尺寸**；键盘焦点保持。

### 6.7 领域色（亮暗同表，去饱和可辨）

| 令牌 | 亮 | 暗 | 用途 |
|---|---|---|---|
| `label-red` | `#d9534f` | 同 | 色标红 |
| `label-yellow` | `#c98a1c` | 同 | 色标黄 |
| `label-green` | `#4a9f63` | 同 | 色标绿 |
| `label-blue` | `#4f7fd0` | 同 | 色标蓝 |
| `label-purple` | `#8b6fd0` | 同 | 色标紫 |
| `pick` / `reject` | `#3a9a5f` / `#d64c44` | 同 | 入选 / 淘汰旗标 |
| `rating` / `focus` | `#c98a1c` | 同 | 星级 / 对焦点叠加 |
| `danger` | `#b3261e` | `#f2b8b5` | 危险 |
| `success` | `#2e7d4f` | `#6dd58c` | 已识别 / 入选 |
| `warning` | `#8a5300` | `#f5b944` | 待复核 |
| `info` | `#1f5fa8` | `#a8c7fa` | 信息 |

**情景色白名单**（其余一律灰阶）：危险操作、成功/淘汰旗标、对焦与检测框叠加、色标五色、
星级、图表/直方图。这些是数据与语义，不是装饰。定义在 `theme::mod` 的常量里，
**不允许在视图代码里写裸色值**。

### 6.8 状态表达

| 状态 | 表达 |
|---|---|
| 选中（网格 / 胶片条） | 2px accent 环（未选中用 2px 透明边框占位，避免布局跳动） |
| 悬浮 | 表面升一档灰（`accent` / `list_hover`），不靠投影 |
| 焦点 | 2px accent 焦点环（`ring`） |
| 危险 | M3 错误色 + 动词标签，不与其他按钮共用形状暗示 |
| 禁用 | 只降文字对比，不改变形状 |
| 当前目录 | accent 容器底 + 左缘 2px accent 竖条（`theme::accent_dim`） |

**黑白之后状态必须靠形状与权重**，不能只靠颜色：色标是方片、旗标角标是 4px 圆角方块、
分段控件是按钮而不是胶囊；全界面圆角只有 4px 一档（§6.5）。

---

## 7. 图片管线

### 7.1 三档图源（关键设计，决定性能）

图片**不走 IPC 传字节**，而是通过自定义协议/本地路径直接给渲染层：

| 档 | 内容 | 用途 |
|---|---|---|
| `thumb` | 磁盘缓存的网格缩略图 JPEG | 网格 cell、胶片条、预览占位 |
| `master` | 预览母版，长边 2560（含调整参数烘焙） | 预览主图 |
| `full` | 1:1 全分辨率 | 预览在 1:1 时切到此档 |

**缓存策略**：
- 缩略图落盘在**照片目录**下 `.pt/thumbs/`（每文件夹独立，随目录搬移）；
  缓存键 = `hash(格式版本 + 变体 + 路径 + 请求尺寸 + 文件大小)`，文件名 `{:016x}.jpg`。
  **文件大小必须参与键**——同名文件被覆盖（重拍导回）时旧缓存自动失效；
  格式版本常量在解码逻辑修复时递增，一次性废掉全部旧缓存。
- RAW 母版按特殊键存一份；网格缩略图 / 预览 / 全分辨率都从母版派生（不额外落盘）。
- 内嵌 JPEG 长边 ≥ 2048 时**直接用内嵌**作母版，省一次完整解码。
- 常规图优先用 EXIF 内嵌缩略图。

**生成与刷新（GPUI 版 photo-ui）**：扫描成功后启动**缩略图后台生成管线**
(`state::engine_ops::start_thumb_pipeline`)：

1. 按 `display_order`（可见顺序）挑出「磁盘上还没有缩略图」的照片，跳过视频等非图片格式；
2. 3 个并发 worker 共享游标，各自调 `ensure_thumb` 写盘；取消沿用 `scan_cancel`——新扫描即中止旧管线；
3. 每 250ms 回主线程写 `thumb_done / thumb_total` 并 `cx.notify()`；状态栏显示「缩略图 n/m」。

**渲染侧不需要逐张回传图像**：网格每帧都重新查一次缓存路径（`get_cached_thumb_path` 只做一次
`path.exists()`），缩略图落盘后下一次绘制即命中——这是「检查式刷新」，比 Vue 版的
`thumb:ready` + URL 版本号更简单，前提是渲染频率足够（进度通知正好提供了这个节奏）。

### 7.2 预览加载序列（避免模糊与跳变）

```
1. 立刻用 thumb 占位（同显示尺寸/原点），主图淡入覆盖 → 无布局跳变
2. 同时请求 master
3. master 到达 → 200ms 淡入替换
4. 用户切 1:1 → 请求 full；用母版占位并按 EXIF 尺寸**预布局**，消除 2x 跳变
5. 无缩略图 → 卡片式加载占位（图标脉冲 + 200ms 淡出）
```

### 7.3 调整参数烘焙（ADR 0007，无 crop）

调整 tab 三条滑杆：曝光 ±2.0 EV（步进 0.05）/ 对比度 ±100 / 饱和度 ±100。

- 拖动只改内存（`AppState::set_adjust_field` → 量化 + 钳制），**350ms 去抖**后写回
  `folder_db.adjustments`；切图 / 换目录 / 退出（`Drop`）前强制 flush——flush 必须用
  「写入当时」的 `current_dir + folder_db`，否则相对路径会算到新目录上。
- 参数持久化在文件夹数据库的 `adjustments` 表（键为相对路径，分隔符统一为 `/`）。
- 预览重算在后台 executor：`ImageManager::render_adjusted_master` 取 2560 母版像素
  （内存缓存最近 2 张，拖动期间只重算色调 + 重编码，不重复解码原图）→ `apply_tone8`
  → JPEG → 回主线程写回 `preview_image`；**主线程零像素工作**。
  母版是 8-bit，RAW 与 JPEG 同口径；ADR 设想的 RAW half_size + 16-bit 母版尚未实现
  （大范围拉曝光会有条带，见 §13 遗留）。
- 渲染世代 `adjust_render_seq` 只认最新一次结果（快速拖动丢弃过期帧），切图后按
  `adjust_path` 丢弃；未调整母版加载晚于调整预览完成时不覆盖它。
- 参数全零 → 短路回未调整母版（ADR「零回归」）；有调整时 1:1 不切原图
  （原文件没有烘焙参数，否则调整效果静默消失）。全尺寸重算属遗留项。

### 7.5 视频与非图片格式（决定：不解码、不抽帧）

`mp4 / mov / m4v / avi` → `ImageFormat::Other`——只被扫描、归类、计数，**不进入图片管线**。

- **网格**：主题 `muted` 占位 + 胶片图标 + 「视频」；左上角徽标显示**真实扩展名**（`MP4`/`MOV`），不再统一显示 `OTHER`（与中央重复）。
- **预览**：不解码、不抽帧，只给一条出口——**「用默认软件打开」**（`open::that`，后台任务调用）。
  缩放 / 检测框 / 对焦点这些叠加语义对视频不成立，所以视频态工具条只保留「返回网格」。
- **明确不做**（2026-09 决定）：不打包 ffmpeg（实测发行版依赖闭包 **223 MB**，自制最小构建也要 10–20 MB），
  也不走"解析内嵌封面"的零体积路子——实测相机 MOV 无 `covr` atom，覆盖不到真实文件。
  收益（照片库中视频占比约 **0.1%**：1664 张照片 vs 2 个视频）不匹配这个体积成本。

### 7.4 直方图与剪切警告

- 直方图：256 级 luma（BT.601 加权）+ RGB 三通道计数 + 剪切统计。**canvas 手绘**：
  luma 面积 + 主线，RGB 三色半透明细线，每 64 级网格，底部 `0/255` 刻度。
  颜色取主题令牌，**主题切换时必须显式重绘**。
- 剪切警告：与图同尺寸的 PNG 叠加，`opacity 70%`，红 = 高光溢出、蓝 = 死黑。
- 两者都有**内存缓存**（按完整路径），切图复用避免重复解码。

---

## 8. 异步与事件协议

### 8.1 通用长任务形态

所有长任务都是同一个模式，移植时**统一实现一次**，各处复用：

```
状态机：idle → running(progress: done/total/current) → done(summary) → idle
        running 期间可 cancel（置位取消令牌，worker 逐张检查后提前退出）
UI：细进度条 + n/m + 当前文件名 + ✕ 取消（Esc 同义）
完成：状态栏摘要，4s 自动消失
```

任务：扫描、识别、批量操作、导出、导入、重复检测、技术质量评分。

### 8.2 事件表（后端 → 前端，共 17 个通道）

| 事件 | 负载 | 用途 |
|---|---|---|
| `scan:progress` | `{stage: scan\|exif\|thumb, done, total}` | 扫描三阶段进度 |
| `scan:done` | `{total, directory}` | 扫描结束（含启动自动恢复场景） |
| `capture:enriched` | `{indices[]}` | EXIF 批量回填完成（增量重排） |
| `thumb:ready` | `{path}` | 缩略图就绪 → 版本号递增刷新 |
| `recognize:progress` | `{done, total, currentPath}` | 识别逐张 |
| `recognize:done` | `{total, confirmed, needsReview, unrecognized, failed}` | 识别汇总 |
| `batch:progress` / `batch:done` | `{done,total,currentPath}` / `{success,failed}` | 批量文件操作 |
| `export:progress` / `export:done` | 同形态 | 批量导出（独立通道，避免与批量操作互相覆盖） |
| `import:progress` | `{done, total, current}` | 导入逐文件 |
| `import:open` | `{path}` | 外部入口（设备动作/单实例转发）→ 打开导入对话框并预选源 |
| `import:done` | `{imported, skipped, failed}` | 导入汇总 |
| `duplicates:progress` / `duplicates:done` | `{done,total}` / `{groups: string[][], error?}` | 近重复检测 |
| `quality:progress` / `quality:done` | `{done,total,currentPath}` / `{total, scores:[path,score][]}` | 技术质量分 |

约定：事件接线集中在**一处初始化**（用一个哨兵位防重复订阅）；
异步回填一律带**序号守卫**（防止切图/切目录后旧请求串数据）。

### 8.3 两个必须实现的一致性机制

**① 扫描生成号（generation）**

每次扫描把生成号 +1；后台的 EXIF 提取与缩略图任务都带着自己的生成号，
回填前检查"我还是当前代吗"，不是就**丢弃结果**。没有这个，切目录时旧任务会把
**上一个目录的元数据写到新目录的列表里**。

**② 目录任务追踪（per-directory task tracker）**

批量识别 / 重复检测 / 技术分三类任务各持一个：
- 防并发（同类型任务运行中拒绝再启动）
- 切目录时**只取消该任务**，不复用旧令牌
- 旧 worker 完成后**不能**复位新任务的状态

### 8.4 两阶段破坏性操作

文件操作、导入、导出都是**干跑预览 → 确认 → 执行**：

```
① 干跑（不碰文件）：返回条数 + 逐条清单（源路径 → 目标路径）+ 额外拉入的兄弟文件数
② 用户确认（显示前 20 条清单 + "其中 M 个来自同名同步"警告）
③ 执行：逐文件推进度；结果明细（成功/失败 + 失败原因可滚动列表）
④ 可撤销（移动/复制/重命名）：单槽撤销日志，Ctrl+Z 执行逆操作 + 全量重扫
```

**批量操作的对象边界**：`paths` 显式传入（= 当前筛选结果）；**空数组 = 空操作集**，
绝不回退成全量目录（防"筛选后误操作整个目录"）。

### 8.5 启动自愈

启动时后端会按上次目录自动恢复扫描（缓存命中约 200ms），
**扫描完成事件可能早于界面挂载**。因此前端初始化时必须主动：
读配置的 `lastDirectory` + 拉一次全量 captures 补齐状态。否则界面会显示空目录。

---

## 9. 区域细则

### 9.1 顶栏（44px）

- **左**：当前目录名（截断，tooltip 全路径）+ `N 项`；无目录显示"未打开目录"。
- **中**：`网格 / 预览 / 统计` 下划线 tab（**签名元素**）——
  底部 2px accent 下划线，未激活用透明占位防跳动；点击与 `G`/`T` 走同一动作。
- **右**：扫描进度（32px 细条 + 阶段名 + `done/total`，total 未知时脉冲）、
  `全部识别`（识别当前目录全部**未识别**照片，`Ctrl+B` 同义；label 带剩余张数，识别中/无剩余时禁用并改 label）、
  `刷新目录`（无目录/扫描中禁用，`F5` 同义）、`设置`（图标按钮，吸最右）。

### 9.2 左活动栏（48px）

图标按钮：左栏显隐（`Ctrl+[`）、打开照片目录（`Ctrl+O`）、进入预览（`G`）、主题切换。
识别不再放左栏（2026-09 删除该图标）：触发入口为 `B` 键（所选）、顶栏「全部识别」（整目录未识别，`Ctrl+B`）
与右栏「重新识别」（当前张）。

### 9.3 左栏（文件树 / 文件操作 双 tab）

顶部**官方 segmented 双 tab**（`TabBar::new(..).segmented()`，medium，tab 高 32），切 tab 保留各自状态。

**文件树**
- 操作区：`导入`（主按钮）、`收藏当前目录`（星标填充态，无目录隐藏）。
- 当前目录卡片：图标瓦片 + 目录名 + `N 张`，用 `dir-card-active` 样式；右键 = 加入/取消收藏。
- **子目录 / 收藏 / 最近打开** 三个分区共用同一个**目录行**（`render_dir_row`）：
  左对齐、统一图标槽（子目录 `Folder` / 收藏 `Star` / 最近 `FolderOpen`）、
  hover 用 accent 底、当前目录用 accent 容器底高亮；右侧 10px 弱化字显示**上级目录名**
  （多个同名"图片"靠它区分），完整路径进 `aria_label`。
  **不要用 Button 铺满整行**：gpui-component 的 Button 内容默认居中，名字长短不一时
  看起来就像缩进错乱的树（这正是本次返工的原因）。
- 行点击 = 走同一扫描流程（`defer_entity_action` → `start_scan`，见工程约定）。
- 底部：`重复照片`。

**文件操作**：见 §10.4。

### 9.4 筛选栏（仅网格态）

**折叠态一行（36px，横向滚动）**：
`筛选` 折叠按钮（有激活筛选时 accent + 加粗）→ **摘要 chips**（每个带 × 单独清除）→ 弹性空白 →
`排序` 下拉（文件名 / 拍摄日期 / 文件大小 / 评分 / 修改时间 / 眼锐度 / 技术分）→ 升降序 →
`技术分` 触发按钮（进行中显示 `n/m`）→ `每行图片数 2–5` 下拉。

**展开态**按条件组排布（可换行，组间发丝分隔）：

| 组 | 控件 | 语义 |
|---|---|---|
| 格式 | 全部 / JPEG / PNG / TIFF / WebP / BMP / GIF / HEIF / RAW / OTHER | 单选，点当前值取消 |
| 评分 | `1★…5★` | "≥N" 语义，点当前值取消 |
| 旗标 | 任意 / 入选 / 淘汰 / 未标记 | 互斥单选 |
| 识别 | 全部 / 已识别 / 待复核 / 未检测到 / 未识别 | 单选 |
| 色标 | 任意 / 红 / 黄 / 绿 / 蓝 / 紫 | 单选 |
| 日期 | 两个日期输入 | `YYYY-MM-DD` 闭区间 |
| ISO / 焦距 | 数字输入 ×2 | 空 = 该侧不限 |
| 镜头 | 多选下拉（候选 = 目录内 distinct lens，带搜索） | 精确匹配多选 |
| 关键词 | 输入框，回车/逗号（中英文）分隔 | 命中任一即中 |
| 鸟种 | 多选下拉（名录并集已选，搜索过滤，拼音排序） | 多选 |
| 清除全部 | 右侧，仅在有激活筛选时出现 | — |

### 9.5 网格

**Cell 内容（自外向内）**：
- 缩略图（`object-cover` 正方裁切）。
- 非图片格式（OTHER）显示**居中格式徽标**而非破图。
- 左上：格式徽标（半透明黑底；RAW 显示扩展名如 `NEF`）。
- 右上：旗标角标（18px 圆，入选绿底白勾 / 淘汰红底白叉）。
- 右下：技术质量分角标（≥0.75 绿点 / <0.4 红点，中档不显示）。
- 左下：连拍徽标（**仅单成员组**，`Layers + n`；最优帧带 amber 皇冠）。
- 底部：**堆叠成员带**（多成员组，见 §9.5.1）。
- 信息区（56px）：文件名 / 文件大小 + 逐位彩虹五星 / 鸟种状态行（常驻 18px）。
- 底缘：3px 色标条。

**选中态**：2px accent 环 + 2px 间隙（用**环**而非边框——与 cell 边缘留呼吸缝且不撑动布局）；
锚点项 100% 不透明，其余 70%。

**交互**：单击选中 / `Ctrl`+单击切换 / `Shift`+单击范围（显示序）/ 双击进预览 / 右键菜单。
方向键移动后按显示位**精确计算滚动位置**（不要依赖 scrollIntoView 的居中行为）。

**滚动条**：网格右缘叠一条 8px 半透明圆角 thumb（滚动 / 悬停显现，空闲淡出）。
GPUI 的溢出滚动容器**只滚不画**——必须 `uniform_list.track_scroll(&AppState.grid_scroll)`
绑定跨帧句柄，再把 `Scrollbar::vertical(&handle)` 叠在同一视口；thumb 的位置与长度由句柄的
offset / content_size 推导，滚轮、thumb 拖拽、键盘精确滚动三处共用同一份状态。

#### 9.5.1 堆叠成员带

- 多成员组在 cell 底部显示 28px 成员缩略图横排，当前成员 accent 描边。
- 点击直达激活并选中；成员 > 4 时两端浮出半透明左右按钮（滚到头自动隐藏），滚轮横滚。
- **语义徽标区分两种堆叠**：多格式（同画面，蓝色 Copy 图标）/ 连拍多帧（橙色 Layers 图标）。
- 连拍徽标只在**单成员组**显示，避免与成员带重复。

### 9.6 预览 + 胶片条

**图片区**：内边距 16px；图源 `master`（1:1 时 `full`）。

**叠加层**（全部不接收指针事件，跟随缩放平移）：

| 叠加 | 键 | 形态 |
|---|---|---|
| 检测框 | `V` | 2px accent 边框 + 20% 填充 |
| 鸟眼角标 | 随 `V` | 眼框**四角 L 形臂条**（臂长随框大小夹紧 4–14px，不遮眼） |
| 对焦点 | `F` | Point = 十字准星（臂长 14px）/ Circle = 圆 / Rectangle = 框，用 `focus` 色 |
| 剪切警告 | `O` | 同尺寸 PNG 叠加，70% 不透明 |
| 框选实时框 | — | `Shift`+拖拽，accent 实框 |
| 框选待确认框 | — | 虚线框，`Esc` 或再次 `Shift`+拖拽清除 |

**工具条**：底部居中胶囊浮条（半透明卡片底 + 背景模糊 + 阴影），
自身吞掉 pointerdown/wheel 事件（不触发图片拖拽与缩放）：
`−` / 百分比（相对原图；加载中带旋转图标）/ `+` / `适应` / `1:1` /
**堆叠成员分段选择器**（多格式显示格式徽标 `JPG·CR3`，连拍显示帧号；最优帧带皇冠；滚轮横滚）/
`检测框` / `对焦点` / `网格`。

**胶片条**：高 80px，横向虚拟列表，点击跳转选中、当前项 accent 描边、选中变化自动横向滚动居中；
**滚动条独立在缩略图下方**（自绘 track + thumb），避免原生滚动条压住图片；
滚轮纵向 → 横向（阻止冒泡到预览缩放）；缩略图右键 = 在预览中打开 / 复制图片 / 复制路径。

**框选**：`Shift` 按下光标变十字；拖拽实时框钳制在图片范围内；
抬起时位移 < 8px 视为误触**不产生**待确认框；产生的归一化框（0–1）虚线显示。

### 9.7 右栏 InfoPanel（信息 / 调整 双 tab）

顶部**信息 / 调整**官方 segmented 双 tab（与顶栏视图 tab、左栏同一规格）；下方卡片流，卡片用 `panel-card`。

**信息 tab（七张卡，无选中时统一显示"未选择图片"）**

1. **Hero**：直方图 + 高光剪切/死黑百分比 + 文件名 + 格式徽标 + 分辨率 + 文件大小（+ 鸟种中文名）。
2. **拍摄信息**：默认只显示 **2×2 曝光四格**（焦距 / 光圈 / 快门 / ISO，发丝网格 + 等宽加粗）；
   `更多/收起` 展开：相机、镜头、日期、位置（十进制 6 位 + 地图链接）、对焦点（形状 + 归一化坐标）。
3. **识别**：状态 chip（已识别绿 / 待复核橙 / 未检测到灰）+ 鸟名 + 置信度条
   （≥80% 绿 / ≥50% 橙 / <50% 蓝）+ 失败阶段中文 + 最接近候选 + 眼锐度（tooltip 展示评分公式）
   + 动作行 `纠正…` / `重新识别` / `显示或隐藏检测框`。
4. **评分**：五星点选（点当前星 = 清除）+ `清除`。
5. **颜色标签**：五个色圆 + "无"圆，选中项加前景色描边。
6. **旗标**：入选 / 淘汰 / 无 三按钮互斥。
7. **关键词**：chips（× 删除）+ 输入框（回车/逗号分隔）。

> **作用域**：右栏所有标记卡作用于**选中集**（多选批量），显示值取主选中项。

**调整 tab**：三条滑杆（曝光 / 对比度 / 饱和度）。
**两行式布局**：标签 + 数值 chip + 单项重置一行，滑杆**独占整行全宽**——
单行布局在 200px 宽下不可用。用 gpui-component 的 `Slider`（`SliderState` 实体 + Change 事件订阅）；
顶部在非中性时显示「已调整」徽标与「全部重置」，底部一行说明「参数随图片保存，原图永不被修改」。
无选中照片时与信息 tab 同款「未选择图片」空态。

### 9.8 状态栏（24px）

三段：左 = 目录全路径（截断，tooltip）；中 = `N 项 · M 已选`（已选 > 0 用 pick 绿）；
右 = 状态区，**互斥优先级**：

```
扫描中 > 识别中 > 识别完成摘要 > 空提示 > 撤销提示 > 就绪
```

识别中显示：脉冲图标 + `识别中 n/m` + 4px 进度条（72px 宽）+ 当前文件名（截断）+ ✕（`Esc` 同义）。
摘要与撤销提示 **4s 后自动消失**。

进度来自 `engine_ops::start_recognition` 的后台 worker 回传：同步 ONNX 推理跑在后台线程，
前台每 250ms 收一次结果并重绘。**不能把同步推理循环直接写在前台执行器里**——那样渲染与
事件循环会被整批识别占住，这里的 `n/m` 直到全部结束才闪一下（2026-09-19 修）。

### 9.9 右键菜单（单一实例）

- 单一实例，挂到窗口根；遮罩层点击/右键关闭。
- 渲染后按**自身实际尺寸**钳制到视口（右下缘留 4px）；
  主菜单贴近右缘（< 320px）时子菜单**向左**展开。
- 菜单项形态：普通项 / 勾选项 / 子菜单（hover 展开一层）/ 分隔线；危险项用 destructive 色。
- 入场 scale 0.97 + 上移 2px + 淡入。

**菜单项来源**：

| 目标 | 项 |
|---|---|
| 网格 cell | 在预览中打开 → 识别（多选显示 `识别所选照片 (N张) (b)`）→ 纠正鸟种… → `评分`▸ / `颜色标签`▸ / `标记`▸（勾选当前值）→ 导出… → 删除（移至回收站，危险） |
| 预览图片 | 返回网格 → …同上… → 复制图片（单选时）→ 缩放组（放大/缩小/适应窗口/实际像素）→ 删除 |
| 胶片条缩略图 | 在预览中打开 / 复制图片 / 复制路径 |
| 文件夹行 | 打开 / 加入或取消收藏 / 从最近移除 |

### 9.10 弹窗家族

统一规格：遮罩 + 居中、圆角 12、进出场 250ms（scale 95→100 + fade）、宽度上限 `100% - 2rem`。
自绘头栏的弹窗把 × 放在标题右侧。

| 弹窗 | 尺寸 | 关闭策略 | 结构要点 |
|---|---|---|---|
| **设置** | 高 640，宽 ≤ 860 | 遮罩 / `Esc` / × | 左侧竖 tab（通用 / 快捷键 / 关于）+ 右侧滚动内容；**改动即保存** |
| **导入** | 宽 ≤ 576，高 ≤ 85vh | **仅 ×**（遮罩/Esc 不关） | 顶部 `导入 / 添加` 分段；流程见 §10.3 |
| **导出** | 宽 512，高 ≤ 85vh | 遮罩 / `Esc` / × | 预设下拉（新建/保存/删除）+ 长边 + JPEG 质量滑杆 + 命名模板（实时预览第一张）+ 目标目录 |
| **重复照片** | 高 576，宽 ≤ 832 | 遮罩 / `Esc` / × | 阈值下拉（6/8/10/12/16，默认 10）+ 检测按钮 + 分组卡片横排缩略图 + "保留第一张，其余标 Rejected" |
| **纠错** | 宽 416，高 ≤ 85vh | 遮罩 / `Esc` / × | 当前识别条 + Top-5 候选 + 常用 chips（本机高频）+ 名录搜索（300ms 防抖，中文/拼音/拉丁）+ 应用（N 张） |
| **连拍选优确认** | 宽 ~448 | 遮罩 / `Esc` | 说明规则（眼锐度→文件大小→路径序）+ "标记 Reject" 危险按钮 |
| **批量操作** 3 个 | 目标目录 md / 删除 md / 进度 sm | 进度不可关 | 见 §10.4 |

**设置三页**

1. **通用**：外观（主题亮/暗、主题色 8 预设 + 取色器、界面字体、界面缩放 75–200%）；
   网格（每行图片数 2–5、堆叠模式三态含一句说明）；
   识别（线程数 1–4、鸟体定位来源：YOLO 检测 / 相机对焦点）；
   扫描（包含子目录，**切换后立即重扫**）；配置文件（打开 config.toml）。
2. **快捷键**：从键位表生成，分五组（常用操作 / 标记 / 识别 / 选择 / 面板），
   键位渲染为按键胶囊（`Ctrl+Shift+B` 形式，同动作多绑定用 `/` 分隔）。
3. **关于**：应用/版本/框架版本/组件库/样式/识别引擎/模型/名录库；
   配置与缓存位置说明；运行日志路径 + 打开日志文件/目录。

### 9.11 覆盖层

- **地图浮层**：全屏；头部（标题 + `N 张有 GPS / M 张无 GPS` + 关闭）；
  地图瓦片 + 带 GPS 的照片上图（白描边 + accent 填充圆点），单点 zoom 6、多点 fitBounds；
  点击 marker 弹窗：缩略图 + 文件名 + "定位到网格"按钮（关地图 → 回网格 → 选中该张并滚动定位）。
  **暗色主题下瓦片压暗**（brightness .72 / saturate .82），保持"照片是唯一高亮"。
- **进度弹窗**：批量操作 / 导出 / 导入各一个，`n/m · 当前文件` + 细进度条，**不可关闭**。
- **toast**：批量 / 导入 / 导出 / 统计导出共用模式——顶部居中（导出为底部），宽上限 80vw，4s 自动消失。

---

## 10. 关键流程

### 10.1 打开目录 / 启动恢复

```
启动 → 后端自动恢复上次目录扫描（缓存命中 ~200ms，事件可能早于界面挂载）
界面 → 初始化时主动读配置 lastDirectory + 拉全量 captures 补齐（自愈）
用户 → 顶栏刷新(F5) / 左栏导入→添加 / 收藏·最近卡片单击 / 空态"打开目录"
     → 同目录且已有数据则跳过；否则扫描 → 进度 → 完成 → 重拉全量
     → 失败必须复位"扫描中"哨兵并清空旧目录（否则旧数据 + 重试守卫死锁）
```

### 10.2 浏览与标记（culling 主循环）

```
网格 → 方向键/Q/E 定位 → 1-5 评分 / 6-9 色标 / P·X·U 旗标（可 Ctrl/Shift 多选批量）
     → 双击或 G 进预览（V 检测框 / F 对焦点 / O 剪切 / 滚轮缩放 / 拖拽平移）
     → S 幻灯片过片
     → 筛选栏收窄（旗标=入选 + 评分≥3）→ 进入批量操作
```

### 10.3 导入（SD 卡）

```
① 源：可移动盘列表（点击即扫描）或浏览目录（扫描只 stat 取文件时间，不读 EXIF）
② 目标根目录（选择或手输，不存在时自动创建）+ 子目录模式（3 种日期格式 / 不建子目录）+ 重命名模板
③ 导入方式：复制（源保留）/ 移动（跨设备自动回退复制+删除）
④ 生成计划预览（干跑）：N 张 → M 个目标目录，跳过清单前 20 条（同名同大小冲突）
⑤ 开始导入 → 进度条 → 成功且 0 失败：状态栏提示 + 自动关闭 + 打开导入结果目录（强制重扫）
   有失败：留在面板看结果明细（成功/跳过/失败）
外部入口：命令行 `--import <挂载点>`（启动参数或单实例转发）→ 打开对话框并预选源、立即扫描
```

### 10.4 批量文件操作（筛选驱动，两阶段）

```
前提：必须有激活筛选（否则三按钮禁用 + 黄色警告条）
① 操作类型：移动 / 复制 / 删除（移动·复制需选目标目录，拒绝"目标 = 源目录"并提示）
② "同步同名文件"开关（默认关）→ 格式多选 chips（默认全选目录内出现的格式）
③ "开始执行" = 干跑预览（只算不动文件）
④ "确认执行（N 个）"：
   删除 → 红色确认框（前 20 条清单 + "其中 M 个来自同名同步"警告）→ 确认
   移动/复制 → 直接执行
⑤ 进度弹窗 → 结果明细（成功/失败 + 失败原因）
⑥ 撤销：Ctrl+Z（执行逆操作 + 全量重扫），结果显示在状态栏
同一面板还有：批量重命名（模板 + 起始序号 + 实时预览）与"导出…"
```

### 10.5 识别与纠错

```
B（所选）/ Ctrl+B（全部未识别）/ Ctrl+Shift+B（重新识别全部）/ 左活动栏按钮
→ 状态栏进度（n/m + 当前文件名 + ✕/Esc 取消）→ 完成摘要 4s
→ 网格 chip / 右栏识别卡；单图失败细分阶段（检测/分类/名录映射/源图不可用）
纠错："纠正…"或右键"纠正鸟种…" → Top-5 候选 / 常用 chips / 名录搜索 → 应用（批量 N 张）
```

### 10.6 其他

- **导出**：预设 / 长边 / 质量 / 模板（实时预览）/ 目标目录 → 进度 → toast。
- **重复检测**：左栏底部 → 阈值 → 检测（dHash + 汉明距离贪心聚类）→ 分组卡片 →
  "保留第一张，其余标 Rejected"（走旗标链路，网格/筛选即时生效）。
- **统计**：`T` → 顶部四卡（鸟种/照片/文件夹/平均命中率）+ 左栏鸟种排行（占比条 + 首见~末见 + 平均锐度）
  + 命中率条形图（弱项在前，样本 < 3 标"样本少"）+ 右栏该鸟种照片网格；
  双击照片 → 切到所在目录并选中 → 退出统计回网格；"导出记录" → CSV。
- **地图**：`M` → 全屏；弹窗"定位到网格" → 关地图 + 回网格 + 选中。

---

## 11. 平台能力映射（移植时必须逐项找替代）

| 能力 | 当前实现 | 移植要点 |
|---|---|---|
| 系统目录选择 | 平台对话框插件 | 用目标框架的原生文件对话框；不要把路径输入框当替代 |
| 打开外部链接 | 平台 opener | 桌面框架的 `window.open` 等价物常被静默忽略，必须走专用 API |
| 打开配置文件 / 日志 | 系统默认程序 / 文件管理器定位 | Linux `xdg-open` + 文件管理器 D-Bus 接口 |
| 剪贴板 | 写文本 / 写图片 | 图片写剪贴板需要平台图像格式转换 |
| 单实例 + 外部入口 | 命令行参数 `--import <path>` + 已运行实例事件转发 | 桌面动作（KDE Solid / Windows AutoPlay）唤起时把窗口拉到前台 |
| 无边框窗口控制 | 平台窗口 API（拖拽/最大化/缩放热区） | GPUI 用 `TitleBar::window_options()`；Qt 用 `Qt::FramelessWindowHint` + 手写命中测试 |
| 自定义图片协议 | 应用自定义 scheme（三档流式 serve） | GPUI/Qt 直接把文件路径交给图像解码器更简单，但要自己做三档缓存与解码线程池 |
| 窗口权限 | 显式能力清单 | 沙箱化框架需声明；非沙箱框架可跳过 |

---

## 12. GPUI（gpui-kit / gpui-component 0.6.x）落地映射

> 面向 `gpui-kit = 0.6.1`（`use gpui_kit::*` 即 GPUI，`gpui_kit::component` = gpui-component）。
> **动手前先读** `gpui-component` 技能里的 Design Guides / Coding Guides——那里是硬性要求，
> 本节只做"本项目的映射建议"，不替代它。

### 12.1 crate 布局

```
photo-ui/            (新 crate，替代 photo-tauri)
  src/
    app.rs           根视图（布局骨架 + 键位上下文）
    state/           captures / selection / view / filter / tasks（Entity<State>）
    model/           §4 纯函数（无 gpui 依赖，独立 #[test]）
    views/           title_bar / header / left_panel / filter_bar / grid /
                     preview / filmstrip / info_panel / status_bar
    image/           三档图源 + 解码线程池 + 缓存
    theme/           material.rs（HCT/CAM16 色彩引擎）+ scheme.rs（seed→明暗方案）
                     + mod.rs（gpui 令牌映射与切换，§6）
```

保留 `photo-domain` / `photo-engine` / `photo-recognize` / `photo-config` 四个 crate **原样不动**——
它们全同步、无 UI 依赖，把 Tauri command 层换成直接函数调用即可（§12.3）。

### 12.2 组件映射

> **图标资源（别踩的坑）**：gpui-kit 有两个 `IconName` 和两个资产包，别混用。
> - `gpui_kit::component::IconName`：**兼容用的精选枚举**，只有 101 个（`default-icons.txt`）。
> - `gpui_kit::assets::IconName`：**完整共享目录 1,830 个**，`IconName::ALL` 可枚举。
> - `gpui_kit::assets::Assets`：默认资产包，**只嵌入那 101 个**——此时用共享枚举里的
>   `Clock` 之类会静默加载不到（图标画不出来），不是编译错误，很容易误判成"没有这个图标"。
> - `gpui_kit::assets::AllAssets`：嵌入全部 1,830 个，release 下约 +1 MiB；
>   本项目用这个（`main.rs` 的 `with_assets`）。
> - 只想要几个额外图标就用 `icon_assets!(AppAssets, [Clock, Bird])` 组合默认 `Assets`
>   （10 个额外图标约 +19 KiB），别为此背全量。

| 本项目 | gpui-component |
|---|---|
| 标题栏 | 默认请求系统装饰（`WindowDecorations::Server`）；仅客户端装饰时自绘 `TitleBar::new()`（自带拖拽/双击最大化） |
| 左右活动栏图标按钮 | `Button`（ghost/icon 尺寸）+ `Icon` |
| 左右边栏停靠 | **gpui-kit Dock**（`DockArea` + `DockSkin`）：左/右停靠区 + 中央面板，边缘拖宽 / 折叠显隐 / 宽度写配置；备选 `h_resizable` + `resizable_panel` |
| 顶栏下划线 tab / 药丸 tab | `TabBar` + `Tab` |
| 筛选栏下拉 / 设置下拉 | `Select` / `Combobox`（镜头与鸟种多选可用 `SearchableList`） |
| 筛选摘要 chips / 关键词 chips | `Tag` |
| 滑杆（调整 tab、导出质量） | `Slider` |
| 开关（扫描含子目录等） | `Switch` |
| 数字输入 / 日期输入 | `Input`（日期可用 `DatePicker` 类组件或自绘） |
| 网格 / 胶片条 | `uniform_list`（行高一致 → 网格按行渲染；胶片条按项渲染） |
| 右栏卡片流 | `v_resizable` + `GroupBox` / `panel-card` 等价容器 |
| 状态栏 | `StatusBar`（left/right 两槽 + 中间自绘） |
| 右键菜单 | `ContextMenu` + `PopupMenu`（自带 Esc 处理与视口钳制） |
| 弹窗 | `Dialog` / `AlertDialog`（危险确认用后者）；`Sheet` 可作侧边面板 |
| toast / 状态提示 | `Notification`（`Root::push_notification`，info/success/error） |
| 空态 / 加载骨架 | `Skeleton` + `Spinner` |
| 快捷键参考页 | `Kbd` |
| 主题色选择 | `ColorPicker` |
| 直方图 / 统计图 | `chart` 模块（或自绘 canvas 等价物） |

**图标（坑）**：`IconName` 枚举由全部 Lucide SVG 生成，但 `Assets` 默认只嵌入
`default-icons.txt` 里的 **101 个**——用集外名字（如 `IconName::Film`）**编译通过、运行时**
报 `could not find asset at path "icons/film.svg"` 且什么也不显示。加图标前先在该清单里确认，
或用 `grep -i "icons/<kebab-case>.svg" ` 核对。
项目用 lucide 图标集（`Copy`/`Layers`/`Crown`/`PanelLeft`/`Square` 等），
gpui-component 的 `Icon` 走 `IconName`/SVG 资源——把用到的图标按名搬过去即可。

### 12.3 后端接入（无 IPC）

Tauri 层做的事，在 GPUI 里各有对应：

| Tauri | GPUI 等价 |
|---|---|
| `#[tauri::command]` + specta 绑定 | 直接调用 `photo_engine::*`（全同步） |
| `spawn_blocking` 包裹 | `cx.background_spawn(async move { ... })` 或 `cx.background_executor()` |
| `app.emit("scan:progress", …)` | `cx.update(...)` / `weak_entity.update(cx, …)` 回主线程改状态 |
| `Mutex<AppState>` | `Entity<AppState>`（GPUI 内部已是单线程模型，用实体而非锁） |
| `ptimg://` 三档协议 | 后台解码 → `Image::from_bytes(...)` → `img(...)`；缓存自己管（§12.5） |
| 事件监听 + 哨兵位 | 订阅实体变更（`cx.observe` / `cx.subscribe`），任务状态用 `Task` 句柄持有 |

**注意**：引擎是同步阻塞的，**绝不能**在渲染/主线程里直接调用扫描或识别；
一律丢到后台执行器，完成后回主线程更新实体。

### 12.4 键位与焦点

- 用 GPUI 的 Action + KeyBinding 体系：为 §5.6 的每个动作定义 `Action`（`actions!` 宏或带参数的结构体，
  如 `Rate(u8)`、`SetColorLabel(ColorLabel)`），在 `keymap` 里绑定键位。
- **焦点上下文**：给网格/预览/输入框设不同 `key_context`，把全局键位绑在窗口级 context 上；
  输入框获得焦点时切到文本 context，全局动作自然不触发（对应 §5.5 规则 1）。
- `Esc` 优先级链（§5.4）用一个中心化函数实现，键位只负责把 `Esc` 路由到它。
- `Ctrl+Shift+B` 三态用带参 Action 或三个独立 Action，别用"先看修饰键再分派"的运行时判断。

### 12.5 图片管线（GPUI 特有注意）

- `Image::from_bytes(ImageFormat, Vec<u8>)` 接受内存字节，**解码在后台做**，
  主线程只拿到已解码的图像句柄。三档缓存（thumb / master / full）自己实现，
  键用 `(路径, 文件大小, 档位)`。
- 缩略图网格：`uniform_list` 按**行**渲染（每行 `COLS` 个 cell），
  行高固定 = §4.8 的 `rowH`，可见行由 GPUI 自己裁剪，无需手写滚动计算。
- 预览的缩放/平移用 §4.6 的公式算出显示矩形，直接作用到 `img()` 的尺寸与偏移。
- 叠加层（检测框 / 眼框臂条 / 对焦点）用绝对定位的 `div()` 叠在图片上，
  坐标乘同一个缩放系数即可。

### 12.6 主题

- 主题**不再走主题 json**：`theme/scheme.rs` 由 seed 生成亮/暗两套色值，
  `theme/mod.rs` 填成 gpui-component 的 `ThemeConfig`（`background`、`primary.background`、
  `list.*`、`tab.*` 等字段），赋给 `Theme::global_mut(cx).light_theme / .dark_theme` 后
  调 `Theme::change(mode, window, cx)`；改主题色只需重建这两个 config。
- 未显式填的字段会回落到 gpui-component 内置的 `ThemeColor::light()/dark()`——
  因此**映射表里有的令牌都要填全**，否则会混进默认蓝紫。
- accent 生成见 §6.2：seed → HCT → neutral(chroma 0) + accent(chroma 0 或 24–48)，
  只重写 accent 一族（§6.4）。
- 领域色（label-* / pick / reject / rating / focus）gpui-component 没有对应槽位，
  定义在 `theme::mod` 的 `Rgba` 常量里，用自绘 `div().bg(...)`，
  **不要散落到视图代码里写裸 hex**。

### 12.7 落地顺序建议

1. `photo-ui` crate 起来 + `Root` + 标题栏 + 五列空壳 + 主题切换（能看见框架）。
2. `model/` 移植 §4 全部纯函数 + 单测（**零 UI 依赖，可先做完**）。
3. 扫描 → `uniform_list` 网格 → 缩略图三档管线（第一张照片出现在屏幕上）。
4. 选择 / 标记 / 键位 / 视图切换（culling 主循环可用）。
5. 右栏信息与调整、筛选栏、状态栏、右键菜单。
6. 批量操作 / 识别 / 导入 / 导出 / 统计 / 地图 / 幻灯片。
7. 主题 seed、动效、无障碍、打包。

---

### 12.8 日志（tracing 统一管道）

- **安装点**：`photo-ui` 的 `main()` 第一行 `logging::init()`，返回的 `WorkerGuard` 必须由 `main`
  持有到进程退出（否则非阻塞 writer 的缓冲日志会丢）。规格与 Tauri 版一致：滚动日切
  `<配置目录>/logs/ftpt.YYYY-MM-DD.log`（即 `~/.config/pt/logs/`），保留 14 份，stderr 同步输出，
  `tracing_log::LogTracer` 桥接第三方 `log::*`。
- **级别**：debug 构建默认 `debug`、release 默认 `info`，`PHOTO_LOG_LEVEL` 覆盖。
- **必须压制第三方 DEBUG 噪声**：GPUI 直接跑在 wgpu 上，`wgpu_hal` / `naga` / `cosmic_text` 在 DEBUG 级是刷屏级的——
  实测 14 秒 **88,348 行 / 5.6 MB**，加目标压制后 **67 行 / 3.6 KB**。Tauri 版没这个问题（WebView 挡在外面），
  把它的过滤器原样搬过来会踩坑。要查渲染问题时按目标显式打开：`PHOTO_LOG_LEVEL=debug,wgpu_hal=debug`。
- 设置「关于」页展示日志目录，并提供「打开日志目录 / 打开今日日志」两个入口。
- 启动第一行日志固定输出「版本 · 日志目录 · 数据根」——排查问题的第一个锚点。

## 13. 已知缺口与维护约定

### 设计层缺口（移植时可选修复）

1. 批量操作与重命名的对象边界依赖"必须存在激活筛选"，但用户无筛选时只能先造一个条件
   （缺"显式确认全量"的逃生门）。
2. 删除到回收站**无确认、无撤销**（`Ctrl+Z` 只覆盖移动/复制/重命名）——与批量删除的二次确认不一致。
3. 左栏两个多选下拉（镜头/鸟种）**无方向键导航**。
4. 右键菜单无键盘导航与最大高度滚动；网格无表格语义（虚拟列表未做行列语义）。
5. 框选识别只画框，尚未接识别动作。

### 实现层缺口

6. 面板宽度双轨（本地存储 vs 配置字段），不会跨设备同步。
7. 导出/导入目标目录互不记忆。
8. 技术分/重复检测的阈值与结果都不落盘，重启后需重算。

### 维护约定（改这份手册的规则）

- 新增键位：更新 §5.6 表 + 快捷键页分组说明，并确保鼠标路径调用同一动作。
- 新增视图/面板：先扩 §3.3 的视图枚举与条件链，再明确它在 §5.4 `Esc` 链中的位置。
- 新增视觉变量：先判断它属于中性面、accent 一族还是领域情景色，进 §6 对应表；
  只有领域情景色可以进 `theme::mod` 的常量，**不允许在视图代码里写裸色值**。
- 新增批量/长任务：套用 §8.1 的状态机与 §8.2 的事件通道，并补生成号/取消令牌。

---

## 附录 A：后端能力清单（53 个 command，按域分组）

移植时按此清单对齐功能，逐个找 GPUI 侧的直接函数调用。

| 域 | 命令 |
|---|---|
| 目录与扫描 | `pick_directory` `scan_directory` `get_captures` `list_subdirs` `list_favorites` `add_favorite` `remove_favorite` `list_recent` `remove_recent` |
| 标记 | `set_rating` `set_flag` `set_color_label` `set_keywords` |
| 文件操作 | `delete_captures` `batch_op_preview` `batch_op_execute` `undo_batch_operation` `batch_rename` `export_captures` `export_adjusted` |
| 识别 | `recognize_captures` `cancel_recognition` `get_recognition` `search_catalog` `correct_recognition` `list_bird_species` |
| 图像数据 | `get_histogram` `get_clipping_mask` `get_adjustments` `set_adjustments` |
| 配置与系统 | `get_app_config` `set_app_config` `open_config_file` `list_system_fonts` `copy_image_to_clipboard` `copy_text_to_clipboard` `log` `get_log_file_path` `open_log_file` `open_log_directory` |
| 统计（全局索引） | `get_species_stats` `get_species_photos` `get_correction_stats` `get_frequent_species` `export_bird_records` |
| 导入 | `list_import_drives` `take_pending_import_path` `scan_import_source` `plan_import` `execute_import` |
| 重复与技术分 | `find_duplicates` `compute_quality_scores` `get_quality_scores` |

## 附录 B：纯函数自检清单（移植后必须有的最小测试）

每条都应是**独立可跑的断言**，不依赖 UI：

```
filterCaptures:
  · 设了日期区间 + dateTaken=null          → 排除
  · dateTaken 有值但解析失败                → 保留
  · formatFilter = RAW 通配                → 匹配 NEF，不匹配 JPEG
  · minRating = Three                      → None/1/2 星排除
  · recognitionFilter = NotRecognized      → 只留 recognitionStatus == null
hasActiveFilters: 全默认 → false；任一字段非默认 → true

compareCaptures:
  · EyeSharpness: null 排最前
  · Quality:      null 排最后
  · 稳定性：同键保持原序

groupStacks:
  · 跨目录同 stem → 分两组
  · 同 stem 被打散 → 聚合到最早位置
groupByTime:
  · 相邻差 = 2000ms → 同组；2001ms → 不同组
  · 无时间成员 → 独立组且切断组链
pickPrimary: 组内有 jpg + nef → 选 jpg；纯 RAW → 组内首个

computeBurstGroups:
  · size < 2 不登记
  · 时间字段越界（13 月）→ 视为 null
pickBestFrame: 三键全并列 → 路径字典序决定，且重复调用结果一致

clampPanAxis: disp <= container → 0
fitScale: 小图 → 1.0（不放大）
panAfterCursorZoom: 缩放后光标下的图像点坐标不变（浮点容差内）
renderNameTemplate: 未知占位符原样保留；非法字符被清洗；空结果回退原名

网格布局: 末行不满 COLS 时 cell 数正确、spacer 高度正确
```

---

## 附：快速核对（当前 GPUI 实现）

```bash
cargo test -p photo-ui       # model 纯逻辑（筛选/排序/堆叠/连拍/预览数学）+ 主题色彩单测
cargo run -p photo-ui        # 真机逐项目检（网格/预览/幻灯片/统计/弹窗/快捷键）
XDG_CONFIG_HOME=/tmp/ptlease-config xvfb-run -a cargo run -p photo-ui --example lease_smoke  # 无头冒烟
```

目检重点：窗口拖拽与八向缩放、`Esc` 优先级链、`markPaths` 在幻灯片下的作用域、
堆叠成员带与 `Q/E`、扫描切目录后旧任务不串数据、批量操作的干跑-确认-撤销闭环。
