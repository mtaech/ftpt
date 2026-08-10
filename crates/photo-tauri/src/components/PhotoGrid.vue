<script setup lang="ts">
// 网格：固定 4 列 + 行级虚拟化（绝对定位行，渲染可见行 ± 2 行缓冲，cell 高约 240px）。
// cell = 缩略图 + 格式徽标(左上) + 旗标角标(右上) + 文件名/大小/星级 + 鸟种状态 chip + 色标条(底缘)。
// 选择交互：单击 select、Ctrl+单击 toggle、Shift+单击 selectRange、双击进预览。
// thumb:ready → store 版本号递增 → img src ?v= 刷新。
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { CheckIcon, XIcon } from '@lucide/vue'
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

const captures = useCapturesStore()
const filter = useFilterStore()
const selection = useSelectionStore()
const preview = usePreviewStore()
const contextMenu = useContextMenuStore()
const config = useConfigStore()

/** 网格布局常量：4 列；行高跟随后端配置 thumbnailSize（cell = thumbnailSize + 56，对齐 GPUI grid.rs cell_size），行距 gap-1.5（6px）、容器内边距 6px */
const COLS = 4
const ROW_HEIGHT = computed(() => config.rowHeight)
const ROW_GAP = 6
const ROW_STEP = computed(() => ROW_HEIGHT.value + ROW_GAP)
const PAD = 6
/** 可见行窗口的缓冲行数（拖动滚动条时即将进入视口的行提前就绪） */
const BUFFER_ROWS = 2

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

/** 显示序：过滤+排序后的 captures.items 下标（对齐 Rust display_order），缓存避免重复计算 */
const displayIndices = computed(() => filter.filteredIndices)

const rowCount = computed(() => Math.ceil(displayIndices.value.length / COLS))
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

/** 行下标 → 该行实际存在的 cell（captures.items 下标；末行可能不满 4 个） */
function rowCells(r: number): number[] {
  const end = Math.min((r + 1) * COLS, displayIndices.value.length)
  const out: number[] = []
  for (let p = r * COLS; p < end; p++) out.push(displayIndices.value[p])
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

/** cell 选中态：选中集高亮，锚点项额外 ring 强调 */
function cellClass(i: number): string {
  if (!selection.isSelected(i)) return 'border-transparent hover:border-border'
  if (selection.anchorIndex === i) return 'border-primary bg-primary/10 ring-2 ring-primary'
  return 'border-primary bg-primary/5 ring-1 ring-primary'
}

/** 鸟种 chip 文本：已确认 → 鸟名 + 置信度（mock 层为 0–1 小数、真实后端 0–100，统一归一化） */
function birdText(c: CaptureMeta): string {
  const conf = c.birdConfidence
  const name = c.birdName ?? '未知'
  if (conf === null) return name
  const pct = conf <= 1 ? conf * 100 : conf
  return `${name} · ${pct.toFixed(1)}%`
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
    const pos = displayIndices.value.indexOf(i)
    if (pos < 0) return
    const rowTop = PAD + Math.floor(pos / COLS) * ROW_STEP.value
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
      v-else-if="displayIndices.length === 0 && !captures.scanning"
      class="flex h-full items-center justify-center text-sm text-muted-foreground"
    >
      没有符合筛选条件的照片
    </div>

    <!-- 虚拟化行容器：撑出滚动高度，可见行窗口内绝对定位渲染 -->
    <div v-else class="relative" :style="{ height: spacerH + 'px' }">
      <div
        v-for="r in visibleRowList"
        :key="r"
        class="absolute inset-x-0 grid grid-cols-4 gap-1.5 px-1.5"
        :style="{ top: PAD + r * ROW_STEP + 'px', height: ROW_HEIGHT + 'px' }"
      >
        <div
          v-for="i in rowCells(r)"
          :key="captures.items[i].primaryPath"
          :data-grid-cell="i"
          class="group relative flex cursor-pointer flex-col overflow-hidden rounded-md border bg-card select-none"
          :class="cellClass(i)"
          :style="{ contentVisibility: 'auto', containIntrinsicSize: 'auto ' + ROW_HEIGHT + 'px' }"
          @click="onCellClick(i, $event)"
          @dblclick="onCellDblClick(i)"
          @contextmenu.prevent="onCellContextMenu(i, $event)"
        >
          <!-- 缩略图区（占满剩余高度，作为徽标/旗标定位容器） -->
          <div class="relative min-h-0 flex-1 overflow-hidden bg-muted">
            <!-- 非图片格式（OTHER）：无缩略图，居中显示格式徽标 -->
            <div v-if="isOtherFormat(captures.items[i])" class="flex size-full items-center justify-center">
              <span
                class="rounded border border-border bg-card px-2 py-0.5 text-xs font-medium text-primary"
              >
                {{ formatBadgeLabel(captures.items[i]) }}
              </span>
            </div>
            <img
              v-else
              :src="thumbSrc(captures.items[i])"
              :alt="displayName(captures.items[i])"
              loading="lazy"
              draggable="false"
              class="size-full object-cover"
            />

            <!-- 格式徽标（左上，半透明黑底，对齐 GPUI BADGE_BG） -->
            <span
              class="absolute top-1 left-1 rounded-sm bg-black/70 px-1 text-[10px] leading-4 text-white"
            >
              {{ formatBadgeLabel(captures.items[i]) }}
            </span>

            <!-- 旗标角标（右上，半透明黑底） -->
            <div
              v-if="captures.items[i].flag"
              class="absolute top-1 right-1 rounded bg-black/70 p-0.5"
              :class="
                captures.items[i].flag === 'Pick' ? 'text-pick' : 'text-reject'
              "
            >
              <CheckIcon v-if="captures.items[i].flag === 'Pick'" class="size-3" />
              <XIcon v-else class="size-3" />
            </div>
          </div>

          <!-- 信息区：文件名 / 大小+星级 / 鸟种状态 chip（常驻等高占位） -->
          <div class="shrink-0 space-y-0.5 px-1.5 py-1">
            <div class="flex items-center gap-1">
              <span
                class="min-w-0 flex-1 truncate text-xs"
                :title="displayName(captures.items[i])"
              >
                {{ displayName(captures.items[i]) }}
              </span>
            </div>
            <div class="flex items-center justify-between">
              <span class="text-[10px] text-muted-foreground font-mono-num">
                {{ formatBytes(captures.items[i].fileSize) }}
              </span>
              <span
                v-if="ratingToNumber(captures.items[i].rating) > 0"
                class="shrink-0 text-[10px] text-rating"
              >
                {{ '★'.repeat(ratingToNumber(captures.items[i].rating)) }}
              </span>
            </div>
            <!-- 鸟种状态 chip：Confirmed 绿 / NeedsReview 黄 / Unrecognized 灰；无记录空行占位 -->
            <div class="flex h-[18px] items-center">
              <span
                v-if="captures.items[i].recognitionStatus === 'Confirmed'"
                class="max-w-full truncate rounded bg-label-green/10 px-1 text-[10px] text-label-green"
              >
                {{ birdText(captures.items[i]) }}
              </span>
              <span
                v-else-if="captures.items[i].recognitionStatus === 'NeedsReview'"
                class="rounded bg-label-yellow/10 px-1 text-[10px] text-label-yellow"
              >
                待复核
              </span>
              <span
                v-else-if="captures.items[i].recognitionStatus === 'Unrecognized'"
                class="rounded bg-muted px-1 text-[10px] text-muted-foreground"
              >
                未检测到鸟类
              </span>
            </div>
          </div>

          <!-- 色标条（cell 底缘 3px） -->
          <div
            v-if="captures.items[i].colorLabel !== 'None'"
            class="absolute inset-x-0 bottom-0 h-[3px]"
            :class="LABEL_BAR[captures.items[i].colorLabel]"
          />
        </div>
      </div>
    </div>
  </div>
</template>
