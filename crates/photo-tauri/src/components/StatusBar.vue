<script setup lang="ts">
// 底部状态栏：24px 三段式。左：当前目录；中：项数 + 选中数；右：扫描状态/进度 +
// 识别进行中（n/m · 文件名 + ✕ 取消）/完成摘要（数秒后消失）/空提示。
import { computed, ref, watch } from 'vue'
import { FolderIcon, SparklesIcon, XIcon } from '@lucide/vue'
import { useCapturesStore } from '@/stores/captures'
import { useSelectionStore } from '@/stores/selection'
import { useRecognitionStore } from '@/stores/recognition'

const captures = useCapturesStore()
const selection = useSelectionStore()
const recognition = useRecognitionStore()

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

// ── 识别状态区 ────────────────────────────────────────────────

/** 当前识别文件名（进度 currentPath 末段；空则显示「准备中」） */
const currentFileName = computed(() => {
  const p = recognition.progress?.currentPath
  if (!p) return '准备中'
  return p.split(/[\\/]/).filter(Boolean).pop() ?? p
})

/** 完成摘要文案：确认必显，待复核/无鸟/失败非零才追加（对齐 GPUI 批量完成 toast） */
const summaryText = computed(() => {
  const s = recognition.summary
  if (!s) return ''
  const parts = [`确认 ${s.confirmed}`]
  if (s.needsReview > 0) parts.push(`待复核 ${s.needsReview}`)
  if (s.unrecognized > 0) parts.push(`无鸟 ${s.unrecognized}`)
  if (s.failed > 0) parts.push(`失败 ${s.failed}`)
  return `识别完成：${parts.join(' · ')}`
})

/** 摘要展示开关：summary 到来显示，SUMMARY_MS 后隐藏并清空 store（GPUI toast 语义） */
const SUMMARY_MS = 4000
const showSummary = ref(false)
let summaryTimer: number | undefined

watch(
  () => recognition.summary,
  (s) => {
    clearTimeout(summaryTimer)
    if (!s) {
      showSummary.value = false
      return
    }
    showSummary.value = true
    summaryTimer = setTimeout(() => {
      showSummary.value = false
      recognition.reset()
    }, SUMMARY_MS)
  },
)
</script>

<template>
  <footer
    class="flex h-6 shrink-0 items-center gap-3 border-t bg-card px-3 text-xs text-muted-foreground"
  >
    <!-- 左段：当前目录（截断，悬停显示完整路径） -->
    <div class="flex min-w-0 flex-1 items-center gap-1.5">
      <FolderIcon class="size-3 shrink-0" />
      <span class="truncate" :title="captures.directory ?? ''">
        {{ captures.directory ? dirName(captures.directory) : '无目录' }}
      </span>
    </div>

    <!-- 中段：项数 + 选中数（等宽数字，对齐 GPUI status_bar 计数区） -->
    <div class="flex shrink-0 items-center gap-1 tabular-nums">
      <span>{{ captures.count }}</span>
      <span class="text-muted-foreground/70">项</span>
      <span class="text-muted-foreground/70">·</span>
      <span :class="selectedCount > 0 ? 'text-pick' : ''">{{ selectedCount }}</span>
      <span class="text-muted-foreground/70">已选</span>
    </div>

    <!-- 右段：扫描状态 + 识别进度/摘要/空提示（优先级：扫描 > 识别中 > 摘要 > 提示 > 就绪） -->
    <div class="flex shrink-0 items-center gap-2">
      <span v-if="captures.scanning" class="tabular-nums text-primary">
        {{ stageText }}
        <template v-if="captures.progress && captures.progress.total > 0">
          {{ captures.progress.done }}/{{ captures.progress.total }}
        </template>
      </span>
      <!-- 识别进行中：n/m · 当前文件名 + ✕/Esc 取消 -->
      <span v-else-if="recognition.running" class="flex items-center gap-1.5 text-primary">
        <SparklesIcon class="size-3 animate-pulse" />
        <span class="tabular-nums">
          {{ recognition.progress?.done ?? 0 }}/{{ recognition.progress?.total ?? '…' }}
        </span>
        <span class="max-w-44 truncate" :title="recognition.progress?.currentPath ?? ''">
          {{ currentFileName }}
        </span>
        <button
          type="button"
          class="text-muted-foreground/70 hover:text-foreground"
          title="取消识别 (Esc)"
          aria-label="取消识别"
          @click="recognition.cancel()"
        >
          <XIcon class="size-3" />
        </button>
      </span>
      <!-- 识别完成摘要（数秒后消失） -->
      <span v-else-if="showSummary && recognition.summary" class="tabular-nums text-label-green">
        {{ summaryText }}
      </span>
      <!-- 空提示（无未识别照片等） -->
      <span v-else-if="recognition.notice" class="text-muted-foreground">
        {{ recognition.notice }}
      </span>
      <span v-else>就绪</span>
    </div>
  </footer>
</template>
