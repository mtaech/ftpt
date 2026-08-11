<script setup lang="ts">
// 胶片条：预览视图底部横条（h-20）。展示全部拍摄的 thumb 缩略图，横向滚动；
// 点击跳转选中（selection.select），当前项高亮边框，选中变化时横滚跟随。
// 缩略图走 ptimgUrl('thumb', path, v)，v 来自 captures.thumbVersions（thumb:ready 后强制刷新）。
import { computed, nextTick, onMounted, useTemplateRef, watch } from 'vue'
import { FileIcon } from '@lucide/vue'
import { useCapturesStore } from '@/stores/captures'
import { useSelectionStore } from '@/stores/selection'
import { ptimgUrl } from '@/lib/ipc'
import { displayName, isOtherFormat } from '@/lib/format'

const captures = useCapturesStore()
const selection = useSelectionStore()

/**
 * 渲染窗口：只挂载选中项 ±40 共 80 个缩略图按钮（选中移动窗口跟随）。
 * 全量 v-for 在大目录（1k+）会一次挂上千个 img 请求与 DOM 节点，是切大图目录卡顿的主要来源。
 */
const WINDOW = 80
const windowStart = computed(() => {
  const total = captures.items.length
  if (total <= WINDOW) return 0
  const sel = selection.selectedIndex ?? 0
  return Math.min(Math.max(sel - WINDOW / 2, 0), total - WINDOW)
})
/** 窗口内条目：c = CaptureMeta，i = 全局下标（data-index / 点击选中用） */
const windowItems = computed(() =>
  captures.items
    .slice(windowStart.value, windowStart.value + WINDOW)
    .map((c, k) => ({ c, i: windowStart.value + k })),
)

const stripRef = useTemplateRef<HTMLElement>('strip')

/** 选中变化时把当前项滚到可见（不可见才滚，目标尽量居中） */
function scrollSelectedIntoView() {
  const el = stripRef.value
  if (!el) return
  const idx = selection.selectedIndex
  if (idx === null) return
  const item = el.querySelector<HTMLElement>(`[data-index="${idx}"]`)
  if (!item) return
  const elRect = el.getBoundingClientRect()
  const itemRect = item.getBoundingClientRect()
  const itemLeft = el.scrollLeft + (itemRect.left - elRect.left)
  const itemRight = itemLeft + itemRect.width
  // 已完全可见则不滚动
  if (itemLeft >= el.scrollLeft && itemRight <= el.scrollLeft + el.clientWidth) return
  el.scrollTo({ left: itemLeft - (el.clientWidth - itemRect.width) / 2, behavior: 'smooth' })
}

watch(() => selection.selectedIndex, () => void nextTick(scrollSelectedIntoView))

// 进入预览（组件挂载）时先对一次当前选中
onMounted(scrollSelectedIntoView)

/**
 * 滚轮横滚：纵向滚轮 deltaY 映射为横向 scrollLeft（横向 overflow 容器默认不响应纵轮）。
 * .prevent.stop：阻止冒泡到 PhotoPreview 的滚轮缩放（滚胶片条不应缩放图片）。
 */
function onWheel(e: WheelEvent) {
  const el = stripRef.value
  if (!el) return
  el.scrollLeft += e.deltaY + e.deltaX
}
</script>

<template>
  <div
    ref="strip"
    class="flex h-20 shrink-0 items-center gap-1.5 overflow-x-auto border-t bg-card px-2"
    @wheel.prevent.stop="onWheel"
  >
    <button
      v-for="{ c, i } in windowItems"
      :key="c.primaryPath"
      :data-index="i"
      type="button"
      class="h-full shrink-0 overflow-hidden rounded-sm border transition-colors"
      :class="
        i === selection.selectedIndex
          ? 'border-primary ring-1 ring-primary'
          : 'border-border hover:border-muted-foreground'
      "
      :title="displayName(c)"
      @click="selection.select(i)"
    >
      <!-- 非图片格式（OTHER，视频等）：无缩略图，居中 File 图标（对齐网格 cell 的特判，避免破图突兀） -->
      <span
        v-if="isOtherFormat(c)"
        class="flex aspect-[4/3] h-full items-center justify-center bg-muted"
      >
        <FileIcon class="size-5 text-muted-foreground/60" />
      </span>
      <img
        v-else
        :src="ptimgUrl('thumb', c.primaryPath, captures.thumbVersions[c.primaryPath])"
        :alt="displayName(c)"
        draggable="false"
        class="h-full w-full object-cover"
      />
    </button>
  </div>
</template>
