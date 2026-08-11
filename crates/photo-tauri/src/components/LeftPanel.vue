<script setup lang="ts">
// 左栏容器：目录 / 批量 双 tab（tab 头对齐右栏 InfoPanel 的下划线式 Tabs）。
// 拖宽把手与宽度持久化（localStorage，200–480）自 Sidebar 上移至此，两个 tab 共享宽度。
import { computed } from 'vue'
import { useStorage } from '@vueuse/core'
import { Tabs, TabsList, TabsTrigger } from '@/components/ui/tabs'
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

/** tab 头样式（与 InfoPanel 头完全一致：下划线选中态） */
const triggerCls =
  'h-full rounded-none border-b-2 border-transparent px-2 text-xs data-[state=active]:border-primary data-[state=active]:bg-transparent data-[state=active]:font-medium data-[state=active]:text-foreground data-[state=active]:shadow-none'
</script>

<template>
  <aside
    class="relative flex h-full shrink-0 flex-col border-r bg-card"
    :style="{ width: `${clampedWidth}px` }"
  >
    <!-- tab 头：目录 / 批量（对齐 InfoPanel 头部） -->
    <div class="flex h-10 shrink-0 items-center border-b border-border px-2">
      <Tabs v-model="tab" class="h-full">
        <TabsList class="h-full items-stretch rounded-none bg-transparent p-0 text-muted-foreground">
          <TabsTrigger value="dir" :class="triggerCls">目录</TabsTrigger>
          <TabsTrigger value="batch" :class="triggerCls">批量</TabsTrigger>
        </TabsList>
      </Tabs>
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
