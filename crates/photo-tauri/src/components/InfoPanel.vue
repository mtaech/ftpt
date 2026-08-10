<script setup lang="ts">
// 右侧信息面板（对齐 GPUI info_panel.rs）：顶部双 tab「信息/调整」+ 卡片化滚动内容。
// 信息 tab：Hero/EXIF/评分/色标/旗标/识别六卡片，数据来自选中拍摄（selection.selected，
// 主选中项优先锚点）；调整 tab：曝光/对比度/饱和度 slider，拖动 350ms 去抖持久化。
// 宽度自持：左缘把手可拖拽，localStorage('ftpt.rightPanelWidth') 持久化，钳制 200–480。
import { computed, onUnmounted, reactive, ref, watch } from 'vue'
import { useStorage } from '@vueuse/core'
import {
  GalleryVerticalEndIcon,
  PanelRightCloseIcon,
  RotateCcwIcon,
  ScanSearchIcon,
  XIcon,
} from '@lucide/vue'
import { Button } from '@/components/ui/button'
import { useCapturesStore } from '@/stores/captures'
import { useSelectionStore } from '@/stores/selection'
import { usePreviewStore } from '@/stores/preview'
import { getAdjustments, ptimgUrl, setAdjustments } from '@/lib/ipc'
import { displayName, formatBadgeLabel, formatBytes, ratingToNumber } from '@/lib/format'
import type { AdjustParams, CaptureMeta, ColorLabel, Flag, RecognitionStatus } from '@/lib/bindings'

defineProps<{
  /** 右侧面板是否可见（关闭按钮与父级 v-show 保持一致） */
  visible: boolean
}>()

/** 关闭面板（App.vue 监听；等价 Ctrl+]） */
const emit = defineEmits<{ toggle: [] }>()

const captures = useCapturesStore()
const selection = useSelectionStore()
const preview = usePreviewStore()

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

// ── 识别：三层状态 chip + 置信度 ──
const STATUS_META: Record<RecognitionStatus, { label: string; cls: string }> = {
  Confirmed: { label: '已识别', cls: 'text-label-green bg-label-green/10 border-label-green/30' },
  NeedsReview: { label: '待复核', cls: 'text-label-yellow bg-label-yellow/10 border-label-yellow/30' },
  Unrecognized: { label: '未检测到鸟类', cls: 'text-muted-foreground bg-muted border-border' },
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

/** 识别动作占位（Phase 3 接线：recognize_captures / 检测框叠加） */
function onRecognize() {
  console.debug('[InfoPanel] 重新识别：Phase 3 接线')
}
function onToggleBbox() {
  console.debug('[InfoPanel] 检测框开关：Phase 3 接线', { bboxVisible: preview.bboxVisible })
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
    class="relative flex h-full shrink-0 flex-col border-l bg-background"
    :style="{ width: `${clampedWidth}px` }"
  >
    <!-- 拖宽把手（左缘，指针捕获保证拖出面板仍生效） -->
    <div
      class="absolute inset-y-0 left-0 z-10 w-1 cursor-col-resize touch-none select-none hover:bg-primary/40"
      @pointerdown="onHandleDown"
      @pointermove="onHandleMove"
    />

    <!-- 面板标题栏：信息/调整 tab + 关闭按钮（对齐 GPUI info_panel 头部） -->
    <div class="flex h-9 shrink-0 items-center justify-between border-b border-border px-2">
      <div class="flex h-full items-center gap-1">
        <button
          type="button"
          class="flex h-full cursor-pointer items-center border-b-2 px-2 text-xs select-none"
          :class="
            activeTab === 'info'
              ? 'border-primary font-medium text-foreground'
              : 'border-transparent text-muted-foreground hover:text-foreground'
          "
          @click="activeTab = 'info'"
        >
          信息
        </button>
        <button
          type="button"
          class="flex h-full cursor-pointer items-center border-b-2 px-2 text-xs select-none"
          :class="
            activeTab === 'adjust'
              ? 'border-primary font-medium text-foreground'
              : 'border-transparent text-muted-foreground hover:text-foreground'
          "
          @click="activeTab = 'adjust'"
        >
          调整
        </button>
      </div>
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
        <div class="flex flex-col gap-2 rounded-md border border-border bg-card p-3">
          <template v-if="focused">
            <div
              class="flex h-[140px] w-full items-center justify-center overflow-hidden rounded-md bg-muted"
            >
              <img :src="thumbSrc(focused)" :alt="displayName(focused)" class="size-full object-cover" />
            </div>
            <div class="flex items-center justify-between gap-2">
              <div class="min-w-0 flex-1 truncate text-[13px] font-semibold" :title="focused.primaryPath">
                {{ displayName(focused) }}
              </div>
              <div
                class="shrink-0 rounded-sm border border-border px-1 font-mono-num text-[10px] text-muted-foreground"
              >
                {{ formatBadgeLabel(focused).toUpperCase() }}
              </div>
            </div>
            <!-- 鸟种中文名（存在时显示，对齐 GPUI hero） -->
            <div v-if="focused.birdName" class="truncate text-[13px] font-medium text-primary">
              {{ focused.birdName }}
            </div>
            <div class="flex items-center gap-2">
              <span class="font-mono-num text-xs">
                {{
                  focused.imageWidth && focused.imageHeight
                    ? `${focused.imageWidth} × ${focused.imageHeight}`
                    : '— × —'
                }}
              </span>
              <span class="text-xs text-muted-foreground/60">·</span>
              <span class="font-mono-num text-xs text-muted-foreground">
                {{ formatBytes(focused.fileSize) }}
              </span>
            </div>
          </template>
          <template v-else>
            <div
              class="flex h-[140px] w-full items-center justify-center rounded-md bg-muted"
            >
              <GalleryVerticalEndIcon class="size-8 text-muted-foreground/30" />
            </div>
            <div class="text-xs text-muted-foreground">未选择图片</div>
          </template>
        </div>

        <!-- ── EXIF 卡：拍摄参数格（对齐 GPUI render_exif_section）── -->
        <div class="flex flex-col gap-2 rounded-md border border-border bg-card p-3">
          <div class="text-xs font-medium text-muted-foreground">拍摄信息</div>
          <template v-if="focused">
            <div class="flex gap-2">
              <div class="flex-1 rounded-md bg-muted px-2 py-1">
                <div class="text-[10px] text-muted-foreground">焦距</div>
                <div class="truncate font-mono-num text-[13px]">{{ focused.focalLength ?? DASH }}</div>
              </div>
              <div class="flex-1 rounded-md bg-muted px-2 py-1">
                <div class="text-[10px] text-muted-foreground">光圈</div>
                <div class="truncate font-mono-num text-[13px]">{{ focused.fNumber ?? DASH }}</div>
              </div>
            </div>
            <div class="flex gap-2">
              <div class="flex-1 rounded-md bg-muted px-2 py-1">
                <div class="text-[10px] text-muted-foreground">快门</div>
                <div class="truncate font-mono-num text-[13px]">{{ focused.exposureTime ?? DASH }}</div>
              </div>
              <div class="flex-1 rounded-md bg-muted px-2 py-1">
                <div class="text-[10px] text-muted-foreground">ISO</div>
                <div class="truncate font-mono-num text-[13px]">{{ focused.iso ?? DASH }}</div>
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
          </template>
          <div v-else class="text-xs text-muted-foreground">未选择图片</div>
        </div>

        <!-- ── 评分卡：1–5 星点选 + 清除 ── -->
        <div class="flex flex-col gap-2 rounded-md border border-border bg-card p-3">
          <div class="flex items-center justify-between">
            <span class="text-xs font-medium text-muted-foreground">评分</span>
            <button
              v-if="focused && ratingToNumber(focused.rating) > 0"
              type="button"
              class="cursor-pointer text-xs text-primary hover:underline"
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
                class="cursor-pointer text-lg leading-none transition-colors select-none"
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
        <div class="flex flex-col gap-2 rounded-md border border-border bg-card p-3">
          <div class="flex items-center justify-between">
            <span class="text-xs font-medium text-muted-foreground">颜色标签</span>
            <button
              v-if="focused && focused.colorLabel !== 'None'"
              type="button"
              class="cursor-pointer text-xs text-primary hover:underline"
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
                class="size-5 cursor-pointer rounded-full border-2 transition-colors select-none"
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
                class="flex size-5 cursor-pointer items-center justify-center rounded-full border-2 bg-muted text-muted-foreground select-none"
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
        <div class="flex flex-col gap-2 rounded-md border border-border bg-card p-3">
          <span class="text-xs font-medium text-muted-foreground">旗标</span>
          <template v-if="focused">
            <div class="flex items-center gap-1">
              <button
                v-for="o in FLAG_OPTIONS"
                :key="o.label"
                type="button"
                class="flex-1 cursor-pointer rounded-sm border px-2 py-1 text-xs transition-colors select-none"
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

        <!-- ── 识别卡：五态 chip + 重新识别/检测框按钮（Phase 3 占位）── -->
        <div class="flex flex-col gap-2 rounded-md border border-border bg-card p-3">
          <span class="text-xs font-medium text-muted-foreground">识别</span>
          <!-- 未选择图片 -->
          <div v-if="!focused" class="flex flex-col gap-2">
            <div class="text-xs text-muted-foreground">未选择图片</div>
            <Button size="sm" disabled>
              <ScanSearchIcon data-icon="inline-start" />
              识别此照片
            </Button>
          </div>
          <!-- 未识别（无记录） -->
          <div v-else-if="!focused.recognitionStatus" class="flex flex-col gap-2">
            <div class="text-xs text-muted-foreground">尚未识别</div>
            <Button size="sm" @click="onRecognize">
              <ScanSearchIcon data-icon="inline-start" />
              识别此照片 (b)
            </Button>
          </div>
          <!-- 有识别记录 -->
          <template v-else>
            <div
              class="flex items-center justify-between gap-2"
            >
              <span
                class="rounded-sm border px-2 py-0.5 text-[11px] select-none"
                :class="STATUS_META[focused.recognitionStatus].cls"
              >
                {{ STATUS_META[focused.recognitionStatus].label }}
              </span>
              <span
                v-if="focused.recognitionStatus === 'Confirmed' && focused.birdConfidence !== null"
                class="font-mono-num text-[15px] font-semibold"
                :class="{
                  'text-label-green': confPercent(focused.birdConfidence) >= 80,
                  'text-label-yellow': confPercent(focused.birdConfidence) >= 50 && confPercent(focused.birdConfidence) < 80,
                  'text-label-blue': confPercent(focused.birdConfidence) < 50,
                }"
              >
                {{ confPercent(focused.birdConfidence).toFixed(1) }}%
              </span>
            </div>
            <!-- 已确认：鸟名 + 置信度条 -->
            <template v-if="focused.recognitionStatus === 'Confirmed'">
              <div v-if="focused.birdName" class="truncate text-xs font-medium text-primary">
                {{ focused.birdName }}
              </div>
              <div v-if="focused.birdConfidence !== null" class="h-1 w-full overflow-hidden rounded-full bg-muted">
                <div
                  class="h-full rounded-full transition-[width]"
                  :class="confBarCls(focused.birdConfidence)"
                  :style="{ width: `${confPercent(focused.birdConfidence)}%` }"
                />
              </div>
            </template>
            <!-- 待复核：提示文案 -->
            <div v-if="focused.recognitionStatus === 'NeedsReview'" class="text-xs text-muted-foreground">
              识别结果需人工复核
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
          </template>
        </div>
      </template>

      <!-- ════════════ 调整 tab ════════════ -->
      <template v-else>
        <div class="flex flex-col gap-2 rounded-md border border-border bg-card p-3">
          <div class="flex items-center justify-between">
            <span class="text-xs font-medium text-muted-foreground">基础调整</span>
            <button
              v-if="!isNeutral"
              type="button"
              class="cursor-pointer text-xs text-primary hover:underline"
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
                class="h-4 w-full cursor-pointer accent-primary"
                min="-2"
                max="2"
                step="0.05"
                :value="adj.exposure"
                aria-label="曝光"
                @input="onSliderInput('exposure', $event)"
              />
              <span class="w-16 shrink-0 text-right font-mono-num text-xs" :class="valueCls(adj.exposure)">
                {{ fmtExposure(adj.exposure) }}
              </span>
              <button
                v-if="adj.exposure !== 0"
                type="button"
                class="w-10 shrink-0 cursor-pointer text-right text-xs text-primary hover:underline"
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
                class="h-4 w-full cursor-pointer accent-primary"
                min="-100"
                max="100"
                step="1"
                :value="adj.contrast"
                aria-label="对比度"
                @input="onSliderInput('contrast', $event)"
              />
              <span class="w-16 shrink-0 text-right font-mono-num text-xs" :class="valueCls(adj.contrast)">
                {{ fmtSigned(adj.contrast) }}
              </span>
              <button
                v-if="adj.contrast !== 0"
                type="button"
                class="w-10 shrink-0 cursor-pointer text-right text-xs text-primary hover:underline"
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
                class="h-4 w-full cursor-pointer accent-primary"
                min="-100"
                max="100"
                step="1"
                :value="adj.saturation"
                aria-label="饱和度"
                @input="onSliderInput('saturation', $event)"
              />
              <span class="w-16 shrink-0 text-right font-mono-num text-xs" :class="valueCls(adj.saturation)">
                {{ fmtSigned(adj.saturation) }}
              </span>
              <button
                v-if="adj.saturation !== 0"
                type="button"
                class="w-10 shrink-0 cursor-pointer text-right text-xs text-primary hover:underline"
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
