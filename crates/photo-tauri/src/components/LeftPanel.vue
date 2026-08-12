<script setup lang="ts">
// 左栏容器：文件树 / 文件操作 双 tab（GPUI sidebar 药丸式 tab，激活 = element-hover 底）。
// 拖宽把手与宽度持久化（localStorage，200–480）自 Sidebar 上移至此，两个 tab 共享宽度。
import { computed } from 'vue'
import { useStorage } from '@vueuse/core'
import Sidebar from '@/components/Sidebar.vue'
import BatchOpsPanel from '@/components/BatchOpsPanel.vue'

/** 当前 tab：'dir' 目录 | 'batch' 批量（App 顶栏「批量操作」按钮可切到 batch） */
const tab = defineModel<string>({ default: 'dir' })

// ── 宽度：可拖拽，localStorage 持久化，范围 200–480（对齐 GPUI 左栏 size_range）──
const width = useStorage('ftpt.leftPanelWidth', 220)
const clampedWidth = computed(() => Math.min(480, Math.max(200, width.value)))
/** 拖拽起始状态（指针捕获在把手上，move/up 仍持续收到） */
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
  width.value = Math.min(480, Math.max(200, dragStartW + (e.clientX - dragStartX)))
}
/** 拖拽结束：指针捕获在 pointerup 后由浏览器自动释放，无需清理 */
function onHandleUp() {}

</script>

<template>
  <aside
    class="relative flex h-full shrink-0 flex-col border-r bg-card"
    :style="{ width: `${clampedWidth}px` }"
  >
    <!-- tab 头：文件树 / 文件操作（GPUI sidebar 药丸式 tab：激活 = bg-element-hover） -->
    <div class="flex shrink-0 gap-1 border-b border-border p-1.5">
      <button
        type="button"
        class="flex-1 rounded-sm py-1 text-center text-xs transition-colors select-none"
        :class="
          tab === 'dir'
            ? 'bg-element-hover font-medium text-foreground'
            : 'text-muted-foreground hover:text-foreground'
        "
        :aria-pressed="tab === 'dir'"
        @click="tab = 'dir'"
      >
        文件树
      </button>
      <button
        type="button"
        class="flex-1 rounded-sm py-1 text-center text-xs transition-colors select-none"
        :class="
          tab === 'batch'
            ? 'bg-element-hover font-medium text-foreground'
            : 'text-muted-foreground hover:text-foreground'
        "
        :aria-pressed="tab === 'batch'"
        @click="tab = 'batch'"
      >
        文件操作
      </button>
    </div>

    <!-- tab 内容（v-show 保状态：目录列表/批量表单互切不丢；包裹 div 承载显隐，BatchOpsPanel 为多根组件） -->
    <div v-show="tab === 'dir'" class="flex min-h-0 flex-1 flex-col">
      <Sidebar />
    </div>
    <div v-show="tab === 'batch'" class="flex min-h-0 flex-1 flex-col">
      <BatchOpsPanel />
    </div>

    <!-- 拖宽把手（右缘，指针捕获保证拖出面板仍生效） -->
    <div
      class="absolute inset-y-0 right-0 z-10 w-1 cursor-col-resize touch-none select-none hover:bg-primary/40"
      @pointerdown="onHandleDown"
      @pointermove="onHandleMove"
      @pointerup="onHandleUp"
      @pointercancel="onHandleUp"
    />
  </aside>
</template>
