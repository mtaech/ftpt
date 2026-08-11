<script setup lang="ts">
// 批量操作面板（移植 GPUI ui/batch_ops.rs render_batch_ops_section）：
//   操作对象 = 当前筛选结果（无筛选时三按钮禁用 + 黄色警告，防全文件误操作）；
//   操作类型三选（移动/复制/删除）；「同步同名文件」开关 + 格式多选 chips；
//   移动/复制一步式选目标目录（ui/dialog，拒绝「目标 = 源目录」toast）；
//   删除红色确认框（前 20 条清单 + 「其中 M 个来自同名同步」警告）；
//   两阶段执行流：开始执行（干跑预览，只列文件名不动文件）→ 确认执行（N 个）
//   → 进度弹窗（n/m · 当前文件）+ 完成 toast 摘要 + 失败明细列表（可滚动）。
import { computed, onMounted, ref, watch } from 'vue'
import { CheckIcon, FolderOpenIcon, TriangleAlertIcon } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import {
  formatsEqual,
  formatsInDirectory,
  opDescription,
  opLabel,
  useBatchStore,
} from '@/stores/batch'
import { useCapturesStore } from '@/stores/captures'
import { useFilterStore } from '@/stores/filter'
import { formatToString } from '@/lib/filter'
import type { BatchOpType } from '@/lib/bindings'

const batch = useBatchStore()
const captures = useCapturesStore()
const filter = useFilterStore()

// ── 本地 UI 状态 ─────────────────────────────────────

/** 目标目录对话框（移动/复制一步式） */
const showTargetDialog = ref(false)
/** 删除红色确认框（干跑后点「确认执行」弹出） */
const showDeleteConfirm = ref(false)

// ── 派生状态 ─────────────────────────────────────────

/** 操作类型三选（label 对齐 GPUI action_label） */
const OP_OPTIONS: { op: BatchOpType; label: string }[] = [
  { op: 'Move', label: '移动' },
  { op: 'Copy', label: '复制' },
  { op: 'Delete', label: '删除' },
]

/** 当前筛选结果数量（操作集大小，随筛选实时联动） */
const count = computed(() => filter.filteredIndices.length)
const empty = computed(() => count.value === 0)
/** 无筛选条件时拒绝执行：操作集 = 全部文件，防误操作 */
const noFilter = computed(() => !filter.hasActiveFilters)
/** 三按钮禁用：空筛选结果 / 无筛选条件 / 执行中（对齐 GPUI disabled = empty || no_filter） */
const opsDisabled = computed(() => empty.value || noFilter.value || batch.running)

/** 移动/复制需要目标目录 */
const targetNeeded = computed(() => batch.op !== 'Delete')

/** 「开始执行 / 确认执行」可用性 */
const canRun = computed(
  () =>
    !empty.value &&
    filter.hasActiveFilters &&
    (!targetNeeded.value || !!batch.targetDir) &&
    !batch.running,
)

/** 同步格式 chips：目录实际出现的格式，默认全选（开启开关时初始化） */
const formatChips = computed(() =>
  formatsInDirectory(captures.items).map((fmt) => ({
    fmt,
    key: typeof fmt === 'string' ? fmt : `raw:${fmt.Raw}`,
    label: formatToString(fmt),
    active: batch.formats.some((f) => formatsEqual(f, fmt)),
  })),
)

/** 干跑预览计数（确认按钮文案） */
const previewCount = computed(() => batch.preview?.count ?? 0)
/** 同步拉入的额外文件数（删除确认框警告） */
const previewSiblingCount = computed(() => batch.preview?.siblingCount ?? 0)
/** 删除确认框清单：前 20 条文件名 */
const deletePreviewNames = computed(() =>
  (batch.preview?.items ?? []).slice(0, 20).map((it) => it.path.split(/[\\/]/).pop() ?? it.path),
)

/** 进度弹窗文案：n/m · 当前文件 */
const progressText = computed(() => {
  const p = batch.progress
  if (!p) return ''
  return `${p.done}/${p.total} · ${p.currentPath.split(/[\\/]/).pop() ?? p.currentPath}`
})
const progressPct = computed(() => {
  const p = batch.progress
  if (!p || p.total <= 0) return 0
  return Math.min(100, Math.round((p.done / p.total) * 100))
})

// ── 交互 ─────────────────────────────────────────────

/**
 * 主按钮两阶段：
 *   无预览 → 干跑（batch_op_preview，只算不动文件）；
 *   有预览 → Delete 弹红色确认框，其余直接确认执行。
 */
function onPrimary() {
  if (batch.running) return
  if (batch.preview) {
    if (batch.op === 'Delete') showDeleteConfirm.value = true
    else void batch.confirmExecute()
  } else {
    void batch.runPreview()
  }
}

/** 删除确认框「确认删除」 */
function onConfirmDelete() {
  showDeleteConfirm.value = false
  void batch.confirmExecute()
}

/** 目标目录对话框「浏览…」：系统目录选择 → 拒绝源目录（toast）→ 成功自动关闭 */
async function chooseTargetAndClose() {
  await batch.chooseTarget()
  if (batch.targetDir) showTargetDialog.value = false
}

// ── 生命周期：接线进度事件（对齐 captures.init 模式）──
onMounted(() => {
  batch.init()
})

// ── toast 自动消失（4s）──
let toastTimer: ReturnType<typeof setTimeout> | null = null
watch(
  () => batch.toast,
  (t) => {
    if (toastTimer) clearTimeout(toastTimer)
    toastTimer = null
    if (!t) return
    toastTimer = setTimeout(() => (batch.toast = null), 4000)
  },
)
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto p-2">
    <!-- 操作指引（简短） -->
    <div class="rounded-md bg-muted/50 px-2 py-1 text-[0.6875rem] leading-snug text-muted-foreground">
      操作对象 = 当前筛选结果。流程：标记/筛选 → 可选「同步同名文件」→ 开始执行（先干跑预览）→ 确认执行
    </div>

    <!-- 操作对象说明（数量随筛选实时联动；无筛选黄色警告） -->
    <div
      class="rounded-md px-2 py-1 text-[0.6875rem] leading-snug"
      :class="noFilter ? 'bg-amber-500/10 text-amber-500' : 'bg-muted/50 text-muted-foreground'"
    >
      <template v-if="noFilter">
        未设置筛选条件——为防止全文件误操作，请先在筛选栏设置条件（旗标/格式/评分等）
      </template>
      <template v-else>
        操作对象：当前筛选结果（{{ count }} 张）
        <template v-if="batch.syncSiblings && batch.preview">，同步 +{{ previewSiblingCount }}</template>
      </template>
    </div>

    <!-- 操作类型三选（对齐 GPUI 移动/复制/删除三按钮，无筛选禁用） -->
    <div class="flex gap-1">
      <Button
        v-for="o in OP_OPTIONS"
        :key="o.op"
        size="sm"
        class="flex-1"
        :variant="batch.op === o.op ? 'default' : 'outline'"
        :disabled="opsDisabled"
        @click="batch.setOp(o.op)"
      >
        {{ o.label }}
      </Button>
    </div>
    <div class="px-1 text-[0.6875rem] text-muted-foreground">操作说明：{{ opDescription(batch.op) }}</div>

    <!-- 目标目录（移动/复制必需；一步式对话框） -->
    <div v-if="targetNeeded" class="flex items-center gap-1">
      <div
        class="min-w-0 flex-1 truncate rounded-md border bg-muted/40 px-2 py-1 tabular-nums text-[0.6875rem]"
        :class="batch.targetDir ? 'text-foreground' : 'text-muted-foreground'"
        :title="batch.targetDir ?? ''"
      >
        {{ batch.targetDir ?? '未选择目标目录' }}
      </div>
      <Button size="xs" variant="outline" :disabled="opsDisabled" @click="showTargetDialog = true">
        选择…
      </Button>
    </div>

    <!-- 同步同名文件开关（默认关） -->
    <button
      class="flex w-fit items-center gap-1.5 text-xs disabled:pointer-events-none disabled:opacity-50"
      :disabled="opsDisabled"
      @click="batch.setSyncSiblings(!batch.syncSiblings)"
    >
      <span
        class="flex size-3.5 items-center justify-center rounded-sm border transition-colors"
        :class="
          batch.syncSiblings
            ? 'border-primary bg-primary text-primary-foreground'
            : 'border-border bg-background'
        "
      >
        <CheckIcon v-if="batch.syncSiblings" class="size-2.5" />
      </span>
      同步同名文件
    </button>

    <!-- 同步格式多选 chips（开启开关后出现；默认全选） -->
    <div v-if="batch.syncSiblings" class="flex flex-wrap gap-1">
      <button
        v-for="chip in formatChips"
        :key="chip.key"
        class="rounded-sm px-2 py-0.5 text-[0.6875rem] transition-colors disabled:pointer-events-none disabled:opacity-50"
        :class="
          chip.active
            ? 'bg-primary text-primary-foreground'
            : 'bg-muted text-muted-foreground hover:bg-accent hover:text-foreground'
        "
        :disabled="opsDisabled"
        @click="batch.toggleFormat(chip.fmt)"
      >
        {{ chip.label }}
      </button>
      <div v-if="formatChips.length === 0" class="text-[0.6875rem] text-muted-foreground">
        目录中没有其他格式的同名文件
      </div>
    </div>

    <!-- 主按钮：开始执行（干跑）→ 确认执行（N 个） -->
    <Button class="w-full" :disabled="!canRun" @click="onPrimary">
      {{ batch.preview ? `确认执行（${previewCount} 个）` : '开始执行' }}
    </Button>

    <!-- 执行中提示（进度弹窗之外的兜底） -->
    <div v-if="batch.running" class="text-center text-[0.6875rem] text-muted-foreground">执行中…</div>

    <!-- 结果明细（完成后保留；失败列表可滚动） -->
    <div v-if="batch.result" class="space-y-1">
      <div class="text-[0.6875rem] font-medium text-muted-foreground">执行结果</div>
      <div class="text-[0.6875rem]" :class="batch.result.failed > 0 ? 'text-amber-500' : 'text-muted-foreground'">
        成功 {{ batch.result.success }} / 失败 {{ batch.result.failed }}
      </div>
      <div v-if="batch.errors.length" class="max-h-40 space-y-0.5 overflow-y-auto pr-1">
        <div
          v-for="(e, i) in batch.errors"
          :key="i"
          class="truncate text-[0.6875rem] leading-snug text-destructive"
          :title="e"
        >
          {{ e }}
        </div>
      </div>
    </div>
  </div>

  <!-- ── 目标目录一步式对话框 ── -->
  <Dialog v-model:open="showTargetDialog">
    <DialogContent class="sm:max-w-md">
      <DialogHeader>
        <DialogTitle>选择目标目录（{{ opLabel(batch.op) }}）</DialogTitle>
        <DialogDescription>
          目标目录不能与当前目录相同（当前：{{ captures.directory ?? '未打开目录' }}）。
        </DialogDescription>
      </DialogHeader>
      <div
        class="truncate rounded-md border bg-muted/50 px-2 py-1.5 tabular-nums text-xs"
        :title="batch.targetDir ?? ''"
      >
        {{ batch.targetDir ?? '未选择' }}
      </div>
      <DialogFooter class="justify-end" :show-close-button="false">
        <Button size="sm" variant="outline" @click="chooseTargetAndClose">
          <FolderOpenIcon data-icon="inline-start" />
          浏览…
        </Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>

  <!-- ── 删除红色确认框（前 20 条清单 + 同名同步警告） ── -->
  <Dialog v-model:open="showDeleteConfirm">
    <DialogContent class="sm:max-w-md">
      <DialogHeader>
        <DialogTitle class="flex items-center gap-1.5 text-destructive">
          <TriangleAlertIcon class="size-4" />
          删除确认
        </DialogTitle>
        <DialogDescription>
          将删除 {{ previewCount }} 个文件
          <template v-if="previewSiblingCount > 0">（其中 {{ previewSiblingCount }} 个来自同名同步）</template>
          ，删除后进入回收站，此操作不可撤销。
        </DialogDescription>
      </DialogHeader>
      <div class="max-h-48 space-y-0.5 overflow-y-auto rounded-md border bg-muted/50 p-2">
        <div
          v-for="(name, i) in deletePreviewNames"
          :key="i"
          class="truncate tabular-nums text-[0.6875rem]"
        >
          {{ name }}
        </div>
        <div v-if="previewCount > deletePreviewNames.length" class="text-[0.6875rem] text-muted-foreground">
          …等共 {{ previewCount }} 个文件
        </div>
      </div>
      <DialogFooter class="justify-between" :show-close-button="false">
        <DialogClose as-child>
          <Button size="sm" variant="outline">取消</Button>
        </DialogClose>
        <Button size="sm" variant="destructive" @click="onConfirmDelete">确认删除</Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>

  <!-- ── 执行进度弹窗（n/m · 当前文件） ── -->
  <Dialog :open="batch.running">
    <DialogContent class="sm:max-w-sm" :show-close-button="false">
      <DialogHeader>
        <DialogTitle>正在批量{{ opLabel(batch.op) }}…</DialogTitle>
        <DialogDescription>执行中请勿关闭应用</DialogDescription>
      </DialogHeader>
      <div class="h-1.5 w-full overflow-hidden rounded bg-muted">
        <div
          class="h-full bg-primary transition-[width]"
          :style="{ width: `${progressPct}%` }"
        />
      </div>
      <div class="truncate text-center tabular-nums text-xs text-muted-foreground">
        {{ progressText }}
      </div>
    </DialogContent>
  </Dialog>

  <!-- ── toast（瞬态提示：拒绝源目录 / 完成摘要 / 失败） ── -->
  <Teleport to="body">
    <div
      v-if="batch.toast"
      class="fixed top-2 left-1/2 z-[70] max-w-[80vw] -translate-x-1/2 rounded-lg border bg-popover px-3 py-1.5 text-xs shadow-lg"
    >
      {{ batch.toast }}
    </div>
  </Teleport>
</template>
