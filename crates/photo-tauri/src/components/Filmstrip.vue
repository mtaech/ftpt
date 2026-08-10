<script setup lang="ts">
// 胶片条：预览视图底部横条（h-20）。展示全部拍摄的 thumb 缩略图，横向滚动；
// 点击跳转选中（selection.select），当前项高亮边框，选中变化时横滚跟随。
// 缩略图走 ptimgUrl('thumb', path, v)，v 来自 captures.thumbVersions（thumb:ready 后强制刷新）。
import { nextTick, onMounted, useTemplateRef, watch } from 'vue'
import { useCapturesStore } from '@/stores/captures'
import { useSelectionStore } from '@/stores/selection'
import { ptimgUrl } from '@/lib/ipc'
import { displayName } from '@/lib/format'

const captures = useCapturesStore()
const selection = useSelectionStore()

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
</script>

<template>
  <div
    ref="strip"
    class="flex h-20 shrink-0 items-center gap-1.5 overflow-x-auto border-t bg-card px-2"
  >
    <button
      v-for="(c, i) in captures.items"
      :key="c.primaryPath"
      :data-index="i"
      type="button"
      class="h-full shrink-0 cursor-pointer overflow-hidden rounded-sm border transition-colors"
      :class="
        i === selection.selectedIndex
          ? 'border-primary ring-1 ring-primary'
          : 'border-border hover:border-muted-foreground'
      "
      :title="displayName(c)"
      @click="selection.select(i)"
    >
      <img
        :src="ptimgUrl('thumb', c.primaryPath, captures.thumbVersions[c.primaryPath])"
        :alt="displayName(c)"
        draggable="false"
        class="h-full w-full object-cover"
      />
    </button>
  </div>
</template>
