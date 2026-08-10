<script setup lang="ts">
// 底部状态栏：24px 三段式。左：当前目录；中：项数 + 选中数（Phase 2 多选）；右：扫描状态/进度摘要 + 识别占位（Phase 3）。
import { computed } from 'vue'
import { FolderIcon, SparklesIcon } from '@lucide/vue'
import { useCapturesStore } from '@/stores/captures'
import { useSelectionStore } from '@/stores/selection'

const captures = useCapturesStore()
const selection = useSelectionStore()

/** 目录显示名（路径末段，对齐 App.vue dirName） */
function dirName(dir: string | null): string {
  if (!dir) return ''
  return dir.split(/[\\/]/).filter(Boolean).pop() ?? dir
}

/** 扫描阶段文案（对齐顶栏进度条措辞） */
const stageText = computed(() => {
  const stage = captures.progress?.stage
  return stage === 'scan' ? '扫描' : stage === 'exif' ? 'EXIF' : '缩略图'
})

/** 选中数（多选集；>0 时以 PICK 色强调） */
const selectedCount = computed(() => selection.selectedIndices.length)
</script>

<template>
  <footer
    class="flex h-6 shrink-0 items-center gap-3 border-t bg-background px-3 text-xs text-muted-foreground"
  >
    <!-- 左段：当前目录（截断，悬停显示完整路径） -->
    <div class="flex min-w-0 flex-1 items-center gap-1.5">
      <FolderIcon class="size-3 shrink-0" />
      <span class="truncate" :title="captures.directory ?? ''">
        {{ captures.directory ? dirName(captures.directory) : '无目录' }}
      </span>
    </div>

    <!-- 中段：项数 + 选中数（等宽数字，对齐 GPUI status_bar 计数区） -->
    <div class="flex shrink-0 items-center gap-1 font-mono-num">
      <span>{{ captures.count }}</span>
      <span class="text-muted-foreground/70">项</span>
      <span class="text-muted-foreground/70">·</span>
      <span :class="selectedCount > 0 ? 'text-pick' : ''">{{ selectedCount }}</span>
      <span class="text-muted-foreground/70">已选</span>
    </div>

    <!-- 右段：扫描状态 + 识别占位 -->
    <div class="flex shrink-0 items-center gap-2">
      <span v-if="captures.scanning" class="font-mono-num text-primary">
        {{ stageText }}
        <template v-if="captures.progress && captures.progress.total > 0">
          {{ captures.progress.done }}/{{ captures.progress.total }}
        </template>
      </span>
      <span v-else>就绪</span>
      <!-- 识别状态占位：Phase 3 接入 recognition store 后替换为真实状态 -->
      <span class="flex items-center gap-1 text-muted-foreground/60" title="识别（Phase 3 接入）">
        <SparklesIcon class="size-3" />
        识别待接入
      </span>
    </div>
  </footer>
</template>
