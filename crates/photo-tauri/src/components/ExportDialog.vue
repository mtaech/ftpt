<script setup lang="ts">
// 导出对话框（T1 批次：导出预设 + 命名模板）：
//   预设下拉（新建/保存/删除，持久化到 AppConfig.exportPresets）、
//   长边像素输入（空 = 原尺寸）、JPEG 质量滑杆、命名模板输入（实时预览第一张渲染结果）、
//   目标目录选择；执行走 exportCaptures command + export:progress/done 事件（进度弹窗）。
// 入口：图片右键菜单「导出…」+ 批量操作面板「导出」按钮（store.open(paths)）。
import { onMounted, watch } from 'vue'
import { FolderOpenIcon, Trash2Icon, XIcon } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/ui/dialog'
import { useExportStore } from '@/stores/export'

const exportStore = useExportStore()

// ── toast 自动消失（4s，对齐 BatchOpsPanel 模式）──
let toastTimer: ReturnType<typeof setTimeout> | null = null
watch(
  () => exportStore.toast,
  (t) => {
    if (toastTimer) clearTimeout(toastTimer)
    toastTimer = null
    if (!t) return
    toastTimer = setTimeout(() => (exportStore.toast = null), 4000)
  },
)

// ── 生命周期：接线 export 进度事件 ──
onMounted(() => {
  exportStore.init()
})

/** 占位符说明（模板输入框下方提示） */
const PLACEHOLDER_HELP = [
  '{name} 原名（无扩展名）',
  '{species} 鸟种名',
  '{date} 拍摄日期 YYYYMMDD',
  '{seq} 序号（补零 3 位）',
  '{camera} 相机型号',
].join(' · ')
</script>

<template>
  <Dialog :open="exportStore.open" @update:open="(v: boolean) => !v && exportStore.closeDialog()">
    <DialogContent
      :show-close-button="false"
      class="flex max-h-[85vh] w-[32rem] flex-col gap-0 p-0 sm:max-w-[32rem]"
    >
      <!-- 头栏 -->
      <div class="flex shrink-0 items-center justify-between border-b px-4 py-3">
        <DialogTitle class="text-base font-semibold">
          导出照片（{{ exportStore.paths.length }} 张）
        </DialogTitle>
        <DialogClose as-child>
          <Button variant="ghost" size="icon-sm" aria-label="关闭" @click="exportStore.closeDialog()">
            <XIcon />
          </Button>
        </DialogClose>
      </div>

      <div class="min-h-0 flex-1 space-y-4 overflow-y-auto p-4">
        <DialogDescription class="sr-only">
          选择导出预设与命名模板，批量导出 JPEG 到目标目录
        </DialogDescription>

        <!-- 预设管理：下拉 + 新建/保存/删除 -->
        <div class="space-y-1.5">
          <label class="text-sm font-medium">导出预设</label>
          <div class="flex items-center gap-1.5">
            <select
              :value="exportStore.presetIndex"
              class="h-8 min-w-0 flex-1 rounded-md border border-input bg-background px-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
              :disabled="exportStore.presets.length === 0"
              @change="
                (e: Event) => exportStore.applyPreset(Number((e.target as HTMLSelectElement).value))
              "
            >
              <option v-if="exportStore.presets.length === 0" :value="-1" disabled>
                暂无预设（可直接填写下方参数）
              </option>
              <option
                v-for="(p, i) in exportStore.presets"
                :key="p.name"
                :value="i"
                :selected="i === exportStore.presetIndex"
              >
                {{ p.name }}
              </option>
            </select>
            <Button variant="outline" size="sm" @click="exportStore.newPreset()">新建</Button>
            <Button
              variant="outline"
              size="sm"
              :disabled="exportStore.presetIndex < 0"
              @click="exportStore.savePreset()"
            >
              保存
            </Button>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label="删除预设"
              :disabled="exportStore.presetIndex < 0"
              @click="exportStore.deletePreset()"
            >
              <Trash2Icon />
            </Button>
          </div>
          <input
            v-model="exportStore.presetName"
            placeholder="预设名称（保存到配置）"
            class="h-8 w-full rounded-md border border-input bg-background px-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
          />
        </div>

        <!-- 长边 + 质量 -->
        <div class="grid grid-cols-2 gap-3">
          <div class="space-y-1.5">
            <label class="text-sm font-medium">长边像素</label>
            <input
              v-model="exportStore.longEdge"
              type="number"
              min="1"
              placeholder="原尺寸"
              class="h-8 w-full rounded-md border border-input bg-background px-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
            />
            <p class="text-[0.6875rem] text-muted-foreground">留空 = 不缩放</p>
          </div>
          <div class="space-y-1.5">
            <label class="text-sm font-medium">
              JPEG 质量：{{ exportStore.qualityParam }}
            </label>
            <input
              v-model.number="exportStore.quality"
              type="range"
              min="1"
              max="100"
              class="h-8 w-full accent-primary"
            />
          </div>
        </div>

        <!-- 命名模板 + 实时预览 -->
        <div class="space-y-1.5">
          <label class="text-sm font-medium">命名模板</label>
          <input
            v-model="exportStore.template"
            placeholder="{name}_{seq}"
            class="h-8 w-full rounded-md border border-input bg-background px-2 font-mono text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
          />
          <p class="text-[0.6875rem] leading-snug text-muted-foreground">{{ PLACEHOLDER_HELP }}</p>
          <div
            class="rounded-md bg-muted/50 px-2 py-1 font-mono text-xs text-foreground"
            :class="{ 'text-muted-foreground': !exportStore.previewName }"
          >
            预览：{{ exportStore.previewName || '（无照片可预览）' }}.jpg
          </div>
        </div>

        <!-- 目标目录 -->
        <div class="space-y-1.5">
          <label class="text-sm font-medium">目标目录</label>
          <div class="flex items-center gap-1.5">
            <div
              class="h-8 min-w-0 flex-1 truncate rounded-md border border-input bg-background px-2 py-1.5 text-sm text-muted-foreground"
              :class="{ 'text-foreground': exportStore.targetDir }"
              :title="exportStore.targetDir ?? ''"
            >
              {{ exportStore.targetDir ?? '未选择' }}
            </div>
            <Button variant="outline" size="sm" @click="exportStore.chooseTarget()">
              <FolderOpenIcon class="size-4" />
              浏览…
            </Button>
          </div>
        </div>
      </div>

      <!-- 底部：执行按钮（目标目录缺失禁用） -->
      <div class="shrink-0 border-t p-3">
        <Button class="w-full" :disabled="!exportStore.targetDir" @click="exportStore.runExport()">
          {{ exportStore.running ? '导出中…' : '开始导出' }}
        </Button>
      </div>
    </DialogContent>
  </Dialog>

  <!-- ── 执行进度弹窗（export:progress/done 驱动） ── -->
  <Dialog :open="exportStore.running">
    <DialogContent :show-close-button="false" class="w-96">
      <DialogTitle class="text-sm font-semibold">导出中…</DialogTitle>
      <div class="space-y-2">
        <div class="flex items-center justify-between text-xs text-muted-foreground">
          <span>{{ exportStore.progressText || '准备中…' }}</span>
        </div>
        <div class="h-1.5 w-full overflow-hidden rounded-full bg-muted">
          <div
            class="h-full rounded-full bg-primary transition-all"
            :style="{
              width: exportStore.progress && exportStore.progress.total > 0
                ? `${Math.min(100, Math.round((exportStore.progress.done / exportStore.progress.total) * 100))}%`
                : '0%',
            }"
          />
        </div>
      </div>
    </DialogContent>
  </Dialog>

  <!-- ── toast ── -->
  <Teleport to="body">
    <div
      v-if="exportStore.toast"
      class="fixed bottom-4 left-1/2 z-[200] -translate-x-1/2 rounded-md bg-popover px-3 py-1.5 text-sm text-popover-foreground shadow-lg"
    >
      {{ exportStore.toast }}
    </div>
  </Teleport>
</template>
