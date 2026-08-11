<script setup lang="ts">
// 右侧信息面板（对齐 GPUI info_panel.rs）：顶部双 tab「信息/调整」+ 卡片化滚动内容。
// 信息 tab：Hero/EXIF/评分/色标/旗标/识别六卡片，数据来自选中拍摄（selection.selected，
// 主选中项优先锚点）；调整 tab：曝光/对比度/饱和度 slider，拖动 350ms 去抖持久化。
// 宽度自持：左缘把手可拖拽，localStorage('ftpt.rightPanelWidth') 持久化，钳制 200–480。
import { computed, onMounted, onUnmounted, reactive, ref, watch } from 'vue'
import { useStorage } from '@vueuse/core'
import {
  GalleryVerticalEndIcon,
  InfoIcon,
  PanelRightCloseIcon,
  PencilIcon,
  RotateCcwIcon,
  ScanSearchIcon,
  XIcon,
} from '@lucide/vue'
import { Button } from '@/components/ui/button'
import { Tabs, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { useCapturesStore } from '@/stores/captures'
import { useSelectionStore } from '@/stores/selection'
import { usePreviewStore } from '@/stores/preview'
import { useRecognitionStore } from '@/stores/recognition'
import { useFilterStore } from '@/stores/filter'
import { correctBird, getAdjustments, getFrequentSpecies, getHistogram, getRecognition, isTauri, ptimgUrl, setAdjustments } from '@/lib/ipc'
import { displayName, formatBadgeLabel, formatBytes, ratingToNumber } from '@/lib/format'
import type {
  AdjustParams,
  CaptureMeta,
  ColorLabel,
  Flag,
  HistogramPayload,
  Recognition,
  RecognitionFailureStage,
  RecognitionStatus,
} from '@/lib/bindings'

defineProps<{
  /** 右侧面板是否可见（关闭按钮与父级 v-show 保持一致） */
  visible: boolean
}>()

/** 关闭面板（App.vue 监听；等价 Ctrl+]） */
const emit = defineEmits<{ toggle: [] }>()

const captures = useCapturesStore()
const selection = useSelectionStore()
const preview = usePreviewStore()
const recognition = useRecognitionStore()
const filter = useFilterStore()

/** 主选中拍摄（锚点优先；无选中/越界为 null） */
const focused = computed<CaptureMeta | null>(() => selection.selected)
const focusedPath = computed<string | null>(() => focused.value?.primaryPath ?? null)

// ── 宽度：可拖拽，localStorage 持久化，范围 200–480（对齐 GPUI 右栏 size_range）──
const width = useStorage('ftpt.rightPanelWidth', 200)
const clampedWidth = computed(() => Math.min(480, Math.max(200, width.value)))
let dragStartX = 0
let dragStartW = 0
function onHandleDown(e: PointerEvent) {
  e.preventDefault()
  dragStartX = e.clientX
  dragStartW = width.value
  ;(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId)
}
function onHandleMove(e: PointerEvent) {
  if (e.buttons === 0) return
  width.value = Math.min(480, Math.max(200, dragStartW + (dragStartX - e.clientX)))
}

// ── 顶部双 tab（信息/调整；状态常驻组件实例，切图不重置）──
const activeTab = ref<'info' | 'adjust'>('info')

// ══════════════════ 信息 tab ══════════════════

/** 缩略图 src（thumb:ready 后 store 版本号递增强制刷新） */
function thumbSrc(c: CaptureMeta): string {
  return ptimgUrl('thumb', c.primaryPath, captures.thumbVersions[c.primaryPath])
}

/** 相机厂商 + 型号拼接（如 "NIKON Z 9"） */
function cameraText(c: CaptureMeta): string {
  if (c.cameraMake && c.cameraModel) return `${c.cameraMake} ${c.cameraModel}`
  return c.cameraMake ?? c.cameraModel ?? '—'
}

/** EXIF 缺失值占位（对齐 GPUI 的 em dash） */
const DASH = '—'

// ── 评分：1–5 星点选，点击当前星清除（0 = 无评分）──
function onStarClick(n: number) {
  const current = focused.value ? ratingToNumber(focused.value.rating) : 0
  void captures.applyRating(selection.selectedPaths, n === current ? 0 : n)
}

// ── 色标：六色 chips（红/黄/绿/蓝/紫 + 清除）──
const LABEL_CHIPS: { label: ColorLabel; cls: string }[] = [
  { label: 'Red', cls: 'bg-label-red' },
  { label: 'Yellow', cls: 'bg-label-yellow' },
  { label: 'Green', cls: 'bg-label-green' },
  { label: 'Blue', cls: 'bg-label-blue' },
  { label: 'Purple', cls: 'bg-label-purple' },
]

// ── 旗标：入选/淘汰/无（互斥单选）──
const FLAG_OPTIONS: { label: string; flag: Flag | null }[] = [
  { label: '入选', flag: 'Pick' },
  { label: '淘汰', flag: 'Reject' },
  { label: '无', flag: null },
]

// ── 关键词：chips 展示 + 输入添加（回车/逗号分隔）+ × 删除 ──
// 语义对齐评分卡：以聚焦图的关键词列表为编辑目标，编辑结果整体作用于选中集
const keywordInput = ref('')

/** 输入添加：回车或逗号分隔多个关键词，去空白/去重后合并进聚焦图列表并应用到选中集 */
function onKeywordAdd() {
  const cur = focused.value?.keywords ?? []
  const parts = keywordInput.value
    .split(/[,，]/)
    .map((s) => s.trim())
    .filter(Boolean)
  if (parts.length === 0) return
  const merged = [...cur]
  for (const p of parts) {
    if (!merged.includes(p)) merged.push(p)
  }
  keywordInput.value = ''
  if (merged.length === cur.length) return
  void captures.applyKeywords(selection.selectedPaths, merged)
}

/** 点击 × 删除单个关键词（聚焦图列表去掉该词后应用到选中集） */
function removeKeyword(kw: string) {
  const cur = focused.value?.keywords ?? []
  const next = cur.filter((k) => k !== kw)
  if (next.length === cur.length) return
  void captures.applyKeywords(selection.selectedPaths, next)
}

// ── 识别：状态 chip + 完整结果（getRecognition）+ 修正鸟种下拉 ──
const STATUS_META: Record<RecognitionStatus, { label: string; cls: string }> = {
  Confirmed: { label: '已识别', cls: 'text-label-green bg-label-green/10 border-label-green/30' },
  NeedsReview: { label: '待复核', cls: 'text-label-yellow bg-label-yellow/10 border-label-yellow/30' },
  Unrecognized: { label: '未检测到鸟类', cls: 'text-muted-foreground bg-muted border-border' },
}

/** 失败阶段中文提示（对齐 domain.rs RecognitionFailureStage::user_message） */
const FAILURE_STAGE_TEXT: Record<RecognitionFailureStage, string> = {
  None: '',
  Detection: '检测异常',
  Classification: '分类异常',
  Mapping: '名录映射失败',
  Assets: '源图不可用',
}

/** 置信度归一化：mock 为 0–1 小数、真实后端 0–100，统一到 0–100 */
function confPercent(c: number | null): number {
  if (c === null) return 0
  return c <= 1 ? c * 100 : c
}

/** 置信度条色：>=80 绿 / >=50 黄 / <50 蓝（对齐 GPUI confidence_color） */
function confBarCls(conf: number): string {
  const pct = confPercent(conf)
  if (pct >= 80) return 'bg-label-green'
  if (pct >= 50) return 'bg-label-yellow'
  return 'bg-label-blue'
}

// ── 完整识别结果：聚焦图有识别记录时 getRecognition 拉取（眼锐度/失败阶段/候选仅完整结果有）──
const fullRecognition = ref<Recognition | null>(null)
/** 加载序号：切图后旧请求结果直接丢弃（防异步回填串图，同调整参数模式） */
let recLoadSeq = 0

function loadRecognition(path: string) {
  const seq = ++recLoadSeq
  void getRecognition(path)
    .then((r) => {
      if (seq !== recLoadSeq || focusedPath.value !== path) return
      fullRecognition.value = r
    })
    .catch(() => {
      if (seq !== recLoadSeq) return
      fullRecognition.value = null
    })
}

// 切图：清空旧结果；有识别记录则重拉
watch(
  focusedPath,
  (path) => {
    fullRecognition.value = null
    if (!path || !focused.value?.recognitionStatus) return
    loadRecognition(path)
  },
  { immediate: true },
)
// 识别状态变化（识别完成回填后）：同图重拉完整结果
watch(
  () => focused.value?.recognitionStatus,
  (status) => {
    const path = focusedPath.value
    if (!path) return
    if (!status) {
      fullRecognition.value = null
      return
    }
    loadRecognition(path)
  },
)
// 识别完成（recognize:done 摘要）后重拉完整结果：mock 模式 captures 原地变更、
// reload 同引用不触发 status watcher，这里以摘要为显式信号补一次（seq 守卫去重）
watch(
  () => recognition.summary,
  (s) => {
    const path = focusedPath.value
    if (!path || !s || !focused.value?.recognitionStatus) return
    loadRecognition(path)
  },
)

/** 展示用鸟名：完整结果优先（名录正式名），回退 CaptureMeta 摘要 */
const displayBirdName = computed(() => fullRecognition.value?.bird?.cnName ?? focused.value?.birdName ?? null)
/** 展示用置信度（完整结果优先；mock 0–1 / 后端 0–100 由 confPercent 归一） */
const displayConfidence = computed<number | null>(() => {
  const r = fullRecognition.value
  if (r?.confidence != null) return r.confidence
  return focused.value?.birdConfidence ?? null
})
/** 置信度数字色：>=80 绿 / >=50 黄 / <50 蓝（对齐 GPUI confidence_color） */
const confTextCls = computed(() => confBarCls(displayConfidence.value ?? 0))
/** 待复核失败阶段中文提示（None 为空串，不显示） */
const failureText = computed(() => FAILURE_STAGE_TEXT[fullRecognition.value?.failureStage ?? 'None'] ?? '')
/** 待复核最接近候选（candidates 中第一个 bird 非空项，对齐 GPUI render_recognition_content） */
const bestCandidate = computed<{ name: string; confidence: number } | null>(() => {
  const r = fullRecognition.value
  if (!r) return null
  for (const c of r.candidates) {
    if (c.bird) return { name: c.bird.cnName, confidence: confPercent(c.confidence) }
  }
  return null
})

/** 眼锐度 tooltip：评分公式（对齐 GPUI eye_sharpness_row 悬浮说明） */
const EYE_SHARPNESS_TIP =
  '眼区域清晰度评分：0.5·ln(1+拉普拉斯方差) + 0.3·ln(1+梯度幅值均值) + 0.2·ln(1+边缘密度)；仅保证单调性，越高越锐利，权重待样片标定'

/** 重新识别当前图（识别进行中由 recognition store 守卫拒绝并发） */
function onRecognize() {
  const path = focusedPath.value
  if (path) void recognition.recognize([path])
}

/** 检测框开关（V 键同一入口） */
function onToggleBbox() {
  preview.toggleBbox()
}

// ── 修正鸟种下拉：展开 → 高频「常用」分组前置 + 名录全量 + 搜索过滤，选即 correctBird（对齐 GPUI correction_open）──
const correctOpen = ref(false)
const correctSearch = ref('')
const correctBoxEl = ref<HTMLElement | null>(null)

/**
 * 高频鸟种（get_frequent_species(10)，全局索引张数降序）——本机使用频次即区域
 * 相关性代理（离线替代区域名录）。加载失败降级空数组（不阻塞下拉主流程）。
 */
const frequentSpecies = ref<string[]>([])

/**
 * 候选 = 高频常用分组（frequent ∩ 名录，保持频次序）+ 其余名录（原名录顺序）。
 * 搜索词命中与否均分组展示：常用组只留命中的，未命中整组隐藏。
 */
const correctOptions = computed(() => {
  const all = filter.speciesOptions
  const freq = frequentSpecies.value.filter((n) => all.includes(n))
  const rest = all.filter((n) => !freq.includes(n))
  const q = correctSearch.value.trim().toLowerCase()
  const match = (n: string) => (q ? n.toLowerCase().includes(q) : true)
  return {
    frequent: freq.filter(match),
    rest: rest.filter(match),
  }
})

function toggleCorrect() {
  correctOpen.value = !correctOpen.value
  correctSearch.value = ''
  if (correctOpen.value) {
    // 名录未加载时补齐（InfoPanel 与 FilterBar 独立挂载，不依赖其 watcher）；
    // 高频列表同理首次展开时拉取
    if (filter.speciesOptions.length === 0) void filter.loadSpecies()
    if (frequentSpecies.value.length === 0) {
      void getFrequentSpecies(10)
        .then((list) => {
          frequentSpecies.value = list
        })
        .catch((e) => console.error('加载高频鸟种失败', e))
    }
  }
}

/** 选择即修正：correctBird 写识别表 → 重拉完整结果 + 全量刷新网格摘要 */
async function onCorrectSelect(name: string) {
  correctOpen.value = false
  correctSearch.value = ''
  const path = focusedPath.value
  if (!path) return
  try {
    await correctBird(path, name)
    loadRecognition(path)
    void captures.reload()
  } catch (e) {
    console.error('鸟种修正失败', e)
  }
}

/** 点击下拉外部关闭（对齐 FilterBar 鸟种下拉模式） */
function onDocMouseDown(e: MouseEvent) {
  if (correctOpen.value && correctBoxEl.value && !correctBoxEl.value.contains(e.target as Node)) {
    correctOpen.value = false
  }
}
onMounted(() => document.addEventListener('mousedown', onDocMouseDown))
onUnmounted(() => {
  document.removeEventListener('mousedown', onDocMouseDown)
  histObserver?.disconnect()
  histObserver = null
})

// ══════════════════ 直方图卡 / GPS（T1 批次 HistogramPanel 切片）══════════════════

/** 直方图数据（null = 未加载；失败置 histError） */
const hist = ref<HistogramPayload | null>(null)
const histError = ref(false)
const histLoading = ref(false)
/** 加载序号：切图后旧请求结果直接丢弃（防异步回填串图，同 getRecognition 模式） */
let histLoadSeq = 0

function loadHistogram(path: string) {
  const seq = ++histLoadSeq
  hist.value = null
  histError.value = false
  histLoading.value = true
  void getHistogram(path)
    .then((h) => {
      if (seq !== histLoadSeq || focusedPath.value !== path) return
      hist.value = h
      histLoading.value = false
    })
    .catch(() => {
      if (seq !== histLoadSeq) return
      histError.value = true
      histLoading.value = false
    })
}

// 切图重拉直方图（EXIF 回填/识别重排后 focusedPath 变化同样触发）
watch(
  focusedPath,
  (path) => {
    if (!path) {
      hist.value = null
      histError.value = false
      histLoading.value = false
      return
    }
    loadHistogram(path)
  },
  { immediate: true },
)

/** 剪切百分比（total 为 0 时为 0%） */
const clipPct = (count: number, total: number): number => (total > 0 ? (count / total) * 100 : 0)
const clipHighPct = computed(() => clipPct(hist.value?.clipHighCount ?? 0, hist.value?.totalPixels ?? 0))
const clipLowPct = computed(() => clipPct(hist.value?.clipLowCount ?? 0, hist.value?.totalPixels ?? 0))

/** 直方图画布引用（挂载/尺寸变化/数据变化时重绘） */
const histCanvas = ref<HTMLCanvasElement | null>(null)

/** 绘制 luma 面积 + 主线 + RGB 细线 + 网格（颜色随 CSS 变量，暗色主题自适应） */
function drawHistogram() {
  const canvas = histCanvas.value
  const h = hist.value
  if (!canvas || !h) return
  const dpr = Math.min(window.devicePixelRatio || 1, 2)
  const cssW = canvas.clientWidth
  const cssH = canvas.clientHeight
  if (cssW <= 0 || cssH <= 0) return
  const pw = Math.round(cssW * dpr)
  const ph = Math.round(cssH * dpr)
  if (canvas.width !== pw || canvas.height !== ph) {
    canvas.width = pw
    canvas.height = ph
  }
  const ctx = canvas.getContext('2d')
  if (!ctx) return
  const css = getComputedStyle(document.documentElement)
  const fg = css.getPropertyValue('--foreground').trim() || '#cdd6f4'
  const muted = css.getPropertyValue('--muted-foreground').trim() || '#a6adc8'
  const border = css.getPropertyValue('--border').trim() || '#313244'
  const W = canvas.width
  const H = canvas.height
  const plotH = H - 14
  ctx.clearRect(0, 0, W, H)
  // 网格：每 64 级竖线 + 底边
  ctx.strokeStyle = border
  ctx.lineWidth = 1
  ctx.beginPath()
  for (let i = 0; i <= 4; i++) {
    const x = Math.round((i * 64 * (W - 1)) / 255) + 0.5
    ctx.moveTo(x, 0)
    ctx.lineTo(x, plotH)
  }
  ctx.moveTo(0, plotH + 0.5)
  ctx.lineTo(W, plotH + 0.5)
  ctx.stroke()
  const maxBin = Math.max(1, ...h.luma)
  const xOf = (i: number) => (i * (W - 1)) / 255
  const yOf = (v: number) => plotH - (v / maxBin) * plotH
  const drawCurve = (bins: number[], color: string, alpha: number) => {
    ctx.strokeStyle = color
    ctx.globalAlpha = alpha
    ctx.beginPath()
    for (let i = 0; i < 256; i++) {
      const x = xOf(i)
      const y = yOf(bins[i])
      if (i === 0) ctx.moveTo(x, y)
      else ctx.lineTo(x, y)
    }
    ctx.stroke()
    ctx.globalAlpha = 1
  }
  // RGB 三色细线（半透明，先画避免盖住 luma 主线）
  drawCurve(h.r, '#f87171', 0.5)
  drawCurve(h.g, '#4ade80', 0.5)
  drawCurve(h.b, '#60a5fa', 0.5)
  // luma 面积填充（低透明度）+ 主线
  ctx.fillStyle = fg
  ctx.globalAlpha = 0.18
  ctx.beginPath()
  ctx.moveTo(0, plotH)
  for (let i = 0; i < 256; i++) ctx.lineTo(xOf(i), yOf(h.luma[i]))
  ctx.lineTo(W - 1, plotH)
  ctx.closePath()
  ctx.fill()
  ctx.globalAlpha = 1
  ctx.strokeStyle = fg
  ctx.lineWidth = 1.2
  ctx.beginPath()
  for (let i = 0; i < 256; i++) {
    const x = xOf(i)
    const y = yOf(h.luma[i])
    if (i === 0) ctx.moveTo(x, y)
    else ctx.lineTo(x, y)
  }
  ctx.stroke()
  // 底部刻度
  ctx.fillStyle = muted
  ctx.font = '9px ui-monospace, monospace'
  ctx.fillText('0', 2, H - 2)
  ctx.fillText('255', W - 24, H - 2)
}

// 数据或画布挂载变化时重绘（flush: 'post' 保证 canvas ref 已就位）
watch([hist, histCanvas], () => drawHistogram(), { flush: 'post' })

/** 画布尺寸变化（面板拖宽）时重绘 */
let histObserver: ResizeObserver | null = null
watch(histCanvas, (el) => {
  histObserver?.disconnect()
  histObserver = null
  if (!el) return
  histObserver = new ResizeObserver(() => drawHistogram())
  histObserver.observe(el)
})

// ── GPS：十进制坐标 + OSM 地图链接 ──

/** 十进制坐标显示（6 位小数；负号保留，南纬/西经为负） */
function fmtGps(v: number): string {
  return v.toFixed(6)
}

/** OSM 地图链接（zoom 15，坐标居中） */
function gpsMapUrl(lat: number, lon: number): string {
  return `https://www.openstreetmap.org/?mlat=${lat.toFixed(6)}&mlon=${lon.toFixed(6)}&zoom=15`
}

/**
 * 打开地图：Tauri 环境用 window.open（WebView2 外部 URL 默认交系统浏览器），
 * 浏览器 mock 环境不拦截，走 anchor target=_blank 默认行为。
 */
function onOpenMap(e: Event) {
  const c = focused.value
  if (!c || c.gpsLat == null || c.gpsLon == null) return
  if (isTauri) {
    e.preventDefault()
    window.open(gpsMapUrl(c.gpsLat, c.gpsLon), '_blank', 'noopener')
  }
}

// ══════════════════ 调整 tab ══════════════════

/** 本地调整参数（跟随焦点图；拖动实时改内存，350ms 去抖后持久化） */
const adj = reactive<AdjustParams>({ exposure: 0, contrast: 0, saturation: 0 })
/** 加载序号：切图后旧请求的结果直接丢弃（防异步回填串图） */
let loadSeq = 0
let persistTimer: ReturnType<typeof setTimeout> | null = null

const isNeutral = computed(
  () => adj.exposure === 0 && adj.contrast === 0 && adj.saturation === 0,
)

/** 焦点图变化：重新拉取该图已持久化的调整参数 */
watch(
  focusedPath,
  (path) => {
    if (persistTimer) {
      clearTimeout(persistTimer)
      persistTimer = null
    }
    if (!path) {
      adj.exposure = 0
      adj.contrast = 0
      adj.saturation = 0
      return
    }
    const seq = ++loadSeq
    void getAdjustments(path).then((p) => {
      // 加载期间焦点已切换：结果过期，丢弃
      if (seq !== loadSeq || focusedPath.value !== path) return
      adj.exposure = p.exposure
      adj.contrast = p.contrast
      adj.saturation = p.saturation
    })
  },
  { immediate: true },
)

function persistNow() {
  const path = focusedPath.value
  if (!path) return
  void setAdjustments(path, { exposure: adj.exposure, contrast: adj.contrast, saturation: adj.saturation })
}

/** 拖动/键盘调整：立即更新本地值，350ms 去抖后持久化（对齐 GPUI 去抖语义） */
function onSliderInput(key: keyof AdjustParams, e: Event) {
  adj[key] = Number((e.target as HTMLInputElement).value)
  if (persistTimer) clearTimeout(persistTimer)
  persistTimer = setTimeout(() => {
    persistTimer = null
    persistNow()
  }, 350)
}

/** 重置（单项/全部）：立即持久化（刷新待写去抖，避免被旧值覆盖） */
function resetField(key: keyof AdjustParams) {
  adj[key] = 0
  flushPersist()
}
function resetAll() {
  adj.exposure = 0
  adj.contrast = 0
  adj.saturation = 0
  flushPersist()
}
function flushPersist() {
  if (persistTimer) {
    clearTimeout(persistTimer)
    persistTimer = null
  }
  persistNow()
}

onUnmounted(() => {
  if (persistTimer) clearTimeout(persistTimer)
})

/** 数值文案（对齐 GPUI adjust_slider_row）：曝光 ±0.00 EV，对比度/饱和度 ±N */
function fmtExposure(v: number): string {
  return `${v >= 0 ? '+' : ''}${v.toFixed(2)} EV`
}
function fmtSigned(v: number): string {
  return `${v >= 0 ? '+' : ''}${v}`
}
/** 非中性数值用 accent（primary）强调 */
function valueCls(v: number): string {
  return v !== 0 ? 'text-primary' : ''
}
</script>

<template>
  <aside
    class="relative flex h-full shrink-0 flex-col border-l bg-card"
    :style="{ width: `${clampedWidth}px` }"
  >
    <!-- 拖宽把手（左缘，指针捕获保证拖出面板仍生效） -->
    <div
      class="absolute inset-y-0 left-0 z-10 w-1 cursor-col-resize touch-none select-none hover:bg-primary/40"
      @pointerdown="onHandleDown"
      @pointermove="onHandleMove"
    />

    <!-- 面板标题栏：信息/调整 tab + 关闭按钮（对齐 GPUI info_panel 头部；标准 Tabs 组件） -->
    <div class="flex h-10 shrink-0 items-center justify-between border-b border-border px-2">
      <Tabs v-model="activeTab" class="h-full">
        <TabsList class="h-full items-stretch rounded-none bg-transparent p-0 text-muted-foreground">
          <TabsTrigger
            value="info"
            class="h-full rounded-none border-b-2 border-transparent px-2 text-xs data-[state=active]:border-primary data-[state=active]:bg-transparent data-[state=active]:font-medium data-[state=active]:text-foreground data-[state=active]:shadow-none"
          >
            信息
          </TabsTrigger>
          <TabsTrigger
            value="adjust"
            class="h-full rounded-none border-b-2 border-transparent px-2 text-xs data-[state=active]:border-primary data-[state=active]:bg-transparent data-[state=active]:font-medium data-[state=active]:text-foreground data-[state=active]:shadow-none"
          >
            调整
          </TabsTrigger>
        </TabsList>
      </Tabs>
      <Button
        size="icon-xs"
        variant="ghost"
        title="关闭右侧面板  Ctrl+]"
        :aria-pressed="visible"
        @click="emit('toggle')"
      >
        <PanelRightCloseIcon class="size-3.5" />
      </Button>
    </div>

    <!-- tab 内容（卡片流，可滚动） -->
    <div class="min-h-0 flex-1 space-y-3 overflow-y-auto p-3">
      <!-- ════════════ 信息 tab ════════════ -->
      <template v-if="activeTab === 'info'">
        <!-- ── Hero 卡：缩略图 + 文件名/格式徽标 + 分辨率/大小 ── -->
        <div class="flex flex-col gap-2 rounded-md border border-border bg-card p-3 shadow-sm">
          <template v-if="focused">
            <div
              class="flex h-[8.75rem] w-full items-center justify-center overflow-hidden rounded-md bg-muted"
            >
              <img :src="thumbSrc(focused)" :alt="displayName(focused)" class="size-full object-cover" />
            </div>
            <div class="flex items-center justify-between gap-2">
              <div class="min-w-0 flex-1 truncate text-[0.8125rem] font-semibold" :title="focused.primaryPath">
                {{ displayName(focused) }}
              </div>
              <div
                class="shrink-0 rounded-sm border border-border px-1 tabular-nums text-[0.625rem] text-muted-foreground"
              >
                {{ formatBadgeLabel(focused).toUpperCase() }}
              </div>
            </div>
            <!-- 鸟种中文名（存在时显示，对齐 GPUI hero） -->
            <div v-if="focused.birdName" class="truncate text-[0.8125rem] font-medium text-primary">
              {{ focused.birdName }}
            </div>
            <div class="flex items-center gap-2">
              <span class="tabular-nums text-xs">
                {{
                  focused.imageWidth && focused.imageHeight
                    ? `${focused.imageWidth} × ${focused.imageHeight}`
                    : '— × —'
                }}
              </span>
              <span class="text-xs text-muted-foreground/60">·</span>
              <span class="tabular-nums text-xs text-muted-foreground">
                {{ formatBytes(focused.fileSize) }}
              </span>
            </div>
          </template>
          <template v-else>
            <div
              class="flex h-[8.75rem] w-full items-center justify-center rounded-md bg-muted"
            >
              <GalleryVerticalEndIcon class="size-8 text-muted-foreground/30" />
            </div>
            <div class="text-xs text-muted-foreground">未选择图片</div>
          </template>
        </div>

        <!-- ── EXIF 卡：拍摄参数格（对齐 GPUI render_exif_section）── -->
        <div class="flex flex-col gap-2 rounded-md border border-border bg-card p-3 shadow-sm">
          <div class="text-xs font-medium text-muted-foreground">拍摄信息</div>
          <template v-if="focused">
            <div class="flex gap-2">
              <div class="flex-1 rounded-md bg-muted px-2 py-1">
                <div class="text-[0.625rem] text-muted-foreground">焦距</div>
                <div class="truncate tabular-nums text-[0.8125rem]">{{ focused.focalLength ?? DASH }}</div>
              </div>
              <div class="flex-1 rounded-md bg-muted px-2 py-1">
                <div class="text-[0.625rem] text-muted-foreground">光圈</div>
                <div class="truncate tabular-nums text-[0.8125rem]">{{ focused.fNumber ?? DASH }}</div>
              </div>
            </div>
            <div class="flex gap-2">
              <div class="flex-1 rounded-md bg-muted px-2 py-1">
                <div class="text-[0.625rem] text-muted-foreground">快门</div>
                <div class="truncate tabular-nums text-[0.8125rem]">{{ focused.exposureTime ?? DASH }}</div>
              </div>
              <div class="flex-1 rounded-md bg-muted px-2 py-1">
                <div class="text-[0.625rem] text-muted-foreground">ISO</div>
                <div class="truncate tabular-nums text-[0.8125rem]">{{ focused.iso ?? DASH }}</div>
              </div>
            </div>
            <div class="flex items-center justify-between gap-2">
              <span class="shrink-0 text-xs text-muted-foreground">相机</span>
              <span class="min-w-0 truncate text-xs">{{ cameraText(focused) }}</span>
            </div>
            <div class="flex items-center justify-between gap-2">
              <span class="shrink-0 text-xs text-muted-foreground">镜头</span>
              <span class="min-w-0 truncate text-xs">{{ focused.lens ?? DASH }}</span>
            </div>
            <div class="flex items-center justify-between gap-2">
              <span class="shrink-0 text-xs text-muted-foreground">日期</span>
              <span class="min-w-0 truncate text-xs">{{ focused.dateTaken ?? DASH }}</span>
            </div>
            <!-- GPS 行（T1 批次）：有坐标显示十进制 6 位 + OSM 地图链接；无坐标显示「无」 -->
            <div class="flex items-center justify-between gap-2">
              <span class="shrink-0 text-xs text-muted-foreground">位置</span>
              <span
                v-if="focused.gpsLat != null && focused.gpsLon != null"
                class="flex min-w-0 items-center gap-1.5 text-xs"
              >
                <span class="truncate tabular-nums">
                  {{ fmtGps(focused.gpsLat) }}, {{ fmtGps(focused.gpsLon) }}
                </span>
                <a
                  :href="gpsMapUrl(focused.gpsLat, focused.gpsLon)"
                  target="_blank"
                  rel="noreferrer"
                  class="shrink-0 text-primary hover:underline"
                  @click="onOpenMap"
                >在地图查看</a>
              </span>
              <span v-else class="text-xs text-muted-foreground">无</span>
            </div>
          </template>
          <div v-else class="text-xs text-muted-foreground">未选择图片</div>
        </div>

        <!-- ── 直方图卡（T1 批次）：luma 曲线 + RGB 细线 + 剪切统计 ── -->
        <div class="flex flex-col gap-2 rounded-md border border-border bg-card p-3 shadow-sm">
          <div class="flex items-center justify-between">
            <span class="text-xs font-medium text-muted-foreground">直方图</span>
            <span v-if="histLoading" class="text-[0.625rem] text-muted-foreground/60">计算中…</span>
          </div>
          <template v-if="hist">
            <canvas ref="histCanvas" class="h-20 w-full" />
            <div class="flex gap-3 text-[0.625rem] tabular-nums">
              <span class="text-label-red">高光剪切 {{ clipHighPct.toFixed(1) }}%</span>
              <span class="text-label-blue">死黑 {{ clipLowPct.toFixed(1) }}%</span>
            </div>
          </template>
          <div v-else-if="histError" class="text-xs text-muted-foreground">
            直方图不可用（解码失败）
          </div>
          <div v-else-if="!focused" class="text-xs text-muted-foreground">未选择图片</div>
          <div v-else class="text-xs text-muted-foreground">计算中…</div>
        </div>

        <!-- ── 识别卡：状态 chip + 完整结果（getRecognition）+ 重新识别/检测框/修正鸟种 ── -->
        <div class="flex flex-col gap-2 rounded-md border border-border bg-card p-3 shadow-sm">
          <span class="text-xs font-medium text-muted-foreground">识别</span>
          <!-- 未选择图片 -->
          <div v-if="!focused" class="flex flex-col gap-2">
            <div class="text-xs text-muted-foreground">未选择图片</div>
            <Button size="sm" disabled>
              <ScanSearchIcon data-icon="inline-start" />
              识别此照片
            </Button>
          </div>
          <!-- 识别进行中（对齐 GPUI busy 分支：隐藏结果内容） -->
          <div v-else-if="recognition.running" class="flex flex-col gap-2">
            <div class="flex items-center gap-1.5 text-xs text-primary">
              <ScanSearchIcon class="size-3.5 animate-pulse" />
              识别中…
            </div>
          </div>
          <!-- 未识别（无记录） -->
          <div v-else-if="!focused.recognitionStatus" class="flex flex-col gap-2">
            <div class="text-xs text-muted-foreground">尚未识别</div>
            <Button size="sm" @click="onRecognize">
              <ScanSearchIcon data-icon="inline-start" />
              识别此照片 (b)
            </Button>
          </div>
          <!-- 有识别记录：完整结果渲染 -->
          <template v-else>
            <div class="flex items-center justify-between gap-2">
              <span
                class="rounded-sm border px-2 py-0.5 text-[0.6875rem] select-none"
                :class="STATUS_META[focused.recognitionStatus].cls"
              >
                {{ STATUS_META[focused.recognitionStatus].label }}
              </span>
              <span
                v-if="confPercent(displayConfidence) > 0"
                class="tabular-nums text-[0.9375rem] font-semibold"
                :class="confTextCls"
              >
                {{ confPercent(displayConfidence).toFixed(1) }}%
              </span>
            </div>
            <!-- 已确认：鸟名 + 置信度条 -->
            <template v-if="focused.recognitionStatus === 'Confirmed'">
              <div v-if="displayBirdName" class="truncate text-xs font-medium text-primary">
                {{ displayBirdName }}
              </div>
              <div v-if="displayConfidence !== null" class="h-1 w-full overflow-hidden rounded-full bg-muted">
                <div
                  class="h-full rounded-full transition-[width]"
                  :class="confBarCls(displayConfidence)"
                  :style="{ width: `${confPercent(displayConfidence)}%` }"
                />
              </div>
            </template>
            <!-- 待复核：失败阶段中文提示 + 最接近候选 -->
            <template v-if="focused.recognitionStatus === 'NeedsReview'">
              <div v-if="failureText" class="text-xs text-label-yellow">{{ failureText }}</div>
              <div v-if="bestCandidate" class="text-xs text-muted-foreground">
                最接近：{{ bestCandidate.name }} {{ bestCandidate.confidence.toFixed(1) }}%
              </div>
            </template>
            <!-- 眼锐度行（完整结果才有；info 图标悬浮显示评分公式） -->
            <div
              v-if="fullRecognition?.eyeSharpness != null"
              class="flex items-center gap-1 text-xs text-muted-foreground"
              :title="EYE_SHARPNESS_TIP"
            >
              <span class="tabular-nums">眼锐度 {{ fullRecognition.eyeSharpness.toFixed(2) }}</span>
              <InfoIcon class="size-3 shrink-0 text-muted-foreground/70" />
            </div>
            <!-- 动作行：重新识别 + 检测框 -->
            <div class="flex justify-end gap-1">
              <Button size="sm" variant="ghost" @click="onRecognize">
                <RotateCcwIcon data-icon="inline-start" />
                重新识别
              </Button>
              <Button size="sm" variant="ghost" @click="onToggleBbox">
                {{ preview.bboxVisible ? '隐藏检测框' : '显示检测框' }}
              </Button>
            </div>
            <!-- 修正鸟种：展开 → 搜索下拉（名录全量，选即修正，对齐 GPUI correction_open） -->
            <div ref="correctBoxEl" class="relative">
              <Button size="sm" variant="ghost" class="w-full justify-start" @click="toggleCorrect">
                <PencilIcon data-icon="inline-start" />
                {{ correctOpen ? '收起修正' : '修正鸟种…' }}
              </Button>
              <!-- 向上展开：识别卡位于面板底部，向下会被滚动容器裁切 -->
              <div
                v-if="correctOpen"
                class="absolute right-0 bottom-full z-20 mb-1 w-full overflow-hidden rounded-md border border-border bg-popover shadow-md"
              >
                <input
                  v-model="correctSearch"
                  type="text"
                  class="w-full border-b border-border bg-transparent px-2 py-1.5 text-xs outline-none placeholder:text-muted-foreground"
                  placeholder="搜索鸟种…"
                  @click.stop
                />
                <div class="max-h-40 overflow-y-auto">
                  <template v-if="correctOptions.frequent.length > 0">
                    <div class="px-2 pt-1 pb-0.5 text-[10px] font-medium text-muted-foreground/70">
                      常用
                    </div>
                    <button
                      v-for="n in correctOptions.frequent"
                      :key="n"
                      type="button"
                      class="block w-full truncate px-2 py-1 text-left text-xs hover:bg-accent hover:text-accent-foreground"
                      @click="onCorrectSelect(n)"
                    >
                      {{ n }}
                    </button>
                  </template>
                  <div
                    v-if="correctOptions.frequent.length > 0 && correctOptions.rest.length > 0"
                    class="my-1 border-t border-border/60"
                  ></div>
                  <button
                    v-for="n in correctOptions.rest"
                    :key="n"
                    type="button"
                    class="block w-full truncate px-2 py-1 text-left text-xs hover:bg-accent hover:text-accent-foreground"
                    @click="onCorrectSelect(n)"
                  >
                    {{ n }}
                  </button>
                  <div
                    v-if="correctOptions.frequent.length === 0 && correctOptions.rest.length === 0"
                    class="px-2 py-1 text-xs text-muted-foreground"
                  >
                    无匹配鸟种
                  </div>
                </div>
              </div>
            </div>
          </template>
        </div>

        <!-- ── 评分卡：1–5 星点选 + 清除 ── -->
        <div class="flex flex-col gap-2 rounded-md border border-border bg-card p-3 shadow-sm">
          <div class="flex items-center justify-between">
            <span class="text-xs font-medium text-muted-foreground">评分</span>
            <button
              v-if="focused && ratingToNumber(focused.rating) > 0"
              type="button"
              class="text-xs text-primary hover:underline"
              @click="captures.applyRating(selection.selectedPaths, 0)"
            >
              清除
            </button>
          </div>
          <template v-if="focused">
            <div class="flex items-center gap-1">
              <button
                v-for="n in 5"
                :key="n"
                type="button"
                class="text-lg leading-none transition-colors select-none"
                :class="n <= ratingToNumber(focused.rating) ? 'text-rating' : 'text-muted-foreground/30 hover:text-muted-foreground/60'"
                :title="`${n} 星`"
                @click="onStarClick(n)"
              >
                ★
              </button>
            </div>
          </template>
          <div v-else class="text-xs text-muted-foreground">未选择图片</div>
        </div>

        <!-- ── 色标卡：六色 chips 点选 ── -->
        <div class="flex flex-col gap-2 rounded-md border border-border bg-card p-3 shadow-sm">
          <div class="flex items-center justify-between">
            <span class="text-xs font-medium text-muted-foreground">颜色标签</span>
            <button
              v-if="focused && focused.colorLabel !== 'None'"
              type="button"
              class="text-xs text-primary hover:underline"
              @click="captures.applyColorLabel(selection.selectedPaths, null)"
            >
              清除
            </button>
          </div>
          <template v-if="focused">
            <div class="flex items-center justify-between">
              <button
                v-for="c in LABEL_CHIPS"
                :key="c.label"
                type="button"
                class="size-5 rounded-full border-2 transition-colors select-none"
                :class="[
                  c.cls,
                  focused.colorLabel === c.label
                    ? 'border-foreground'
                    : 'border-transparent hover:border-muted-foreground',
                ]"
                :title="c.label"
                @click="captures.applyColorLabel(selection.selectedPaths, c.label)"
              />
              <!-- 第六 chip：清除色标（无） -->
              <button
                type="button"
                class="flex size-5 items-center justify-center rounded-full border-2 bg-muted text-muted-foreground select-none"
                :class="
                  focused.colorLabel === 'None'
                    ? 'border-foreground'
                    : 'border-transparent hover:border-muted-foreground'
                "
                title="清除色标"
                @click="captures.applyColorLabel(selection.selectedPaths, null)"
              >
                <XIcon class="size-3" />
              </button>
            </div>
          </template>
          <div v-else class="text-xs text-muted-foreground">未选择图片</div>
        </div>

        <!-- ── 旗标卡：入选/淘汰/无 ── -->
        <div class="flex flex-col gap-2 rounded-md border border-border bg-card p-3 shadow-sm">
          <span class="text-xs font-medium text-muted-foreground">旗标</span>
          <template v-if="focused">
            <div class="flex items-center gap-1">
              <button
                v-for="o in FLAG_OPTIONS"
                :key="o.label"
                type="button"
                class="flex-1 rounded-sm border px-2 py-1 text-xs transition-colors select-none"
                :class="
                  focused.flag === o.flag
                    ? 'border-primary bg-primary/10 font-medium text-primary'
                    : 'border-border text-muted-foreground hover:bg-muted'
                "
                @click="captures.applyFlag(selection.selectedPaths, o.flag)"
              >
                {{ o.label }}
              </button>
            </div>
          </template>
          <div v-else class="text-xs text-muted-foreground">未选择图片</div>
        </div>

        <!-- ── 关键词卡：chips 展示 + 输入添加（回车/逗号分隔）+ × 删除；作用于选中集 ── -->
        <div class="flex flex-col gap-2 rounded-md border border-border bg-card p-3 shadow-sm">
          <div class="flex items-center justify-between">
            <span class="text-xs font-medium text-muted-foreground">关键词</span>
            <button
              v-if="focused && focused.keywords.length > 0"
              type="button"
              class="text-xs text-primary hover:underline"
              @click="captures.applyKeywords(selection.selectedPaths, [])"
            >
              清除
            </button>
          </div>
          <template v-if="focused">
            <!-- chips（× 删除） -->
            <div v-if="focused.keywords.length > 0" class="flex flex-wrap gap-1">
              <span
                v-for="kw in focused.keywords"
                :key="kw"
                class="flex items-center gap-1 rounded-sm border border-border bg-muted px-1.5 py-0.5 text-xs select-none"
              >
                {{ kw }}
                <button
                  type="button"
                  class="text-muted-foreground hover:text-foreground"
                  :aria-label="`删除关键词 ${kw}`"
                  @click="removeKeyword(kw)"
                >
                  <XIcon class="size-3" />
                </button>
              </span>
            </div>
            <div v-else class="text-xs text-muted-foreground">暂无关键词</div>
            <!-- 输入添加：回车/逗号提交 -->
            <input
              v-model="keywordInput"
              type="text"
              placeholder="添加关键词（回车或逗号分隔）"
              class="h-8 w-full rounded-sm border border-border bg-card px-2 text-xs text-foreground outline-none placeholder:text-muted-foreground"
              @keydown.enter.prevent="onKeywordAdd"
            />
          </template>
          <div v-else class="text-xs text-muted-foreground">未选择图片</div>
        </div>
      </template>

      <!-- ════════════ 调整 tab ════════════ -->
      <template v-else>
        <div class="flex flex-col gap-2 rounded-md border border-border bg-card p-3 shadow-sm">
          <div class="flex items-center justify-between">
            <span class="text-xs font-medium text-muted-foreground">基础调整</span>
            <button
              v-if="!isNeutral"
              type="button"
              class="text-xs text-primary hover:underline"
              @click="resetAll"
            >
              重置全部
            </button>
          </div>
          <template v-if="focused">
            <!-- 曝光（EV ±2.0，步进 0.05） -->
            <div class="flex items-center gap-2">
              <span class="w-11 shrink-0 text-xs text-muted-foreground">曝光</span>
              <input
                type="range"
                class="h-4 w-full accent-primary"
                min="-2"
                max="2"
                step="0.05"
                :value="adj.exposure"
                aria-label="曝光"
                @input="onSliderInput('exposure', $event)"
              />
              <span class="w-16 shrink-0 text-right tabular-nums text-xs" :class="valueCls(adj.exposure ?? 0)">
                {{ fmtExposure(adj.exposure ?? 0) }}
              </span>
              <button
                v-if="adj.exposure !== 0"
                type="button"
                class="w-10 shrink-0 text-right text-xs text-primary hover:underline"
                @click="resetField('exposure')"
              >
                重置
              </button>
              <span v-else class="w-10 shrink-0" />
            </div>
            <!-- 对比度（±100，步进 1） -->
            <div class="flex items-center gap-2">
              <span class="w-11 shrink-0 text-xs text-muted-foreground">对比度</span>
              <input
                type="range"
                class="h-4 w-full accent-primary"
                min="-100"
                max="100"
                step="1"
                :value="adj.contrast"
                aria-label="对比度"
                @input="onSliderInput('contrast', $event)"
              />
              <span class="w-16 shrink-0 text-right tabular-nums text-xs" :class="valueCls(adj.contrast ?? 0)">
                {{ fmtSigned(adj.contrast ?? 0) }}
              </span>
              <button
                v-if="adj.contrast !== 0"
                type="button"
                class="w-10 shrink-0 text-right text-xs text-primary hover:underline"
                @click="resetField('contrast')"
              >
                重置
              </button>
              <span v-else class="w-10 shrink-0" />
            </div>
            <!-- 饱和度（±100，步进 1） -->
            <div class="flex items-center gap-2">
              <span class="w-11 shrink-0 text-xs text-muted-foreground">饱和度</span>
              <input
                type="range"
                class="h-4 w-full accent-primary"
                min="-100"
                max="100"
                step="1"
                :value="adj.saturation"
                aria-label="饱和度"
                @input="onSliderInput('saturation', $event)"
              />
              <span class="w-16 shrink-0 text-right tabular-nums text-xs" :class="valueCls(adj.saturation ?? 0)">
                {{ fmtSigned(adj.saturation ?? 0) }}
              </span>
              <button
                v-if="adj.saturation !== 0"
                type="button"
                class="w-10 shrink-0 text-right text-xs text-primary hover:underline"
                @click="resetField('saturation')"
              >
                重置
              </button>
              <span v-else class="w-10 shrink-0" />
            </div>
          </template>
          <div v-else class="text-xs text-muted-foreground">未选择图片</div>
        </div>
      </template>
    </div>
  </aside>
</template>
