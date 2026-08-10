<script setup lang="ts">
// 设置弹窗：对齐 GPUI toolbar.rs SettingsOverlay（遮罩/Esc/× 关闭、卡片式三页 tab：
// 通用 / 快捷键 / 关于）。所有改动即保存（setAppConfig → 后端 save_config），
// 主题/字体即时应用 DOM（html.dark class / --font-family-app 变量）。
// 打开时重新 getAppConfig 初始化当前值（对齐 GPUI「设置保存即时刷新」语义）。
import { computed, onMounted, ref, watch } from 'vue'
import { getVersion, getTauriVersion } from '@tauri-apps/api/app'
import { version as vueVersion } from 'vue'
import { BookOpenIcon, InfoIcon, SettingsIcon, XIcon } from '@lucide/vue'
import { Dialog, DialogClose, DialogContent, DialogTitle } from '@/components/ui/dialog'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { Button } from '@/components/ui/button'
import { useConfigStore } from '@/stores/config'
import { listSystemFonts } from '@/lib/ipc'
import { BINDINGS, type KeyBinding, type KeymapAction } from '@/keymap'
import type { Theme } from '@/lib/bindings'

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

const THEMES: { value: Theme; label: string }[] = [
  { value: 'Light', label: '亮色' },
  { value: 'Dark', label: '暗色' },
]
/** 主题：即时切换 html.dark（GPUI 语义同 activity_rail 主题按钮） */
const theme = computed<Theme>({
  get: () => config.theme,
  set: (v: Theme) => void config.update({ theme: v }),
})

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

/** 缩略图尺寸（px）：即时调整网格 cell（对齐 GPUI grid.rs cell_size = thumbnailSize + 56），缩略图缓存按需重新生成 */
const THUMB_RANGE = { min: 160, max: 320, step: 20 } as const
const thumbnailSize = computed({
  get: () => config.thumbnailSize,
  set: (v: number) => void config.update({ thumbnailSize: v }),
})

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
  flagPick: '标记为入选',
  flagReject: '标记为淘汰',
  flagNone: '清除旗标',
  recognize: '识别当前图片',
  recognizeUnrecognized: '识别未识别的',
  recognizeAll: '重新识别全部',
  toggleBbox: '切换检测框',
  toggleGridPreview: '切换网格/预览',
  prev: '上一张',
  next: '下一张',
  first: '第一张',
  last: '最后一张',
  delete: '删除到回收站',
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
    actions: ['prev', 'next', 'first', 'last', 'toggleGridPreview', 'delete', 'refresh'],
  },
  {
    title: '标记',
    actions: [
      'rate1', 'rate2', 'rate3', 'rate4', 'rate5', 'rate0',
      'labelRed', 'labelYellow', 'labelGreen', 'labelBlue',
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
</script>

<template>
  <Dialog :open="open" @update:open="open = $event">
    <!-- 自绘头栏（× 按钮放标题右侧，对齐 GPUI settings-card 头栏），故关闭默认 × 按钮 -->
    <DialogContent
      :show-close-button="false"
      class="flex h-[640px] max-w-3xl flex-col gap-0 p-0 sm:max-w-[860px]"
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
        <TabsList class="flex w-44 shrink-0 flex-col items-stretch gap-1 rounded-none border-r bg-transparent p-2">
          <TabsTrigger
            v-for="t in TABS"
            :key="t.id"
            :value="t.id"
            class="justify-start gap-2 rounded-md px-2 py-1.5 data-[state=active]:bg-accent data-[state=active]:text-accent-foreground data-[state=active]:shadow-none"
          >
            <component :is="t.icon" class="h-4 w-4" />
            {{ t.label }}
          </TabsTrigger>
        </TabsList>

        <div class="min-w-0 flex-1 overflow-y-auto p-4">
          <!-- ── 通用 ── -->
          <TabsContent value="general" class="mt-0 space-y-6">
            <!-- 主题：亮/暗，即时切换（html.dark） -->
            <div class="space-y-1.5">
              <label class="text-sm font-medium">主题</label>
              <div class="flex w-fit items-center rounded-md bg-muted p-0.5">
                <button
                  v-for="t in THEMES"
                  :key="t.value"
                  type="button"
                  class="rounded-sm px-3 py-1 text-sm transition-colors"
                  :class="
                    theme === t.value
                      ? 'bg-card text-foreground shadow-sm'
                      : 'text-muted-foreground hover:text-foreground'
                  "
                  @click="theme = t.value"
                >
                  {{ t.label }}
                </button>
              </div>
              <p class="text-xs text-muted-foreground">即时切换界面配色（html.dark）</p>
            </div>

            <!-- 界面字体：改动即保存 + 即时应用 -->
            <div class="space-y-1.5">
              <label for="settings-font" class="text-sm font-medium">界面字体</label>
              <select
                id="settings-font"
                v-model="fontFamily"
                class="h-8 w-full rounded-md border border-input bg-background px-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
              >
                <option v-for="f in fonts" :key="f" :value="f">{{ f }}</option>
              </select>
              <p class="text-xs text-muted-foreground">应用界面字体（即时生效）</p>
            </div>

            <!-- 识别线程数：1–4 -->
            <div class="space-y-1.5">
              <label for="settings-threads" class="text-sm font-medium">识别线程数</label>
              <select
                id="settings-threads"
                v-model.number="threadCount"
                class="h-8 w-24 rounded-md border border-input bg-background px-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
              >
                <option v-for="n in 4" :key="n" :value="n">{{ n }}</option>
              </select>
              <p class="text-xs text-muted-foreground">
                批量识别时并发的线程数（1–4），线程越多占用内存越高
              </p>
            </div>

            <!-- 缩略图尺寸：即时调整网格 cell（对齐 GPUI grid.rs） -->
            <div class="space-y-1.5">
              <div class="flex items-center justify-between">
                <label for="settings-thumb" class="text-sm font-medium">缩略图尺寸</label>
                <span class="font-mono-num text-xs text-muted-foreground">{{ thumbnailSize }}px</span>
              </div>
              <input
                id="settings-thumb"
                v-model.number="thumbnailSize"
                type="range"
                :min="THUMB_RANGE.min"
                :max="THUMB_RANGE.max"
                :step="THUMB_RANGE.step"
                class="w-full accent-primary"
              />
              <p class="text-xs text-muted-foreground">
                即时调整网格 cell（对齐 GPUI）；缩略图缓存按需重新生成
              </p>
            </div>
          </TabsContent>

          <!-- ── 快捷键 ── -->
          <TabsContent value="shortcuts" class="mt-0 space-y-5">
            <div v-for="sec in SHORTCUT_SECTIONS" :key="sec.title" class="space-y-1.5">
              <h3 class="text-sm font-medium">{{ sec.title }}</h3>
              <div class="space-y-1">
                <div
                  v-for="action in sec.actions"
                  :key="action"
                  class="flex items-center justify-between text-sm"
                >
                  <span class="text-muted-foreground">{{ ACTION_DESC[action] }}</span>
                  <kbd
                    class="rounded bg-muted px-2 py-0.5 font-mono-num text-xs text-foreground"
                  >
                    {{ shortcutRows(action).join(' / ') }}
                  </kbd>
                </div>
              </div>
            </div>
          </TabsContent>

          <!-- ── 关于 ── -->
          <TabsContent value="about" class="mt-0 space-y-5">
            <div class="space-y-1">
              <h3 class="text-sm font-medium">Photo Tool（ftpt）</h3>
              <p class="text-xs text-muted-foreground">照片管理与筛选工具（鸟类摄影工作流）</p>
            </div>
            <div class="space-y-1">
              <div
                v-for="[label, value] in aboutRows"
                :key="label"
                class="flex items-center justify-between text-sm"
              >
                <span class="text-muted-foreground">{{ label }}</span>
                <span class="font-mono-num text-xs">{{ value }}</span>
              </div>
            </div>
            <!-- 便携布局说明（对齐 photo-config determine_config_path 语义） -->
            <div class="space-y-1 rounded-md border p-3 text-xs text-muted-foreground">
              <p class="font-medium text-foreground">便携布局</p>
              <p>配置存于可执行文件旁的 PT.db（程序不在系统安装目录时始终视为便携版）。</p>
              <p>缩略图缓存随扫描目录存放（每个目录下 .pt/thumbs）。</p>
            </div>
          </TabsContent>
        </div>
      </Tabs>
    </DialogContent>
  </Dialog>
</template>
