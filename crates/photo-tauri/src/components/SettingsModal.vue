<script setup lang="ts">
// 设置弹窗：对齐 GPUI toolbar.rs SettingsOverlay（遮罩/Esc/× 关闭、卡片式三页 tab：
// 通用 / 快捷键 / 关于）。所有改动即保存（setAppConfig → 后端 save_config），
// 主题/字体即时应用 DOM（html.dark class / --font-family-app 变量）。
// 打开时重新 getAppConfig 初始化当前值（对齐 GPUI「设置保存即时刷新」语义）。
import { computed, onMounted, ref, watch } from 'vue'
import { getVersion, getTauriVersion } from '@tauri-apps/api/app'
import { version as vueVersion } from 'vue'
import { BookOpenIcon, FileTextIcon, InfoIcon, MoonIcon, SettingsIcon, SunIcon, XIcon } from '@lucide/vue'
import { Dialog, DialogClose, DialogContent, DialogTitle } from '@/components/ui/dialog'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { Button } from '@/components/ui/button'
import { useConfigStore } from '@/stores/config'
import { useCapturesStore } from '@/stores/captures'
import { listSystemFonts, openConfigFile as openConfigFileIpc } from '@/lib/ipc'
import { BINDINGS, type KeyBinding, type KeymapAction } from '@/keymap'
import type { DetectionSource, StackMode, Theme } from '@/lib/bindings'
import { DEFAULT_ACCENT } from '@/lib/m3Theme'

/** 弹窗开关（App.vue v-model 控制：顶栏齿轮按钮打开、keymap Esc 分支 / 弹窗自身 × 关闭） */
const open = defineModel<boolean>('open', { default: false })

const config = useConfigStore()

// ── tab 结构（对齐 GPUI Settings 三页：通用 / 快捷键 / 关于） ─────────────
const TABS = [
  { id: 'general', label: '通用', icon: SettingsIcon },
  { id: 'shortcuts', label: '快捷键', icon: BookOpenIcon },
  { id: 'about', label: '关于', icon: InfoIcon },
] as const
type TabId = (typeof TABS)[number]['id']
const activeTab = ref<TabId>('general')

// 打开时初始化 getAppConfig 当前值（后端可能已被其他入口改动，如主题按钮）
watch(open, (v) => {
  if (v) {
    activeTab.value = 'general'
    void config.load()
  }
})

// ── 通用页：主题 / 界面字体 / 识别线程数 / 缩略图尺寸（全部改动即保存） ──

const THEMES: { value: Theme; label: string; icon: typeof SunIcon }[] = [
  { value: 'Light', label: '亮色', icon: SunIcon },
  { value: 'Dark', label: '暗色', icon: MoonIcon },
]
/** 主题：即时切换 html.dark（GPUI 语义同 activity_rail 主题按钮） */
const theme = computed<Theme>({
  get: () => config.theme,
  set: (v: Theme) => void config.update({ theme: v }),
})

/** Material You 预设 seed 色板（8 个色块） */
const ACCENT_PRESETS = ['#3b82f6', '#6750a4', '#00696e', '#386a20', '#b3261e', '#8d4f2f', '#a6374c', '#605b63']
/** 主题 seed 色：预设/取色器改动即保存 + 整套配色即时重染；null = 默认蓝 */
const accentColor = computed<string | null>({
  get: () => config.accentColor,
  set: (v: string | null) => void config.update({ accentColor: v }),
})
/** 当前生效 seed（null 回退默认蓝），取色器 value 与选中态判定用 */
const effectiveAccent = computed(() => config.accentColor ?? DEFAULT_ACCENT)

/** 系统字体列表（listSystemFonts 一次性拉取，真实后端为全系统字体） */
const fonts = ref<string[]>([])
/** 界面字体：改动即保存 + 即时应用 --font-family-app（对齐 GPUI 字体下拉） */
const fontFamily = computed({
  get: () => config.fontFamily,
  set: (v: string) => void config.update({ fontFamily: v }),
})

/** 识别线程数 1–4（对齐 GPUI NumberFieldOptions min 1 / max 4） */
const threadCount = computed({
  get: () => config.recognitionThreadCount,
  set: (v: number) => void config.update({ recognitionThreadCount: v }),
})

/**
 * 识别鸟体定位来源（photo-config DetectionSource）：Yolo = 全图 YOLO 检测（默认，
 * 多鸟场景不漏）；Focus = 优先相机对焦点 ROI 直接分类（相机对焦位置先验可靠），
 * 无对焦点的照片自动回退 YOLO。改动即保存，下次批量识别生效。
 */
const DETECTION_SOURCES: { value: DetectionSource; label: string; desc: string }[] = [
  { value: 'Yolo', label: 'YOLO 检测', desc: '全图 YOLO 检测鸟体（默认，多鸟不漏）' },
  { value: 'Focus', label: '相机对焦点', desc: '优先用相机对焦点 ROI 分类，无对焦点回退 YOLO' },
]
const detectionSource = computed({
  get: () => config.detectionSource,
  set: (v: DetectionSource) => void config.update({ detectionSource: v }),
})

/** 网格每行图片数 2-5（下拉选项；固定列数后 cell 宽由容器自适应，即时重排） */
const gridColumns = computed({
  get: () => config.gridColumns,
  set: (v: number) => void config.update({ gridColumns: v }),
})

/** 界面缩放比例（90/100/110/120%，即时应用 DOM） */
const uiScale = computed({
  get: () => config.uiScale,
  set: (v: number) => void config.update({ uiScale: v }),
})

/** 网格堆叠模式选项（对齐 photo-config StackMode 三态；改动即保存，网格即时重排） */
const STACK_MODES: { value: StackMode; label: string; desc: string }[] = [
  { value: 'None', label: '不堆叠', desc: '每个文件独立显示' },
  { value: 'ByFileName', label: '同文件名', desc: 'JPG/NEF 等同画面合并（同 stem）' },
  { value: 'ByTime', label: '同组照片', desc: '连拍照片按拍摄时间合并（≤2 秒）' },
]
const stackMode = computed({
  get: () => config.stackMode,
  set: (v: StackMode) => void config.update({ stackMode: v }),
})

/** 扫描包含子目录开关：改动即保存（setAppConfig → save_config）；有打开目录时立即重扫按新设置生效 */
const includeSubdirectories = computed(() => config.includeSubdirectories)
async function toggleIncludeSubdirectories() {
  await config.update({ includeSubdirectories: !config.includeSubdirectories })
  // 自动重扫当前目录（扫描编排按配置选单层/递归；无目录时等下次打开生效）
  const captures = useCapturesStore()
  if (captures.directory) void captures.rescan()
}

onMounted(async () => {
  try {
    fonts.value = await listSystemFonts()
  } catch {
    // mock/后端未就绪：字体下拉留空，不阻塞弹窗
  }
})

// ── 快捷键页：列出 keymap.ts BINDINGS 全部键位（中文明细，分组对齐 GPUI shortcuts_page） ──

/** action → 中文说明（对齐 GPUI shortcuts_page 文案） */
const ACTION_DESC: Record<KeymapAction, string> = {
  rate1: '评分 1 星',
  rate2: '评分 2 星',
  rate3: '评分 3 星',
  rate4: '评分 4 星',
  rate5: '评分 5 星',
  rate0: '清除评分',
  labelRed: '红色标签',
  labelYellow: '黄色标签',
  labelGreen: '绿色标签',
  labelBlue: '蓝色标签',
  labelPurple: '紫色标签',
  flagPick: '标记为入选',
  flagReject: '标记为淘汰',
  flagNone: '清除旗标',
  recognize: '识别当前图片',
  recognizeUnrecognized: '识别未识别的',
  recognizeAll: '重新识别全部',
  toggleBbox: '切换检测框',
  toggleFocus: '切换对焦点叠加（预览）',
  toggleClipping: '切换剪切警告叠加（预览）',
  toggleGridPreview: '切换网格/预览',
  zoomIn: '放大（预览/对比）',
  zoomOut: '缩小（预览/对比）',
  slideshow: '幻灯片模式',
  slideshowTogglePlay: '幻灯片：暂停/继续',
  compare: '对比模式（多选 2–4 张 / 连拍组前 4 张）',
  stats: '统计视图（全局鸟种索引）',
  prev: '上一张',
  next: '下一张',
  first: '第一张',
  last: '最后一张',
  stackPrev: '堆叠内上一个成员（网格）',
  stackNext: '堆叠内下一个成员（网格）',
  delete: '删除到回收站',
  undoBatch: '撤销批量操作（移动/复制/重命名）',
  selectAll: '全选',
  deselectAll: '取消全选',
  closePreview: '取消/关闭',
  refresh: '刷新目录',
  toggleLeftPanel: '切换左侧面板',
  toggleRightPanel: '切换右侧面板',
}

/** 方向键/特殊键的展示名（对齐 GPUI 快捷键页的 ← → Home End 写法） */
const KEY_LABELS: Record<string, string> = {
  left: '←',
  right: '→',
  home: 'Home',
  end: 'End',
  escape: 'Esc',
  f5: 'F5',
  delete: 'Delete',
  ' ': '空格',
}

/** 把绑定行渲染为可读键位串（修饰键顺序 Ctrl → Shift → 键名，对齐 GPUI） */
function formatKeys(b: KeyBinding): string {
  const mods: string[] = []
  if (b.ctrl) mods.push('Ctrl')
  if (b.shift) mods.push('Shift')
  const key = KEY_LABELS[b.key] ?? b.key.toUpperCase()
  return [...mods, key].join('+')
}

/** 分组表（对齐 GPUI shortcuts_page 的五组：常用操作/标记/识别/选择/面板） */
const SHORTCUT_SECTIONS: { title: string; actions: KeymapAction[] }[] = [
  {
    title: '常用操作',
    actions: [
      'prev', 'next', 'first', 'last', 'toggleGridPreview', 'delete', 'undoBatch', 'refresh',
      'zoomIn', 'zoomOut', 'slideshow', 'slideshowTogglePlay', 'toggleClipping',
      'stackPrev', 'stackNext',
    ],
  },
  {
    title: '标记',
    actions: [
      'rate1', 'rate2', 'rate3', 'rate4', 'rate5', 'rate0',
      'labelRed', 'labelYellow', 'labelGreen', 'labelBlue', 'labelPurple',
      'flagPick', 'flagReject', 'flagNone',
    ],
  },
  {
    title: '识别',
    actions: ['recognize', 'recognizeUnrecognized', 'recognizeAll', 'toggleBbox'],
  },
  {
    title: '选择',
    actions: ['selectAll', 'deselectAll'],
  },
  {
    title: '面板',
    actions: ['closePreview', 'toggleLeftPanel', 'toggleRightPanel'],
  },
]

/** 某 action 的全部绑定行（同 action 可能多条，如 b 键三态） */
function shortcutRows(action: KeymapAction): string[] {
  return BINDINGS.filter((b) => b.action === action).map(formatKeys)
}

// ── 关于页：应用名 / 版本 / 技术栈 / 便携布局说明（对齐 GPUI about_page + 便携说明） ──

const appVersion = ref('0.1.0')
const tauriVersion = ref('2')
onMounted(async () => {
  try {
    appVersion.value = await getVersion()
  } catch {
    // 浏览器 mock 模式无 Tauri 环境，回退 0.1.0
  }
  try {
    tauriVersion.value = await getTauriVersion()
  } catch {
    // 同上，回退 2
  }
})

const aboutRows = computed(
  () =>
    [
      ['应用名', 'ftpt'],
      ['版本', appVersion.value],
      ['界面框架', `Tauri v${tauriVersion.value}`],
      ['前端框架', `Vue ${vueVersion}`],
      ['组件库', 'shadcn-vue（reka-ui）'],
      ['样式', 'Tailwind CSS v4'],
      ['识别引擎', 'ONNX Runtime (DirectML)'],
      ['检测模型', 'YOLOv8n 0.5'],
      ['分类模型', 'bird_model'],
      ['名录库', 'pica_ref.db'],
    ] as [string, string][],
)

/** 「打开配置文件」失败提示（成功无感；失败展示错误原因，下次点击重置） */
const configOpenError = ref<string | null>(null)
async function openConfigFile() {
  configOpenError.value = null
  try {
    await openConfigFileIpc()
  } catch (e) {
    configOpenError.value = String(e)
  }
}
</script>

<template>
  <Dialog :open="open" @update:open="open = $event">
    <!-- 自绘头栏（× 按钮放标题右侧，对齐 GPUI settings-card 头栏），故关闭默认 × 按钮 -->
    <DialogContent
      :show-close-button="false"
      class="flex h-[40rem] max-w-3xl flex-col gap-0 p-0 sm:max-w-[53.75rem]"
    >
      <!-- 头栏：标题 + 关闭按钮 -->
      <div class="flex shrink-0 items-center justify-between border-b px-4 py-3">
        <DialogTitle class="text-base font-semibold">设置</DialogTitle>
        <DialogClose as-child>
          <Button variant="ghost" size="icon-sm" aria-label="关闭">
            <XIcon />
          </Button>
        </DialogClose>
      </div>

      <!-- 主体：左 tab 导航 + 右内容（对齐 GPUI Settings 页布局；标准 Tabs 组件） -->
      <Tabs v-model="activeTab" class="flex min-h-0 flex-1">
        <TabsList class="flex w-44 shrink-0 flex-col items-stretch justify-start gap-1 rounded-none border-r bg-transparent p-2">
          <TabsTrigger
            v-for="t in TABS"
            :key="t.id"
            :value="t.id"
            class="justify-start gap-2 rounded-lg px-2 py-1.5 data-[state=active]:bg-secondary-container data-[state=active]:text-on-secondary-container data-[state=active]:shadow-none"
          >
            <component :is="t.icon" class="h-4 w-4" />
            {{ t.label }}
          </TabsTrigger>
        </TabsList>

        <div class="min-w-0 flex-1 overflow-y-auto p-5">
          <!-- ── 通用 ── -->
          <TabsContent value="general" class="mt-0 space-y-6">
            <!-- 外观：主题 / 界面字体 / 界面缩放 -->
            <section class="space-y-4">
              <h3 class="section-header">外观</h3>

              <!-- 主题：亮/暗分段，带图标，即时切换 -->
              <div class="settings-row">
                <div class="settings-row-label">
                  <label class="text-sm font-medium">主题</label>
                  <p class="mt-0.5 text-xs text-muted-foreground">即时切换界面配色</p>
                </div>
                <div class="flex shrink-0 items-center gap-0.5 rounded-lg bg-muted p-1">
                  <button
                    v-for="t in THEMES"
                    :key="t.value"
                    type="button"
                    class="flex items-center gap-1.5 rounded-md px-3 py-1.5 text-sm transition-colors"
                    :class="
                      theme === t.value
                        ? 'bg-card text-foreground shadow-sm'
                        : 'text-muted-foreground hover:text-foreground'
                    "
                    @click="theme = t.value"
                  >
                    <component :is="t.icon" class="size-4" />
                    {{ t.label }}
                  </button>
                </div>
              </div>

              <!-- 主题色：Material You seed 色，预设/取色器即时重染整套配色 -->
              <div class="settings-row">
                <div class="settings-row-label">
                  <label class="text-sm font-medium">主题色</label>
                  <p class="mt-0.5 text-xs text-muted-foreground">Material You seed 色，整套配色即时重染</p>
                </div>
                <div class="flex shrink-0 items-center gap-1.5">
                  <button
                    v-for="s in ACCENT_PRESETS"
                    :key="s"
                    type="button"
                    :style="{ backgroundColor: s }"
                    class="size-6 rounded-full transition ring-2 ring-offset-2"
                    :class="effectiveAccent === s ? 'ring-foreground' : 'ring-transparent hover:ring-outline'"
                    :aria-label="s"
                    @click="accentColor = s"
                  />
                  <input
                    type="color"
                    :value="effectiveAccent"
                    class="size-6 cursor-pointer appearance-none rounded-full border-0 bg-transparent p-0"
                    @input="accentColor = ($event.target as HTMLInputElement).value"
                  />
                  <Button variant="ghost" size="xs" @click="accentColor = null">默认</Button>
                </div>
              </div>

              <!-- 界面字体 -->
              <div class="settings-row">
                <div class="settings-row-label">
                  <label for="settings-font" class="text-sm font-medium">界面字体</label>
                  <p class="mt-0.5 text-xs text-muted-foreground">应用界面字体，即时生效</p>
                </div>
                <select
                  id="settings-font"
                  v-model="fontFamily"
                  class="settings-select w-52 shrink-0"
                >
                  <option v-for="f in fonts" :key="f" :value="f">{{ f }}</option>
                </select>
              </div>

              <!-- 界面缩放 -->
              <div class="settings-row">
                <div class="settings-row-label">
                  <label for="settings-ui-scale" class="text-sm font-medium">界面缩放</label>
                  <p class="mt-0.5 text-xs text-muted-foreground">
                    整体界面等比缩放（100% = 基准 15px）
                  </p>
                </div>
                <select
                  id="settings-ui-scale"
                  v-model.number="uiScale"
                  class="settings-select w-24 shrink-0"
                >
                  <option v-for="n in [75, 100, 125, 150, 175, 200]" :key="n" :value="n">
                    {{ n }}%
                  </option>
                </select>
              </div>
            </section>

            <!-- 网格：每行图片数 / 堆叠模式 -->
            <section class="space-y-4">
              <h3 class="section-header">网格</h3>

              <div class="settings-row">
                <div class="settings-row-label">
                  <label for="settings-grid-cols" class="text-sm font-medium">每行图片数</label>
                  <p class="mt-0.5 text-xs text-muted-foreground">
                    固定列数，缩略图随容器宽度自适应
                  </p>
                </div>
                <select
                  id="settings-grid-cols"
                  v-model.number="gridColumns"
                  class="settings-select w-24 shrink-0"
                >
                  <option v-for="n in [2, 3, 4, 5]" :key="n" :value="n">{{ n }} 张</option>
                </select>
              </div>

              <div class="settings-row">
                <div class="settings-row-label">
                  <label for="settings-stack" class="text-sm font-medium">堆叠模式</label>
                  <p class="mt-0.5 text-xs text-muted-foreground">
                    {{ STACK_MODES.find((m) => m.value === stackMode)?.desc }}
                  </p>
                </div>
                <select
                  id="settings-stack"
                  v-model="stackMode"
                  class="settings-select w-32 shrink-0"
                >
                  <option v-for="m in STACK_MODES" :key="m.value" :value="m.value">
                    {{ m.label }}
                  </option>
                </select>
              </div>
            </section>

            <!-- 识别 -->
            <section class="space-y-4">
              <h3 class="section-header">识别</h3>

              <div class="settings-row">
                <div class="settings-row-label">
                  <label for="settings-threads" class="text-sm font-medium">识别线程数</label>
                  <p class="mt-0.5 text-xs text-muted-foreground">
                    批量识别并发线程（1–4），越多内存占用越高
                  </p>
                </div>
                <select
                  id="settings-threads"
                  v-model.number="threadCount"
                  class="settings-select w-20 shrink-0"
                >
                  <option v-for="n in 4" :key="n" :value="n">{{ n }}</option>
                </select>
              </div>

              <div class="settings-row">
                <div class="settings-row-label">
                  <label for="settings-detection" class="text-sm font-medium">鸟体定位来源</label>
                  <p class="mt-0.5 text-xs text-muted-foreground">
                    {{ DETECTION_SOURCES.find((m) => m.value === detectionSource)?.desc }}
                  </p>
                </div>
                <select
                  id="settings-detection"
                  v-model="detectionSource"
                  class="settings-select w-32 shrink-0"
                >
                  <option v-for="m in DETECTION_SOURCES" :key="m.value" :value="m.value">
                    {{ m.label }}
                  </option>
                </select>
              </div>
            </section>

            <!-- 扫描 -->
            <section class="space-y-4">
              <h3 class="section-header">扫描</h3>

              <div class="settings-row">
                <div class="settings-row-label">
                  <label class="text-sm font-medium">扫描包含子目录</label>
                  <p class="mt-0.5 text-xs text-muted-foreground">
                    递归扫描全部子目录；改动即保存并自动重扫当前目录
                  </p>
                </div>
                <button
                  type="button"
                  role="switch"
                  :aria-checked="includeSubdirectories"
                  aria-label="扫描包含子目录"
                  class="relative h-8 w-[3.25rem] shrink-0 rounded-full border-2 transition-colors focus-visible:ring-2 focus-visible:ring-ring"
                  :class="includeSubdirectories ? 'border-transparent bg-primary' : 'border-outline bg-surface-container-highest'"
                  @click="toggleIncludeSubdirectories"
                >
                  <span
                    class="absolute top-1 left-1 size-6 rounded-full shadow transition-transform"
                    :class="includeSubdirectories ? 'translate-x-6 bg-on-primary' : 'bg-outline'"
                  />
                </button>
              </div>
            </section>

            <!-- 配置文件：打开按钮（用户可见入口，对齐 open_config_file 语义） -->
            <section class="space-y-4">
              <h3 class="section-header">配置文件</h3>
              <div class="settings-row">
                <div class="settings-row-label">
                  <label class="text-sm font-medium">打开配置文件</label>
                  <p class="mt-0.5 text-xs text-muted-foreground">
                    用系统默认文本编辑器查看 config.toml（未保存过设置时自动生成）
                  </p>
                </div>
                <button
                  type="button"
                  class="inline-flex shrink-0 items-center gap-1 text-sm font-medium text-primary underline-offset-4 hover:underline"
                  @click="openConfigFile"
                >
                  <FileTextIcon class="size-4" />
                  打开配置文件
                </button>
              </div>
              <p v-if="configOpenError" class="text-xs text-destructive">打开失败：{{ configOpenError }}</p>
            </section>
          </TabsContent>

          <!-- ── 快捷键 ── -->
          <TabsContent value="shortcuts" class="mt-0 space-y-6">
            <div v-for="sec in SHORTCUT_SECTIONS" :key="sec.title" class="space-y-2">
              <h3 class="section-header">{{ sec.title }}</h3>
              <div class="divide-y divide-border rounded-md border border-border bg-card/50">
                <div
                  v-for="action in sec.actions"
                  :key="action"
                  class="flex items-center justify-between px-3 py-1.5 text-sm"
                >
                  <span class="text-muted-foreground">{{ ACTION_DESC[action] }}</span>
                  <kbd class="rounded-md border border-border bg-muted px-2 py-0.5 text-xs text-foreground tabular-nums">
                    {{ shortcutRows(action).join(' / ') }}
                  </kbd>
                </div>
              </div>
            </div>
          </TabsContent>

          <!-- ── 关于 ── -->
          <TabsContent value="about" class="mt-0 space-y-5">
            <div class="space-y-1.5">
              <h3 class="text-base font-semibold">Photo Tool（ftpt）</h3>
              <p class="text-sm text-muted-foreground">
                照片管理与筛选工具（鸟类摄影工作流）
              </p>
            </div>
            <div class="divide-y divide-border rounded-md border border-border bg-card/50">
              <div
                v-for="[label, value] in aboutRows"
                :key="label"
                class="flex items-center justify-between px-3 py-2 text-sm"
              >
                <span class="text-muted-foreground">{{ label }}</span>
                <span class="text-xs tabular-nums">{{ value }}</span>
              </div>
            </div>
            <!-- 配置位置说明（对齐 photo-config determine_config_path 语义） -->
            <div class="space-y-1.5 rounded-md border border-border bg-card/50 p-3 text-xs text-muted-foreground">
              <p class="text-sm font-medium text-foreground">配置与缓存位置</p>
              <p>配置存于用户主目录统一位置（~/.config/pt/config.toml，Windows 为 %USERPROFILE%\.config\pt\config.toml）。</p>
              <p>缩略图缓存随扫描目录存放（每个目录下 .pt/thumbs）。</p>
            </div>
          </TabsContent>
        </div>
      </Tabs>
    </DialogContent>
  </Dialog>
</template>
