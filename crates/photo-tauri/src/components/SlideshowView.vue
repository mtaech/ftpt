<script setup lang="ts">
// 幻灯片模式（PolishPack 批次）：'s' 进入（从当前选中张开始，按当前筛选结果顺序），
// 黑底全屏主区 + ptimg master 图源；自动播放 3s/张（空格暂停/继续，暂停时显示提示），
// ←/→ 手动切换（切换即重置计时），Esc/G 退出还原来源视图（对齐 compare 模式）。
// 标记键（1-5/0 评分、6-9/Ctrl+6 色标、P/X/U 旗标）经 App.vue markPaths 的
// slideshow 分支作用于当前显示张。计时用递归 setTimeout（切张/暂停即重置，
// 不用 interval 避免累积漂移）。
import { computed, onUnmounted, watch } from 'vue'
import { PauseIcon, PlayIcon } from '@lucide/vue'
import { useCapturesStore } from '@/stores/captures'
import { useFilterStore } from '@/stores/filter'
import { usePreviewStore } from '@/stores/preview'
import { ptimgUrl } from '@/lib/ipc'
import { displayName } from '@/lib/format'

/** 自动播放间隔（3s/张） */
const SLIDE_MS = 3000

const captures = useCapturesStore()
const filter = useFilterStore()
const preview = usePreviewStore()

/** 当前显示张（captures.items 下标；筛选序越界安全） */
const currentIndex = computed(() => filter.filteredIndices[preview.slideshowIndex] ?? null)
const current = computed(() => (currentIndex.value === null ? null : captures.items[currentIndex.value] ?? null))

/** 图源：master 预览母版（对齐对比模式图源） */
const imgSrc = computed(() => {
  const c = current.value
  if (!c) return ''
  return ptimgUrl('master', c.primaryPath, captures.thumbVersions[c.primaryPath])
})

const total = computed(() => filter.filteredIndices.length)
/** 位置文案（1-based） */
const positionText = computed(() => (total.value === 0 ? '0/0' : `${preview.slideshowIndex + 1}/${total.value}`))

// ── 自动播放计时（递归 setTimeout：切张/暂停立即重置，杜绝 interval 漂移） ──
let timer: number | undefined
function schedule() {
  clearTimeout(timer)
  if (!preview.slideshowPlaying) return
  timer = window.setTimeout(() => preview.slideshowStep(1), SLIDE_MS)
}

// 切张（手动 ←/→ 或自动步进）与暂停状态变化 → 重置计时；
// immediate：挂载即启动自动播放（否则首个 3s 计时永远不会开始）
watch(
  () => [preview.slideshowIndex, preview.slideshowPlaying] as const,
  () => schedule(),
  { immediate: true },
)

// 筛选结果变化（标记筛选/重扫）→ 钳制到合法范围并重排计时
watch(
  () => total.value,
  (n) => {
    if (n === 0) return
    if (preview.slideshowIndex >= n) preview.slideshowIndex = n - 1
    schedule()
  },
)

onUnmounted(() => clearTimeout(timer))
</script>

<template>
  <div class="flex h-full flex-col bg-black text-white">
    <!-- 主区：黑底 + 图片 contain 居中 -->
    <div class="relative flex min-h-0 flex-1 items-center justify-center overflow-hidden">
      <img
        v-if="current"
        :src="imgSrc"
        :alt="displayName(current)"
        draggable="false"
        class="max-h-full max-w-full select-none object-contain"
      />

      <!-- 左上：文件名 + 位置 -->
      <div
        v-if="current"
        class="absolute top-3 left-3 flex items-center gap-2 rounded bg-black/60 px-2 py-1 text-xs"
      >
        <span class="max-w-[30rem] truncate">{{ displayName(current) }}</span>
        <span class="tabular-nums text-white/70">{{ positionText }}</span>
      </div>

      <!-- 暂停提示（暂停时显示，继续后淡出） -->
      <Transition name="pause-fade">
        <div
          v-if="!preview.slideshowPlaying"
          class="absolute inset-0 flex items-center justify-center bg-black/40"
        >
          <div class="flex items-center gap-2 rounded-lg bg-black/70 px-4 py-2 text-sm">
            <PauseIcon class="size-4" />
            已暂停 · 空格继续
          </div>
        </div>
      </Transition>

      <!-- 底部提示条：播放状态 + 操作说明 -->
      <div
        class="absolute bottom-3 left-1/2 flex -translate-x-1/2 items-center gap-3 rounded bg-black/60 px-3 py-1 text-xs whitespace-nowrap text-white/80"
      >
        <span class="flex items-center gap-1">
          <PlayIcon v-if="preview.slideshowPlaying" class="size-3" />
          <PauseIcon v-else class="size-3" />
          {{ preview.slideshowPlaying ? '自动播放' : '已暂停' }}
        </span>
        <span class="text-white/40">|</span>
        <span>空格 暂停/继续 · ←/→ 切换 · Esc/G 退出</span>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* 暂停提示淡入淡出（对齐 PhotoPreview loading-fade：200ms 强 ease-out） */
.pause-fade-enter-active,
.pause-fade-leave-active {
  transition: opacity 200ms cubic-bezier(0.23, 1, 0.32, 1);
}
.pause-fade-enter-from,
.pause-fade-leave-to {
  opacity: 0;
}
</style>
