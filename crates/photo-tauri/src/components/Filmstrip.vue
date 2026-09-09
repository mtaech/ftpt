<script setup lang="ts">
// 胶片条：预览视图底部横条。展示全部拍摄的 thumb 缩略图，横向滚动；
// 点击跳转选中（selection.select），当前项高亮边框，选中变化时横滚跟随。
// 缩略图走 ptimgUrl('thumb', path, v)，v 来自 captures.thumbVersions（thumb:ready 后强制刷新）。
//
// 交互约定（2026-09 修复）：
// - 采用「横向虚拟列表」：滚动容器为全内容宽度（scrollWidth 反映全部条目），
//   缩略图按滚动位置绝对定位渲染（只挂载可视区 ±BUFFER）。大目录（5000+）也能
//   随意滚到任意位置，不会因为选中没移过去而「渐进加载」缺失。
// - 横向滚动条独立放在缩略图下方（自定义 track + thumb，随滚动同步、可点击/拖拽），
//   不在图片条内（浏览器原生横向滚动条会叠在缩略图下缘之上，遮住图片）。
// - 缩略图支持右键菜单（filmstripMenuItems：在预览中打开 / 复制图片 / 复制路径）。
import { computed, nextTick, onMounted, onUnmounted, useTemplateRef, ref, watch } from 'vue'
import { FileIcon } from '@lucide/vue'
import { useCapturesStore } from '@/stores/captures'
import { useSelectionStore } from '@/stores/selection'
import { useContextMenuStore, filmstripMenuItems } from '@/stores/contextMenu'
import { copyImage, copyText } from '@/lib/clipboard'
import { ptimgUrl } from '@/lib/ipc'
import { displayName, isOtherFormat } from '@/lib/format'
import type { CaptureMeta } from '@/lib/bindings'

const captures = useCapturesStore()
const selection = useSelectionStore()
const contextMenu = useContextMenuStore()

/** 缩略图尺寸（px）：固定宽高比，虚拟列表以它为定位单位 */
const ITEM_W = 96
const GAP = 6
/** 可视区外预渲染的缓冲条目数（滚动时提前挂载，避免白屏） */
const BUFFER = 6

const stripRef = useTemplateRef<HTMLElement>('strip')
const trackRef = useTemplateRef<HTMLElement>('track')
// 注：滚动条 thumb 是 v-if 渲染的，不持有 template ref（见 syncScrollbar 注释）

/** 全部条目（c = CaptureMeta，i = 全局下标 / data-index） */
const allItems = computed(() => captures.items.map((c, i) => ({ c, i })))

/** 内容总宽（虚拟列表 scrollWidth = 全部条目宽度；滚动条据此反映整体比例） */
const totalWidth = computed(() => {
  const n = captures.items.length
  if (n === 0) return 0
  return n * ITEM_W + (n - 1) * GAP
})

/** 可视窗口（目前挂载的下标区间），由滚动位置驱动 */
const win = ref({ start: 0, end: 0 })

/** 根据滚动位置计算可视窗口（含缓冲） */
function computeWindow() {
  const el = stripRef.value
  if (!el) return
  const n = captures.items.length
  if (n === 0) {
    win.value = { start: 0, end: 0 }
    return
  }
  const sl = el.scrollLeft
  const vw = el.clientWidth
  const step = ITEM_W + GAP
  const start = Math.max(0, Math.floor(sl / step) - BUFFER)
  const end = Math.min(n, Math.ceil((sl + vw) / step) + BUFFER)
  win.value = { start, end }
}

/** 当前挂载的可视条目（带绝对定位 left） */
const visibleItems = computed(() =>
  allItems.value
    .slice(win.value.start, win.value.end)
    .map(({ c, i }) => ({ c, i, left: i * (ITEM_W + GAP) })),
)

/** 自定义滚动条 thumb 位置/宽度（相对 track；无溢出时隐藏） */
const scrollThumb = ref({ left: 0, width: 0, visible: false })

/** 根据容器滚动量刷新滚动条 thumb（滚动 / 尺寸变化 / 条目变化时调用） */
function syncScrollbar() {
  const el = stripRef.value
  const track = trackRef.value
  // 不能用 thumb 的 template ref 做前置条件：它是 v-if="scrollThumb.visible"
  // 渲染的，首帧必为 null，会把 visible 永远卡在 false（滚动条永不出现）
  if (!el || !track) return
  const total = totalWidth.value
  const client = el.clientWidth
  if (total <= client) {
    scrollThumb.value = { left: 0, width: 0, visible: false }
    return
  }
  const maxScroll = total - client
  const trackW = track.clientWidth
  const width = Math.max((client / total) * trackW, 24)
  const left = (el.scrollLeft / maxScroll) * (trackW - width)
  scrollThumb.value = { left: Math.max(0, Math.min(left, trackW - width)), width, visible: true }
}

/** 虚拟列表 + 滚动条一起刷新 */
function refresh() {
  computeWindow()
  syncScrollbar()
}

/** 选中变化时把当前项滚到可见（虚拟列表按下标直接算目标 scrollLeft，不依赖 DOM） */
function scrollSelectedIntoView() {
  const el = stripRef.value
  if (!el) return
  const idx = selection.selectedIndex
  if (idx === null) return
  const step = ITEM_W + GAP
  const itemLeft = idx * step
  const itemRight = itemLeft + ITEM_W
  const sl = el.scrollLeft
  const vw = el.clientWidth
  // 已完全可见则不滚动
  if (itemLeft >= sl && itemRight <= sl + vw) return
  const target = itemLeft - (vw - ITEM_W) / 2
  el.scrollTo({ left: Math.max(0, Math.min(target, totalWidth.value - vw)), behavior: 'smooth' })
}

/**
 * 滚轮横滚：纵向滚轮 deltaY 映射为横向 scrollLeft（横向 overflow 容器默认不响应纵轮）。
 * .prevent.stop：阻止冒泡到 PhotoPreview 的滚轮缩放（滚胶片条不应缩放图片）。
 */
function onWheel(e: WheelEvent) {
  const el = stripRef.value
  if (!el) return
  el.scrollLeft += e.deltaY + e.deltaX
}

// ── 自定义滚动条布局/同步 ──────────────────────────────
function onScroll() {
  computeWindow()
  syncScrollbar()
}
let resizeObserver: ResizeObserver | null = null

onMounted(() => {
  scrollSelectedIntoView()
  const track = trackRef.value
  if (track) {
    resizeObserver = new ResizeObserver(() => refresh())
    resizeObserver.observe(track)
  }
  void nextTick(refresh)
})
onUnmounted(() => {
  resizeObserver?.disconnect()
  resizeObserver = null
})

// 条目数量变化（重扫/筛选）后刷新窗口
watch(() => captures.items.length, () => void nextTick(refresh))

// 选中变化（方向键/网格单击/堆叠切换）→ 胶片条跟随滚动到当前项
watch(() => selection.selectedIndex, () => void nextTick(scrollSelectedIntoView))

// ── 自定义滚动条拖拽 ──────────────────────────────
// 在 track 上按下开始拖拽 thumb：按住拖动时按百分比映射 scrollLeft。
// 点击 track 空白处直接跳到对应位置。
let draggingTrack = false
function onTrackPointerDown(e: PointerEvent) {
  const track = trackRef.value
  const el = stripRef.value
  if (!track || !el) return
  if (e.target === track) {
    seekTrack(e.clientX - track.getBoundingClientRect().left)
  } else {
    draggingTrack = true
  }
  track.setPointerCapture(e.pointerId)
}
function onTrackPointerMove(e: PointerEvent) {
  const track = trackRef.value
  const el = stripRef.value
  if (!track || !el) return
  if (draggingTrack) seekTrack(e.clientX - track.getBoundingClientRect().left)
}
function onTrackPointerUp(e: PointerEvent) {
  draggingTrack = false
  trackRef.value?.releasePointerCapture(e.pointerId)
}
/** 把 track 上的某点（相对 left）映射为容器滚动量（thumb 行走范围扣除自身宽度） */
function seekTrack(offset: number) {
  const track = trackRef.value
  const el = stripRef.value
  if (!track || !el) return
  const rect = track.getBoundingClientRect()
  const trackW = rect.width
  const total = totalWidth.value
  const client = el.clientWidth
  if (total <= client || trackW <= 0) return
  const width = Math.max((client / total) * trackW, 24)
  const range = trackW - width
  const fraction = Math.max(0, Math.min(1, (offset - width / 2) / range))
  el.scrollLeft = fraction * (total - client)
}

// ── 缩略图右键菜单（在预览中打开 / 复制图片 / 复制路径） ─────
function onThumbContextMenu(c: CaptureMeta, i: number, e: MouseEvent) {
  e.preventDefault()
  contextMenu.openMenu(
    filmstripMenuItems({
      meta: c,
      onOpen: () => selection.select(i),
      onCopyImage: (m) => void copyImage(m.primaryPath),
      onCopyPath: (m) => void copyText(m.primaryPath),
    }),
    e.clientX,
    e.clientY,
  )
}
</script>

<template>
  <div class="flex shrink-0 flex-col border-t bg-card">
    <!-- 缩略图条：横向虚拟列表（原生滚动条隐藏，滚动条在最下方独立 track） -->
    <div
      ref="strip"
      class="relative h-20 shrink-0 overflow-x-auto [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
      @wheel.prevent.stop="onWheel"
      @scroll="onScroll"
    >
      <!-- 全内容宽度占位（决定 scrollWidth，让滚动条反映全部条目） -->
      <div class="relative h-full" :style="{ width: totalWidth + 'px' }">
        <button
          v-for="{ c, i, left } in visibleItems"
          :key="c.primaryPath"
          :data-index="i"
          type="button"
          :style="{ left: left + 'px' }"
          class="absolute top-0 h-full w-24 overflow-hidden rounded-sm border transition-colors"
          :class="
            i === selection.selectedIndex
              ? 'border-primary ring-1 ring-primary'
              : 'border-border hover:border-muted-foreground'
          "
          :title="displayName(c)"
          @click="selection.select(i)"
          @contextmenu="onThumbContextMenu(c, i, $event)"
        >
          <!-- 非图片格式（OTHER，视频等）：无缩略图，居中 File 图标（对齐网格 cell 的特判，避免破图突兀） -->
          <span
            v-if="isOtherFormat(c)"
            class="flex h-full items-center justify-center bg-muted"
          >
            <FileIcon class="size-5 text-muted-foreground/60" />
          </span>
          <img
            v-else
            :src="ptimgUrl('thumb', c.primaryPath, captures.thumbVersions[c.primaryPath])"
            :alt="displayName(c)"
            draggable="false"
            loading="lazy"
            class="h-full w-full object-cover"
          />
        </button>
      </div>
    </div>

    <!-- 独立横向滚动条（缩略图下方）：track + thumb，随滚动同步，可点击/拖拽 -->
    <div
      ref="track"
      class="relative h-1.5 shrink-0 cursor-pointer bg-muted"
      @pointerdown="onTrackPointerDown"
      @pointermove="onTrackPointerMove"
      @pointerup="onTrackPointerUp"
      @pointercancel="onTrackPointerUp"
    >
      <div
        v-if="scrollThumb.visible"
        ref="thumb"
        class="absolute top-0 h-full rounded-full bg-muted-foreground/40 transition-colors hover:bg-muted-foreground/60"
        :style="{ left: scrollThumb.left + 'px', width: scrollThumb.width + 'px' }"
      />
    </div>
  </div>
</template>
