<script setup lang="ts">
// 右侧 Activity Rail：48px 宽，与左侧 rail 对称（对齐 GPUI right_rail.rs）。
// 按钮：右侧信息面板切换（统计视图已上移到顶栏 网格/预览/统计 tab）。
import { PanelRightIcon, PanelRightOpenIcon } from '@lucide/vue'
import { Button } from '@/components/ui/button'

defineProps<{
  /** 右侧面板当前是否可见（决定图标与提示文案） */
  visible: boolean
}>()

/** 切换右侧面板（App.vue 监听；等价 Ctrl+]） */
const emit = defineEmits<{ toggle: [] }>()
</script>

<template>
  <nav
    class="flex w-12 shrink-0 flex-col items-center gap-1 border-l bg-card pt-2"
    aria-label="右侧活动栏"
  >
    <Button
      size="icon-sm"
      variant="ghost"
      class="text-muted-foreground hover:bg-accent"
      :title="visible ? '隐藏右侧面板' : '显示右侧面板'"
      :aria-pressed="visible"
      :class="{ 'bg-accent text-accent-foreground': visible }"
      @click="emit('toggle')"
    >
      <PanelRightIcon v-if="visible" class="size-4" />
      <PanelRightOpenIcon v-else class="size-4" />
    </Button>
  </nav>
</template>
