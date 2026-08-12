<script setup lang="ts">
// 网格：动态列数 + 行级虚拟化（绝对定位行，渲染可见行 ± 2 行缓冲，cell 高约 240px）。
// 列数 = 容器宽 ÷ cell 宽（thumbnailSize + gap），实时跟随缩略图尺寸滑块与窗口宽度；
// 行高跟随后端配置 thumbnailSize（cell = thumbnailSize + 56，对齐 GPUI grid.rs cell_size）。
// cell = 缩略图 + 格式徽标(左上) + 旗标角标(右上) + 文件名/大小/星级 + 鸟种状态 chip + 色标条(底缘)。
// 选择交互：单击 select、Ctrl+单击 toggle、Shift+单击 selectRange、双击进预览。
// thumb:ready → store 版本号递增 → img src ?v= 刷新。
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { CheckIcon, LayersIcon, XIcon } from '@lucide/vue'
import { useCapturesStore } from '@/stores/captures'
import { useFilterStore } from '@/stores/filter'
import { useSelectionStore } from '@/stores/selection'
import { usePreviewStore } from '@/stores/preview'
import { useContextMenuStore, captureMenuItems } from '@/stores/contextMenu'
import { useConfigStore } from '@/stores/config'
import { ptimgUrl } from '@/lib/ipc'
import {
  displayName,
  formatBadgeLabel,
  formatBytes,
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

/** 网格布局常量：行高跟随后端配置 thumbnailSize（cell = thumbnailSize + 56，对齐 GPUI grid.rs cell_size），
 * 行距 8px、容器内边距 4px（对齐 GPUI p_1；值与模板 gap-[8px]/px-[4px] 保持一致，保证滚动定位精确） */
const ROW_HEIGHT = computed(() => config.rowHeight)
const ROW_GAP = 8
const ROW_STEP = computed(() => ROW_HEIGHT.value + ROW_GAP)
const PAD = 4
/** 可见行窗口的缓冲行数（拖动滚动条时即将进入视口的行提前就绪） */
const BUFFER_ROWS = 2
/** 网格容器可视宽度（ResizeObserver 测量，列数计算依据） */
const gridWidth = ref(0)
/**
 * 动态列数：容器宽 ÷ cell 宽（thumbnailSize + gap），下限 1；未测量时回退 4
 * （对齐原固定 4 列）。缩略图尺寸滑块/窗口宽度变化即时生效。
 */
const COLS = computed(() => {
  const w = gridWidth.value
  if (w <= 0) return 4
  return Math.max(1, Math.floor((w + ROW_GAP) / (config.thumbnailSize + ROW_GAP)))
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

/** cell 选中态：选中集高亮（对齐 GPUI：2px primary 边框 + element-hover 底），锚点项额外 ring 供范围选择定位；
 * 未选中 = 透明边框占位（border-2 防跳动），悬停显示弱边框 */
function cellClass(i: number): string {
  if (!selection.isSelected(i)) return 'border-transparent hover:border-border'
  if (selection.anchorIndex === i) return 'border-primary bg-element-hover ring-1 ring-primary/60'
  return 'border-primary bg-element-hover'
}

/** 连拍组信息（filter store 按显示序分组，key = captures.items 下标；仅 size≥2 的组） */
function burstOf(i: number) {
  return filter.burstGroups.get(i)
}

/**
 * 堆叠徽标 tooltip：成员格式列表（「点击切换格式」提示）。
 * 单成员组返回 undefined（无堆叠 UI）。
 */
function stackTitle(g: StackGroup): string | undefined {
  if (g.members.length < 2) return undefined
  const names = g.members.map((i) => displayName(captures.items[i])).join(' / ')
  return `${names}（点击切换格式）`
}

/** 堆叠徽标点击：循环切换该组激活成员并选中（±1；单成员组 no-op） */
function cycleStackOf(g: StackGroup) {
  selection.cycleStackFrom(g.active, 1)
}

/** 五星逐位彩虹色（GPUI RATING 数组原值）：第 i 颗已填星颜色，仅着色用 */
const RATING_COLORS = ['#ef4444', '#f97316', '#e8ab07', '#22c55e', '#3b82f6']

/** 置信度归一化到 0–100（mock 层为 0–1 小数、真实后端 0–100），无置信度返回 null */
function confPct(conf: number | null): number | null {
  if (conf === null) return null
  return conf <= 1 ? conf * 100 : conf
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

            <!-- 格式徽标（左上，半透明黑底，对齐 GPUI BADGE_BG） -->
            <span
              class="absolute top-1 left-1 rounded-sm bg-black/70 px-1 text-[10px] leading-4 text-white"
            >
              {{ formatBadgeLabel(cell.c) }}
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

            <!-- 连拍组徽标（左下，半透明黑底，对齐格式/旗标徽标风格；仅 size≥2 显示） -->
            <div
              v-if="burstOf(cell.g.active)"
              class="absolute bottom-1 left-1 flex items-center gap-0.5 rounded-sm bg-black/70 px-1 text-[0.625rem] leading-4 text-white"
            >
              <LayersIcon class="size-3" />
              <span class="tabular-nums">{{ burstOf(cell.g.active)!.size }}</span>
            </div>

            <!-- 堆叠徽标（右下，仅同 stem 多成员组；点击循环切换激活格式并选中） -->
            <button
              v-if="cell.g.members.length > 1"
              class="absolute right-1 bottom-1 cursor-pointer rounded-sm bg-black/70 px-1 text-[10px] leading-4 text-white transition-colors hover:bg-black/90"
              :title="stackTitle(cell.g)"
              @click.stop="cycleStackOf(cell.g)"
            >
              ×{{ cell.g.members.length }}
            </button>
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
              <span class="font-mono text-xs text-muted-foreground tabular-nums">
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
                  class="shrink-0 font-mono text-muted-foreground tabular-nums"
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
