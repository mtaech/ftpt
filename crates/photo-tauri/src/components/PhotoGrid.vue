<script setup lang="ts">
// 网格：动态列数 + 行级虚拟化（绝对定位行，渲染可见行 ± 2 行缓冲，cell 高约 240px）。
// 列数 = 容器宽 ÷ cell 宽（thumbnailSize + gap），实时跟随缩略图尺寸滑块与窗口宽度；
// 行高跟随后端配置 thumbnailSize（cell = thumbnailSize + 56，对齐 GPUI grid.rs cell_size）。
// cell = 缩略图 + 格式徽标(左上) + 旗标角标(右上) + 堆叠成员带(底部，多成员组：成员缩略图+语义徽标) + 文件名/大小/星级 + 鸟种状态 chip + 色标条(底缘)。
// 选择交互：单击 select、Ctrl+单击 toggle、Shift+单击 selectRange、双击进预览。
// thumb:ready → store 版本号递增 → img src ?v= 刷新。
import { computed, onMounted, onUnmounted, reactive, ref, watch, type Component } from 'vue'
import { CheckIcon, ChevronLeftIcon, ChevronRightIcon, CopyIcon, CrownIcon, LayersIcon, XIcon } from '@lucide/vue'
import { useCapturesStore } from '@/stores/captures'
import { useFilterStore } from '@/stores/filter'
import { useSelectionStore } from '@/stores/selection'
import { usePreviewStore } from '@/stores/preview'
import { useContextMenuStore, captureMenuItems } from '@/stores/contextMenu'
import { useConfigStore } from '@/stores/config'
import { useQualityStore } from '@/stores/quality'
import { ptimgUrl } from '@/lib/ipc'
import { pickBestFrame } from '@/lib/bestFrame'
import {
  displayName,
  formatBadgeLabel,
  formatBytes,
  formatName,
  isOtherFormat,
  ratingToNumber,
} from '@/lib/format'
import type { CaptureMeta, ColorLabel } from '@/lib/bindings'
import type { StackGroup } from '@/lib/stacks'

const captures = useCapturesStore()
const filter = useFilterStore()
const selection = useSelectionStore()
const preview = usePreviewStore()
const contextMenu = useContextMenuStore()
const config = useConfigStore()
const quality = useQualityStore()

/** 网格布局常量：行高跟随后端配置 thumbnailSize（cell = thumbnailSize + 56，对齐 GPUI grid.rs cell_size），
 * 行距 8px、容器内边距 4px（对齐 GPUI p_1；值与模板 gap-[8px]/px-[4px] 保持一致，保证滚动定位精确） */
const ROW_HEIGHT = computed(() =>
  gridWidth.value > 0 ? Math.max(80, Math.round(cellW.value) + 56) : config.rowHeight,
)
const ROW_GAP = 8
const ROW_STEP = computed(() => ROW_HEIGHT.value + ROW_GAP)
const PAD = 4
/** 可见行窗口的缓冲行数（拖动滚动条时即将进入视口的行提前就绪） */
const BUFFER_ROWS = 2
/** 网格容器可视宽度（ResizeObserver 测量，列数计算依据） */
const gridWidth = ref(0)
/**
 * 固定列数 = 配置的每行图片数（2-5，下拉栏选择）。cell 宽由容器宽 ÷ 列数
 * 自适应，行高 = cell 宽 + 56（缩略图正方形，对齐 GPUI cell_size 公式）。
 * 容器未测量（宽 0）时按默认 4 列 + thumbnailSize 估算行高。
 */
const COLS = computed(() => config.gridColumns)
/** cell 宽（px）：容器宽扣除内边距与列间距后均分 */
const cellW = computed(() => {
  const w = gridWidth.value
  if (w <= 0) return (config.thumbnailSize ?? 220) + 56 - 56
  return (w - PAD * 2 - (COLS.value - 1) * ROW_GAP) / COLS.value
})

/** 色标条颜色（取 theme.rs LABEL_* 原值，经 @theme 注册为 bg-label-*） */
const LABEL_BAR: Record<Exclude<ColorLabel, 'None'>, string> = {
  Red: 'bg-label-red',
  Yellow: 'bg-label-yellow',
  Green: 'bg-label-green',
  Blue: 'bg-label-blue',
  Purple: 'bg-label-purple',
}

const scrollEl = ref<HTMLElement | null>(null)
const scrollTop = ref(0)
const viewportH = ref(0)

/** 显示堆叠组：filteredIndices 按 stem 分组（同 stem 文件合并为一个网格项，见 stacks.ts） */
const stackGroups = computed(() => filter.stackGroups)

/** 网格 cell 视图模型：堆叠组 + 其激活成员（模板消费 c 免去重复下标解引用） */
interface GridCell {
  g: StackGroup
  c: CaptureMeta
}

const rowCount = computed(() => Math.ceil(stackGroups.value.length / COLS.value))
/** 撑出滚动高度的占位容器（rows 绝对定位在其中） */
const spacerH = computed(() =>
  rowCount.value === 0 ? 0 : PAD * 2 + rowCount.value * ROW_STEP.value - ROW_GAP,
)

/** 可见行窗口：可见行 ± 2 行缓冲，钳制到 [0, rowCount) */
const visibleRowList = computed(() => {
  const rows = rowCount.value
  if (rows === 0) return []
  const first = Math.max(0, Math.floor(scrollTop.value / ROW_STEP.value) - BUFFER_ROWS)
  const last = Math.min(rows, Math.ceil((scrollTop.value + viewportH.value) / ROW_STEP.value) + BUFFER_ROWS)
  const out: number[] = []
  for (let r = first; r < last; r++) out.push(r)
  return out
})

/** 行下标 → 该行实际存在的 cell（堆叠组；末行可能不满 COLS 个） */
function rowCells(r: number): GridCell[] {
  const end = Math.min((r + 1) * COLS.value, stackGroups.value.length)
  const out: GridCell[] = []
  for (let p = r * COLS.value; p < end; p++) {
    const g = stackGroups.value[p]
    out.push({ g, c: captures.items[g.active] })
  }
  return out
}

function onScroll() {
  const el = scrollEl.value
  if (!el) return
  // 内容收缩（筛选/重扫）后钳制滚动位置
  const max = Math.max(0, spacerH.value - el.clientHeight)
  scrollTop.value = Math.min(el.scrollTop, max)
}

function measure() {
  const el = scrollEl.value
  if (el) {
    viewportH.value = el.clientHeight
    gridWidth.value = el.clientWidth
    // 组件重挂载（HMR/切视图）后元素可能保留原滚动位置，同步到 ref 保证渲染窗口一致
    scrollTop.value = el.scrollTop
  }
}

let ro: ResizeObserver | null = null
onMounted(() => {
  measure()
  ro = new ResizeObserver(measure)
  if (scrollEl.value) ro.observe(scrollEl.value)
})
onUnmounted(() => ro?.disconnect())

function thumbSrc(c: CaptureMeta): string {
  return ptimgUrl('thumb', c.primaryPath, captures.thumbVersions[c.primaryPath])
}

/** cell 选中态：选中集高亮。用 ring + 2px offset（box-shadow 不占布局，替代原 border-2 直描）——
 *  选中框与 cell 边缘、相邻 cell 之间留出呼吸缝，不再紧贴旁边图片；
 *  锚点项满色 ring 供范围选择定位，其余选中项 70% 透明度区分层级 */
function cellClass(i: number): string {
  if (!selection.isSelected(i)) return 'border-transparent hover:border-border'
  if (selection.anchorIndex === i)
    return 'border-transparent bg-element-hover ring-2 ring-primary ring-offset-2 ring-offset-background'
  return 'border-transparent bg-element-hover ring-2 ring-primary/70 ring-offset-2 ring-offset-background'
}

/** 连拍组信息（filter store 按显示序分组，key = captures.items 下标；仅 size≥2 的组） */
function burstOf(i: number) {
  return filter.burstGroups.get(i)
}

/**
 * 连拍组最优帧下标集合（captures.items 下标；仅 size≥2 组）：驱动最优帧皇冠徽标。
 * 每组独立选优（pickBestFrame 确定性全序），无锐度信息时按文件大小/路径序兜底。
 */
const bestFrameIndices = computed(() => {
  const best = new Set<number>()
  const byGroup = new Map<string, number[]>()
  for (const [i, e] of filter.burstGroups) {
    const arr = byGroup.get(e.groupId)
    if (arr) arr.push(i)
    else byGroup.set(e.groupId, [i])
  }
  for (const members of byGroup.values()) {
    const bestPath = pickBestFrame(members.map((i) => captures.items[i]))
    if (bestPath === null) continue
    const bestIdx = members.find((i) => captures.items[i].primaryPath === bestPath)
    if (bestIdx !== undefined) best.add(bestIdx)
  }
  return best
})

/**
 * 堆叠语义：按组内成员格式多样性区分两种堆叠——
 * 多格式 = 同画面（JPG/RAW 等，Copy 图标 + 蓝）；单格式 = 连拍多帧（Layers 图标 + 橙）。
 */
function stackSemantic(g: StackGroup): { icon: Component; cls: string; label: string; multiFormat: boolean } {
  const fmts = new Set(g.members.map((i) => captures.items[i].primaryFormat))
  if (fmts.size > 1) return { icon: CopyIcon, cls: 'text-sky-300', label: '同画面多格式', multiFormat: true }
  return { icon: LayersIcon, cls: 'text-amber-300', label: '连拍', multiFormat: false }
}

/** 堆叠语义徽标 tooltip：语义 + 成员数（成员细节见各缩略图 title） */
function stackTitle(g: StackGroup): string {
  const s = stackSemantic(g)
  return `${s.label}（${g.members.length} 张）`
}

/** 成员缩略图点击：直达激活该成员并选中（替代原 ×N 循环点击） */
function activateStackMember(g: StackGroup, m: number) {
  if (m === g.active) return
  filter.setStackActive(g.key, m)
  selection.select(m)
}

/** 成员带滚轮横滚：纵向滚轮转横向滚动（无需 Shift；prevent 阻止冒泡触发网格整体滚动） */
function onStripWheel(e: WheelEvent) {
  const el = e.currentTarget as HTMLElement
  el.scrollLeft += e.deltaY !== 0 ? e.deltaY : e.deltaX
}

/** 成员带滚动状态：组 key → 可否向左/右滚（驱动两端半透明按钮显隐；默认右可滚） */
const stripScroll = reactive<Record<string, { l: boolean; r: boolean }>>({})

/** 成员带滚动事件：刷新两端按钮显隐（1px 容差吸收小数滚动值） */
function onStripScroll(key: string, e: Event) {
  const el = e.currentTarget as HTMLElement
  stripScroll[key] = { l: el.scrollLeft > 1, r: el.scrollLeft + el.clientWidth < el.scrollWidth - 1 }
}

/** 左右按钮点击：平滑滚动 3 个缩略图位（32px/位）；经 data-strip-scroll 找同带滚动容器 */
function scrollStrip(e: MouseEvent, dir: 1 | -1) {
  const strip = (e.currentTarget as HTMLElement).parentElement?.querySelector<HTMLElement>('[data-strip-scroll]')
  if (!strip) return
  strip.scrollBy({ left: dir * 96, behavior: 'smooth' })
}

/** 五星逐位彩虹色（GPUI RATING 数组原值）：第 i 颗已填星颜色，仅着色用 */
const RATING_COLORS = ['#ef4444', '#f97316', '#e8ab07', '#22c55e', '#3b82f6']

/** 置信度归一化到 0–100（mock 层为 0–1 小数、真实后端 0–100），无置信度返回 null */
function confPct(conf: number | null): number | null {
  if (conf === null) return null
  return conf <= 1 ? conf * 100 : conf
}

// ── 技术质量机筛分角标（QualityScore 批次）：分档阈值常量顶置 ──
/** 技术分 ≥ 此值 = 绿点（优） */
const QUALITY_GOOD = 0.75
/** 技术分 < 此值 = 红点（劣）；中间档不显示角标 */
const QUALITY_BAD = 0.4

/** 质量角标：≥0.75 绿点 / <0.4 红点；未评分（null）不显示。
 *  返回 null 时模板不渲染（与旗标/连拍徽标互不冲突，占位右下角） */
function qualityDot(path: string): { cls: string; title: string } | null {
  const s = quality.scoreOf(path)
  if (s === null) return null
  const title = `技术分 ${s.toFixed(2)}`
  if (s >= QUALITY_GOOD) return { cls: 'bg-green-500', title }
  if (s < QUALITY_BAD) return { cls: 'bg-red-500', title }
  return null
}

/** Confirmed 鸟名着色（GPUI 置信度三档）：≥80 success / ≥50 warning / 否则 primary（无置信度回退 primary） */
function confColor(pct: number | null): string {
  if (pct === null) return 'text-primary'
  return pct >= 80 ? 'text-success' : pct >= 50 ? 'text-warning' : 'text-primary'
}

function onCellClick(i: number, e: MouseEvent) {
  if (e.ctrlKey || e.metaKey) selection.toggle(i)
  else if (e.shiftKey) selection.selectRange(i)
  else selection.select(i)
}

function onCellDblClick(i: number) {
  selection.select(i)
  preview.openPreview()
}

/**
 * 右键菜单：先聚焦到被点项（不在多选中则独占选中、已在多选中保持多选，
 * 对齐 GPUI focus_for_context_menu），再弹出图片菜单（动作作用于选中集）。
 */
function onCellContextMenu(i: number, e: MouseEvent) {
  if (!selection.isSelected(i)) selection.select(i)
  contextMenu.openMenu(
    captureMenuItems({
      meta: captures.items[i] ?? null,
      inPreview: false,
      selectedCount: selection.selectedIndices.length,
      paths: selection.selectedPaths,
      onToggleView: () => preview.openPreview(),
    }),
    e.clientX,
    e.clientY,
  )
}

// 方向键移动选中后滚动到可见区。虚拟化下目标 cell 可能尚未渲染，
// 不依赖 scrollIntoView，直接按显示位（filteredIndices 中的位置）计算目标 scrollTop。
// 原生 scroll 事件在部分环境（无头/隐藏页）对程序化滚动不可靠，故同步写 scrollTop ref。
watch(
  () => selection.selectedIndex,
  (i) => {
    if (i === null) return
    const el = scrollEl.value
    if (!el) return
    // 选中项不在显示序中（被筛选掉）时不滚动
    const pos = filter.stackPositionOf(i)
    if (pos < 0) return
    const rowTop = PAD + Math.floor(pos / COLS.value) * ROW_STEP.value
    const rowBottom = rowTop + ROW_HEIGHT.value
    let target = el.scrollTop
    if (rowTop < el.scrollTop) {
      target = rowTop
    } else if (rowBottom > el.scrollTop + el.clientHeight) {
      target = rowBottom - el.clientHeight
    }
    if (target !== el.scrollTop) {
      el.scrollTop = target
      scrollTop.value = target
    }
  },
)

// 内容行数/行高变化（筛选/重扫/设置缩略图尺寸）导致滚动位置越界时收敛（同步 ref，见上注释）
watch([rowCount, ROW_STEP], () => {
  const el = scrollEl.value
  if (!el) return
  const max = Math.max(0, spacerH.value - el.clientHeight)
  if (el.scrollTop > max) {
    el.scrollTop = max
    scrollTop.value = max
  }
})
</script>

<template>
  <div ref="scrollEl" class="relative h-full overflow-y-auto" @scroll="onScroll">
    <!-- 空态：未打开目录 -->
    <div
      v-if="captures.count === 0 && !captures.scanning"
      class="flex h-full items-center justify-center text-sm text-muted-foreground"
    >
      点击左上角「打开目录」开始浏览
    </div>

    <!-- 空态：筛选无匹配项 -->
    <div
      v-else-if="stackGroups.length === 0 && !captures.scanning"
      class="flex h-full items-center justify-center text-sm text-muted-foreground"
    >
      没有符合筛选条件的照片
    </div>

    <!-- 虚拟化行容器：撑出滚动高度，可见行窗口内绝对定位渲染 -->
    <div v-else class="relative" :style="{ height: spacerH + 'px' }">
      <div
        v-for="r in visibleRowList"
        :key="r"
        class="absolute inset-x-0 grid gap-[8px] px-[4px]"
        :style="{
          top: PAD + r * ROW_STEP + 'px',
          height: ROW_HEIGHT + 'px',
          gridTemplateColumns: 'repeat(' + COLS + ', minmax(0, 1fr))',
        }"
      >
        <div
          v-for="cell in rowCells(r)"
          :key="cell.g.key"
          :data-grid-cell="cell.g.active"
          class="group relative flex flex-col overflow-hidden rounded-md border-2 bg-card transition-colors select-none"
          :class="cellClass(cell.g.active)"
          :style="{ contentVisibility: 'auto', containIntrinsicSize: 'auto ' + ROW_HEIGHT + 'px' }"
          @click="onCellClick(cell.g.active, $event)"
          @dblclick="onCellDblClick(cell.g.active)"
          @contextmenu.prevent="onCellContextMenu(cell.g.active, $event)"
        >
          <!-- 缩略图区（占满剩余高度，作为徽标/旗标定位容器） -->
          <div class="relative min-h-0 flex-1 overflow-hidden bg-element">
            <!-- 非图片格式（OTHER）：无缩略图，居中显示格式徽标 -->
            <div v-if="isOtherFormat(cell.c)" class="flex size-full items-center justify-center">
              <span
                class="rounded border border-border bg-card px-2 py-0.5 text-xs font-medium text-primary"
              >
                {{ formatBadgeLabel(cell.c) }}
              </span>
            </div>
            <img
              v-else
              :src="thumbSrc(cell.c)"
              :alt="displayName(cell.c)"
              loading="lazy"
              draggable="false"
              class="size-full object-cover"
            />

            <!-- 格式徽标（左上，半透明黑底，RAW 直读扩展名，对齐 GPUI BADGE_BG） -->
            <span
              class="absolute top-1 left-1 rounded-sm bg-black/70 px-1 text-[10px] leading-4 text-white"
            >
              {{ formatName(cell.c) }}
            </span>

            <!-- 旗标角标（右上 18px 圆，Pick 绿底白勾 / Reject 红底白叉，对齐 GPUI flag 覆层） -->
            <div
              v-if="cell.c.flag"
              class="absolute top-1 right-1 flex size-[18px] items-center justify-center rounded-full"
              :class="
                cell.c.flag === 'Pick' ? 'bg-pick' : 'bg-reject'
              "
            >
              <CheckIcon v-if="cell.c.flag === 'Pick'" class="size-3 text-white" />
              <XIcon v-else class="size-3 text-white" />
            </div>

            <!-- 技术质量分角标（右下 8px 圆点：≥0.75 绿 / <0.4 红，中档与未评分不显示；
                 阈值常量见脚本 QUALITY_GOOD/QUALITY_BAD） -->
            <div
              v-if="qualityDot(cell.c.primaryPath)"
              class="absolute right-1 bottom-1 size-2 rounded-full ring-1 ring-black/40"
              :class="qualityDot(cell.c.primaryPath)!.cls"
              :title="qualityDot(cell.c.primaryPath)!.title"
            />

            <!-- 连拍组徽标（左下，仅单成员组：堆叠组的连拍语义由成员带+语义徽标表达，避免双徽标重复）。
                 组内最优帧在徽标行前置皇冠（amber，仅 size≥2 组存在；样式对齐 OTHER 格式徽标黑底） -->
            <div
              v-if="cell.g.members.length === 1 && burstOf(cell.g.active)"
              class="absolute bottom-1 left-1 flex items-center gap-0.5 rounded-sm bg-black/70 px-1 text-[0.625rem] leading-4 text-white"
            >
              <span
                v-if="bestFrameIndices.has(cell.g.active)"
                class="flex items-center text-amber-300"
                title="连拍组最优帧（K 键保留）"
              >
                <CrownIcon class="size-3" />
              </span>
              <LayersIcon class="size-3" />
              <span class="tabular-nums">{{ burstOf(cell.g.active)!.size }}</span>
            </div>

            <!-- 堆叠成员带（多成员组）：底部横向成员缩略图 + 语义徽标（同画面多格式/连拍）。
                 点击缩略图直达激活并选中；滚动条隐藏，成员多时两端浮出半透明左右按钮
                 （滚到头自动隐藏），滚轮直接横滚。 -->
            <div
              v-if="cell.g.members.length > 1"
              class="absolute inset-x-0 bottom-0 flex items-center gap-0.5 bg-black/60 p-0.5 backdrop-blur-[2px]"
              @dblclick.stop
            >
              <!-- 左滚按钮（半透明，滚到最左隐藏） -->
              <button
                v-if="cell.g.members.length > 4"
                class="z-10 flex size-5 shrink-0 cursor-pointer items-center justify-center rounded-full bg-black/40 text-white/80 transition-opacity hover:bg-black/70 hover:text-white"
                :class="stripScroll[cell.g.key]?.l ? 'opacity-100' : 'pointer-events-none opacity-0'"
                title="向左滚动"
                @click.stop="scrollStrip($event, -1)"
              >
                <ChevronLeftIcon class="size-3.5" />
              </button>
              <div
                data-strip-scroll
                class="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto p-[2px] [scrollbar-width:none]! [&::-webkit-scrollbar]:hidden"
                @scroll="onStripScroll(cell.g.key, $event)"
                @wheel.prevent="onStripWheel"
              >
                <button
                  v-for="m in cell.g.members"
                  :key="m"
                  class="relative size-[28px] shrink-0 cursor-pointer overflow-hidden rounded transition-all"
                  :class="
                    m === cell.g.active
                      ? 'ring-2 ring-primary'
                      : 'opacity-75 hover:opacity-100 hover:ring-1 hover:ring-white/70'
                  "
                  :title="displayName(captures.items[m])"
                  @click.stop="activateStackMember(cell.g, m)"
                >
                  <img
                    v-if="!isOtherFormat(captures.items[m])"
                    :src="thumbSrc(captures.items[m])"
                    :alt="displayName(captures.items[m])"
                    loading="lazy"
                    draggable="false"
                    class="size-full object-cover"
                  />
                  <div v-else class="flex size-full items-center justify-center bg-element">
                    <span
                      class="rounded-sm bg-card px-0.5 text-[8px] leading-3 font-medium text-primary"
                    >
                      {{ formatBadgeLabel(captures.items[m]) }}
                    </span>
                  </div>
                  <span
                    v-if="stackSemantic(cell.g).multiFormat"
                    class="absolute bottom-0 left-0 rounded-sm bg-black/70 px-0.5 text-[8px] leading-3 text-white"
                  >
                    {{ formatBadgeLabel(captures.items[m]) }}
                  </span>
                  <!-- 连拍组最优帧皇冠（右上角；仅 size≥2 组的选优帧，样式对齐语义徽标黑底） -->
                  <span
                    v-if="bestFrameIndices.has(m)"
                    class="absolute top-0 right-0 rounded-sm bg-black/70 p-0.5 text-amber-300"
                    title="连拍组最优帧（K 键保留）"
                  >
                    <CrownIcon class="size-2.5" />
                  </span>
                </button>
              </div>
              <!-- 右滚按钮（半透明，滚到最右隐藏；初始默认可滚） -->
              <button
                v-if="cell.g.members.length > 4"
                class="z-10 flex size-5 shrink-0 cursor-pointer items-center justify-center rounded-full bg-black/40 text-white/80 transition-opacity hover:bg-black/70 hover:text-white"
                :class="(stripScroll[cell.g.key]?.r ?? true) ? 'opacity-100' : 'pointer-events-none opacity-0'"
                title="向右滚动"
                @click.stop="scrollStrip($event, 1)"
              >
                <ChevronRightIcon class="size-3.5" />
              </button>
              <div
                class="flex shrink-0 items-center gap-1 self-center rounded-full bg-white/10 px-1.5 py-0.5 text-[10px] leading-3"
                :class="stackSemantic(cell.g).cls"
                :title="stackTitle(cell.g)"
              >
                <component :is="stackSemantic(cell.g).icon" class="size-3" />
                <span class="tabular-nums">{{ cell.g.members.length }}</span>
              </div>
            </div>
          </div>

          <!-- 信息区：文件名 / 大小+五星 / 鸟种状态行（常驻等高占位） -->
          <div class="shrink-0 space-y-0.5 bg-card px-2 py-1.5">
            <div class="flex items-center gap-1">
              <span
                class="min-w-0 flex-1 truncate text-[13px]"
                :title="displayName(cell.c)"
              >
                {{ displayName(cell.c) }}
              </span>
            </div>
            <div class="flex items-center justify-between">
              <span class="text-xs text-muted-foreground tabular-nums">
                {{ formatBytes(cell.c.fileSize) }}
              </span>
              <!-- 五星（GPUI 逐位彩虹色）：第 i 颗已填星 = RATING_COLORS[i]，未填星 muted -->
              <span class="flex shrink-0 items-center gap-px">
                <span
                  v-for="s in 5"
                  :key="s"
                  class="text-[10px] leading-none select-none"
                  :class="s > ratingToNumber(cell.c.rating) ? 'text-muted-foreground' : ''"
                  :style="
                    s <= ratingToNumber(cell.c.rating)
                      ? { color: RATING_COLORS[s - 1] }
                      : undefined
                  "
                >★</span>
              </span>
            </div>
            <!-- 鸟种状态行常驻 h-[18px]：Confirmed 名按置信度着色 + 右侧 mono 百分比 /
                 NeedsReview 待复核 / Unrecognized 未检测到鸟类 / 无记录空占位 -->
            <div class="flex h-[18px] items-center gap-1 text-xs">
              <template v-if="cell.c.recognitionStatus === 'Confirmed'">
                <span
                  class="min-w-0 flex-1 truncate"
                  :class="confColor(confPct(cell.c.birdConfidence))"
                >
                  {{ cell.c.birdName ?? '未知' }}
                </span>
                <span
                  v-if="confPct(cell.c.birdConfidence) !== null"
                  class="shrink-0 text-muted-foreground tabular-nums"
                >
                  {{ confPct(cell.c.birdConfidence)!.toFixed(1) }}%
                </span>
              </template>
              <span v-else-if="cell.c.recognitionStatus === 'NeedsReview'" class="text-warning">
                待复核
              </span>
              <span v-else-if="cell.c.recognitionStatus === 'Unrecognized'" class="text-muted-foreground">
                未检测到鸟类
              </span>
            </div>
          </div>

          <!-- 色标条（cell 底缘 3px） -->
          <div
            v-if="cell.c.colorLabel !== 'None'"
            class="absolute inset-x-0 bottom-0 h-[3px]"
            :class="LABEL_BAR[cell.c.colorLabel]"
          />
        </div>
      </div>
    </div>
  </div>
</template>
