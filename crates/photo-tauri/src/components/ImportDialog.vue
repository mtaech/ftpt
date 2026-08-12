<script setup lang="ts">
// 导入对话框（T1 批次 ImportRebuild）：SD 卡 → 按日期建目录 → 去重 → 复制/移动。
// 流程：选源（可移动盘列表 / 手动浏览）→ 扫描 → 选目标根目录 → 干跑计划预览
//       （N 张 → M 个日期目录，跳过清单前 20 条）→ 复制/移动 → 执行进度 → 结果明细。
// 对齐 SettingsModal 弹窗样式 + BatchOpsPanel 两阶段（预览→执行）交互。
import { computed, onMounted, ref, watch } from 'vue'
import { FolderOpenIcon, ScanSearchIcon, TriangleAlertIcon, XIcon } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/ui/dialog'
import {
  executeImport,
  listImportDrives,
  onImportDone,
  onImportProgress,
  pickDirectory,
  planImport,
  scanImportSource,
  type ImportDonePayload,
  type ImportProgressPayload,
} from '@/lib/ipc'
import type { ImportCandidate, ImportDrive, ImportMode, ImportPlan, ImportResult } from '@/lib/bindings'
import { useCapturesStore } from '@/stores/captures'

/** 弹窗开关（store 驱动：文件树 tab「导入」按钮打开、×/Esc/遮罩关闭） */
const open = defineModel<boolean>('open', { default: false })

const captures = useCapturesStore()

/** 顶部选项：导入（SD 卡 → 复制/移动）| 添加（选目录直接打开浏览，不动文件） */
const tab = ref<'import' | 'add'>('import')

/** 添加目录：系统选目录 → 直接打开（= 原「打开目录」逻辑），关闭弹窗 */
async function addDirectory() {
  const dir = await pickDirectory()
  if (!dir) return
  open.value = false
  await captures.openPath(dir)
}

// ── 状态 ──────────────────────────────────────────────

/** 检测到的可移动驱动器列表（打开时加载） */
const drives = ref<ImportDrive[]>([])
/** 已选源路径（驱动器根或手动浏览目录） */
const source = ref<string | null>(null)
/** 源扫描进行中 */
const scanning = ref(false)
/** 扫描出的候选文件 */
const candidates = ref<ImportCandidate[]>([])
/** 目标根目录（导入后生成 destRoot/YYYY-MM-DD/） */
const destRoot = ref<string | null>(null)
/** 执行模式：复制（源保留）/ 移动（源删除） */
const mode = ref<ImportMode>('Copy')
/** 干跑计划（plan_import 结果，不碰文件） */
const plan = ref<ImportPlan | null>(null)
/** 计划生成中 */
const planning = ref(false)
/** 执行中（进度显示依据） */
const running = ref(false)
/** 执行进度（import:progress 事件） */
const progress = ref<ImportProgressPayload | null>(null)
/** 最近一次执行结果 */
const result = ref<ImportResult | null>(null)
/** 瞬态 toast（4s 自动消失） */
const toast = ref<string | null>(null)
/** 事件是否已接线（防重复 listen） */
const listening = ref(false)

// ── 派生状态 ──────────────────────────────────────────

/** 计划文件总数（执行按钮文案） */
const planCount = computed(() =>
  (plan.value?.groups ?? []).reduce((acc, g) => acc + g.files.length, 0),
)
/** 跳过清单前 20 条（路径末段 + 原因） */
const skippedPreview = computed(() =>
  (plan.value?.skipped ?? []).slice(0, 20).map((s) => ({
    name: s.path.split(/[\\/]/).pop() ?? s.path,
    reason: s.reason,
  })),
)
/** 日期目录清单（前 10 组 + 每组张数） */
const groupPreview = computed(() =>
  (plan.value?.groups ?? []).slice(0, 10).map((g) => ({
    dateDir: g.dateDir,
    count: g.files.length,
  })),
)
const hasSkippedMore = computed(
  () => (plan.value?.skipped.length ?? 0) > skippedPreview.value.length,
)
const hasGroupMore = computed(() => (plan.value?.groups.length ?? 0) > groupPreview.value.length)

/** 进度文案：n/m · 当前文件 */
const progressText = computed(() => {
  const p = progress.value
  if (!p) return ''
  return `${p.done}/${p.total} · ${p.current.split(/[\\/]/).pop() ?? p.current}`
})
const progressPct = computed(() => {
  const p = progress.value
  if (!p || p.total <= 0) return 0
  return Math.min(100, Math.round((p.done / p.total) * 100))
})

// ── 交互 ──────────────────────────────────────────────

/** 打开时：复位 + 加载驱动器列表（对齐 SettingsModal watch(open) 模式） */
watch(open, (v) => {
  if (!v) return
  tab.value = 'import'
  source.value = null
  scanning.value = false
  candidates.value = []
  destRoot.value = null
  mode.value = 'Copy'
  plan.value = null
  planning.value = false
  running.value = false
  progress.value = null
  result.value = null
  void loadDrives()
})

async function loadDrives() {
  try {
    drives.value = await listImportDrives()
  } catch (e) {
    toast.value = `检测可移动驱动器失败：${String(e)}`
  }
}

/** 选中驱动器（源 = 根路径，Windows "E:\" / Linux 挂载点）后立即扫描 */
function selectDrive(d: ImportDrive) {
  if (scanning.value || running.value) return
  source.value = d.path
  void scan()
}

/** 手动浏览源目录（系统目录对话框；tauri-plugin-dialog 经 pick_directory command） */
async function browseSource() {
  if (scanning.value || running.value) return
  const dir = await pickDirectory()
  if (!dir) return
  source.value = dir
  void scan()
}

/** 递归扫描源（EXIF 日期优先，回退 mtime） */
async function scan() {
  if (!source.value || scanning.value || running.value) return
  scanning.value = true
  plan.value = null
  result.value = null
  try {
    candidates.value = await scanImportSource(source.value)
  } catch (e) {
    toast.value = `扫描导入源失败：${String(e)}`
  } finally {
    scanning.value = false
  }
}

/** 选择目标根目录（导入后生成 destRoot/YYYY-MM-DD/） */
async function chooseDest() {
  if (running.value) return
  const dir = await pickDirectory()
  if (!dir) return
  destRoot.value = dir
  plan.value = null
  result.value = null
}

/** 切换复制/移动（计划与模式无关，不失效） */
function setMode(m: ImportMode) {
  mode.value = m
}

/** 干跑计划：按日期分组 + 目标去重（不碰文件） */
async function generatePlan() {
  if (!source.value || !destRoot.value || planning.value || running.value) return
  planning.value = true
  result.value = null
  try {
    plan.value = await planImport(candidates.value, destRoot.value)
  } catch (e) {
    toast.value = `生成导入计划失败：${String(e)}`
  } finally {
    planning.value = false
  }
}

/** 执行导入：进度事件驱动进度条；完成后结果明细留在面板 */
async function execute() {
  if (!plan.value || !destRoot.value || running.value) return
  running.value = true
  result.value = null
  progress.value = { done: 0, total: planCount.value, current: '' }
  try {
    result.value = await executeImport(plan.value, destRoot.value, mode.value)
  } catch (e) {
    toast.value = `导入执行失败：${String(e)}`
  } finally {
    running.value = false
  }
}

/** 主按钮可用性：源已扫描 + 目标已选 + 非执行中 */
const canPlan = computed(
  () => !!source.value && !!destRoot.value && !planning.value && !running.value && !scanning.value,
)
const canExecute = computed(() => !!plan.value && !!destRoot.value && !running.value && planCount.value > 0)

// ── 生命周期：接线进度/完成事件（对齐 captures.init 模式）──
onMounted(() => {
  if (listening.value) return
  listening.value = true
  void onImportProgress((p) => {
    progress.value = p
  })
  void onImportDone((p: ImportDonePayload) => {
    // invoke 返回与事件同内容：事件先到则以事件为准（幂等）
    if (!result.value) result.value = { ...p }
  })
})

// ── toast 自动消失（4s，对齐 BatchOpsPanel 模式）──
let toastTimer: ReturnType<typeof setTimeout> | null = null
watch(toast, (t) => {
  if (toastTimer) clearTimeout(toastTimer)
  toastTimer = null
  if (!t) return
  toastTimer = setTimeout(() => (toast.value = null), 4000)
})
</script>

<template>
  <Dialog :open="open" @update:open="open = $event">
    <!-- 自绘头栏（× 按钮放标题右侧，对齐 SettingsModal 头栏样式） -->
    <DialogContent
      :show-close-button="false"
      class="flex max-h-[85vh] w-full max-w-xl flex-col gap-0 p-0 sm:max-w-xl"
    >
      <!-- 头栏：标题 + 关闭按钮 -->
      <div class="flex shrink-0 items-center justify-between border-b px-4 py-3">
        <DialogTitle class="text-base font-semibold">导入照片</DialogTitle>
        <DialogClose as-child>
          <Button variant="ghost" size="icon-sm" aria-label="关闭">
            <XIcon />
          </Button>
        </DialogClose>
      </div>

      <!-- 主体（可滚动） -->
      <div class="min-h-0 flex-1 space-y-4 overflow-y-auto p-4">
        <!-- ── 选项：导入（复制/移动到库）| 添加（直接打开目录浏览）── -->
        <div class="flex gap-1 rounded-md bg-element p-0.5">
          <button
            type="button"
            class="flex-1 rounded-sm py-1 text-center text-xs transition-colors select-none"
            :class="
              tab === 'import'
                ? 'bg-card font-medium text-foreground shadow-sm'
                : 'text-muted-foreground hover:text-foreground'
            "
            :aria-pressed="tab === 'import'"
            @click="tab = 'import'"
          >
            导入
          </button>
          <button
            type="button"
            class="flex-1 rounded-sm py-1 text-center text-xs transition-colors select-none"
            :class="
              tab === 'add'
                ? 'bg-card font-medium text-foreground shadow-sm'
                : 'text-muted-foreground hover:text-foreground'
            "
            :aria-pressed="tab === 'add'"
            @click="tab = 'add'"
          >
            添加
          </button>
        </div>

        <!-- ── 添加：选目录直接打开（不复制/移动文件）── -->
        <div v-if="tab === 'add'" class="space-y-3">
          <p class="text-xs leading-relaxed text-muted-foreground">
            把本机已有目录加入照片库直接浏览，文件保持原位，不复制、不移动。
          </p>
          <Button size="sm" @click="addDirectory">
            <FolderOpenIcon data-icon="inline-start" />
            选择目录…
          </Button>
        </div>

        <template v-else>
        <!-- ── 源位置：可移动盘列表 + 手动浏览 ── -->
        <div class="space-y-1.5">
          <div class="text-xs font-medium text-muted-foreground">源位置（SD 卡 / 目录）</div>
          <div v-if="drives.length" class="flex flex-wrap gap-1">
            <button
              v-for="d in drives"
              :key="d.path"
              class="rounded-md border px-2.5 py-1 text-xs transition-colors disabled:pointer-events-none disabled:opacity-50"
              :class="
                source === d.path
                  ? 'border-primary bg-primary text-primary-foreground'
                  : 'hover:bg-accent hover:text-accent-foreground'
              "
              :disabled="scanning || running"
              @click="selectDrive(d)"
            >
              {{ d.label ?? '可移动磁盘' }}（{{ d.path }}）
            </button>
            <Button
              size="xs"
              variant="outline"
              :disabled="scanning || running"
              @click="browseSource"
            >
              <FolderOpenIcon data-icon="inline-start" />
              浏览…
            </Button>
          </div>
          <div v-else class="flex items-center gap-1.5 rounded-md border bg-muted/40 px-2 py-1.5">
            <span class="text-xs text-muted-foreground">未检测到可移动驱动器</span>
            <Button size="xs" variant="outline" :disabled="scanning || running" @click="browseSource">
              <FolderOpenIcon data-icon="inline-start" />
              浏览目录…
            </Button>
          </div>
          <div v-if="source" class="flex items-center gap-1.5">
            <div
              class="min-w-0 flex-1 truncate rounded-md border bg-muted/40 px-2 py-1 tabular-nums text-[0.6875rem]"
              :title="source"
            >
              {{ source }}
            </div>
            <Button size="xs" variant="ghost" :disabled="scanning || running" @click="scan">
              <ScanSearchIcon data-icon="inline-start" />
              {{ scanning ? '扫描中…' : '重新扫描' }}
            </Button>
          </div>
          <div v-if="!scanning && candidates.length" class="text-[0.6875rem] text-muted-foreground">
            已扫描 {{ candidates.length }} 个候选文件（图片 / RAW / 视频）
          </div>
        </div>

        <!-- ── 目标根目录 ── -->
        <div class="space-y-1.5">
          <div class="text-xs font-medium text-muted-foreground">目标根目录（按日期建子目录）</div>
          <div class="flex items-center gap-1.5">
            <div
              class="min-w-0 flex-1 truncate rounded-md border bg-muted/40 px-2 py-1 tabular-nums text-[0.6875rem]"
              :class="destRoot ? 'text-foreground' : 'text-muted-foreground'"
              :title="destRoot ?? ''"
            >
              {{ destRoot ?? '未选择目标根目录' }}
            </div>
            <Button size="xs" variant="outline" :disabled="running" @click="chooseDest">选择…</Button>
          </div>
        </div>

        <!-- ── 模式：复制 / 移动（对齐 BatchOpsPanel 操作类型按钮） ── -->
        <div class="space-y-1.5">
          <div class="text-xs font-medium text-muted-foreground">导入方式</div>
          <div class="flex gap-1">
            <Button
              size="sm"
              class="flex-1"
              :variant="mode === 'Copy' ? 'default' : 'outline'"
              :disabled="running"
              @click="setMode('Copy')"
            >
              复制
            </Button>
            <Button
              size="sm"
              class="flex-1"
              :variant="mode === 'Move' ? 'default' : 'outline'"
              :disabled="running"
              @click="setMode('Move')"
            >
              移动
            </Button>
          </div>
          <div class="px-1 text-[0.6875rem] text-muted-foreground">
            {{ mode === 'Copy' ? '复制：源文件保留' : '移动：完成后删除源文件（跨设备自动回退为复制+删除）' }}
          </div>
        </div>

        <!-- ── 干跑计划预览 ── -->
        <div class="space-y-1.5">
          <div class="text-xs font-medium text-muted-foreground">计划预览（不碰文件）</div>
          <Button class="w-full" :disabled="!canPlan" @click="generatePlan">
            {{ planning ? '生成中…' : plan ? '重新生成计划' : '生成计划预览' }}
          </Button>

          <div v-if="plan" class="space-y-2 rounded-md border bg-muted/30 p-2">
            <div class="text-[0.6875rem] font-medium text-foreground">
              {{ candidates.length }} 张 → {{ plan.groups.length }} 个日期目录
              <template v-if="plan.skipped.length">，跳过 {{ plan.skipped.length }} 张</template>
            </div>
            <!-- 日期目录清单（前 10 组） -->
            <div v-if="groupPreview.length" class="max-h-28 space-y-0.5 overflow-y-auto pr-1">
              <div
                v-for="g in groupPreview"
                :key="g.dateDir"
                class="flex justify-between rounded-sm bg-muted/50 px-2 py-0.5 text-[0.6875rem] tabular-nums"
              >
                <span>{{ g.dateDir }}</span>
                <span class="text-muted-foreground">{{ g.count }} 张</span>
              </div>
              <div v-if="hasGroupMore" class="text-[0.6875rem] text-muted-foreground">
                …等共 {{ plan.groups.length }} 个日期目录
              </div>
            </div>
            <!-- 跳过清单（前 20 条） -->
            <div v-if="skippedPreview.length">
              <div class="text-[0.6875rem] text-amber-500">跳过 {{ plan.skipped.length }} 张：</div>
              <div class="max-h-32 space-y-0.5 overflow-y-auto pr-1">
                <div
                  v-for="(s, i) in skippedPreview"
                  :key="i"
                  class="truncate rounded-sm bg-muted/50 px-2 py-0.5 text-[0.6875rem] leading-snug"
                  :title="`${s.name}（${s.reason}）`"
                >
                  <span class="tabular-nums">{{ s.name }}</span>
                  <span class="text-muted-foreground"> — {{ s.reason }}</span>
                </div>
                <div v-if="hasSkippedMore" class="text-[0.6875rem] text-muted-foreground">
                  …等共 {{ plan.skipped.length }} 张
                </div>
              </div>
            </div>
            <div v-else class="text-[0.6875rem] text-muted-foreground">无跳过（无同名同大小冲突）</div>
          </div>
        </div>

        <!-- ── 执行 ── -->
        <div class="space-y-1.5">
          <Button class="w-full" :disabled="!canExecute" @click="execute">
            {{ running ? '导入中…' : `开始导入（${planCount} 张）` }}
          </Button>

          <!-- 进度条 -->
          <div v-if="running" class="space-y-1">
            <div class="h-1.5 w-full overflow-hidden rounded bg-muted">
              <div class="h-full bg-primary transition-[width]" :style="{ width: `${progressPct}%` }" />
            </div>
            <div class="truncate text-center tabular-nums text-xs text-muted-foreground">
              {{ progressText }}
            </div>
          </div>

          <!-- 结果明细（完成后保留） -->
          <div v-if="result" class="space-y-0.5 rounded-md border bg-muted/30 p-2 text-[0.6875rem]">
            <div class="font-medium text-muted-foreground">导入结果</div>
            <div>
              成功 {{ result.imported }} / 跳过 {{ result.skipped }}
              <span :class="result.failed > 0 ? 'text-amber-500' : ''">/ 失败 {{ result.failed }}</span>
            </div>
            <div v-if="result.failed > 0" class="flex items-center gap-1 text-amber-500">
              <TriangleAlertIcon class="size-3" />
              部分文件失败，请检查目标目录权限或磁盘空间
            </div>
          </div>
        </div>
        </template>
      </div>

      <!-- 底部：目标目录提示（对齐 BatchOpsPanel 对话框描述风格） -->
      <DialogDescription class="sr-only">
        从 {{ source ?? '源' }} 导入照片到 {{ destRoot ?? '目标根目录' }}（按拍摄日期建子目录，同名同大小自动跳过）
      </DialogDescription>
    </DialogContent>
  </Dialog>

  <!-- toast（瞬态提示：扫描失败 / 计划失败 / 执行失败） -->
  <Teleport to="body">
    <div
      v-if="toast"
      class="fixed top-2 left-1/2 z-[70] max-w-[80vw] -translate-x-1/2 rounded-lg border bg-popover px-3 py-1.5 text-xs shadow-lg"
    >
      {{ toast }}
    </div>
  </Teleport>
</template>
