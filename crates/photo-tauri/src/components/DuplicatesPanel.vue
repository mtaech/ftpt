<script setup lang="ts">
// 重复照片面板：pHash 近重复检测（dHash → 汉明距离贪心聚类）结果浏览。
// 显隐由 duplicates store 管理（Sidebar 底部按钮打开；Esc/×/遮罩关闭），
// 参照 ExportDialog「自身 store 管理显隐」模式，App.vue 仅挂载组件。
// 「保留第一张，其余标 Rejected」走 captures store 现有旗标 mutation（applyFlag
// → set_flag → folder_db xmp_meta 真相表），不改动重复分组本身。
import { onMounted, ref, watch } from 'vue'
import { ScanSearchIcon, XIcon } from '@lucide/vue'
import { Dialog, DialogClose, DialogContent, DialogTitle } from '@/components/ui/dialog'
import { Button } from '@/components/ui/button'
import { useDuplicatesStore } from '@/stores/duplicates'
import { useCapturesStore } from '@/stores/captures'
import { ptimgUrl } from '@/lib/ipc'

const duplicates = useDuplicatesStore()
const captures = useCapturesStore()

/** 事件接线：面板常驻挂载（对齐 SettingsModal），启动即 listen，事件不丢失 */
onMounted(() => {
  duplicates.init()
})

/** 汉明距离阈值选项（越小越严格）：默认 10 对齐后端 DEFAULT_HASH_THRESHOLD */
const THRESHOLDS = [6, 8, 10, 12, 16] as const
const threshold = ref<number>(10)

/** 已执行「保留第一张」操作的组索引（本地状态，重测后自动清空） */
const handledGroups = ref<Set<number>>(new Set())
watch(
  () => duplicates.groups,
  () => handledGroups.value = new Set(),
)

function run() {
  void duplicates.run(threshold.value)
}

/** 保留组内第一张，其余标 Rejected（走现有旗标链路，网格/筛选即时生效） */
function keepFirst(group: string[], gi: number) {
  const rest = group.slice(1)
  if (rest.length === 0) return
  void captures.applyFlag(rest, 'Reject')
  handledGroups.value = new Set(handledGroups.value).add(gi)
}

/** 完整路径 → 文件名（缩略图 alt/标题） */
function fileName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path
}
</script>

<template>
  <Dialog :open="duplicates.open" @update:open="(v) => !v && duplicates.closePanel()">
    <DialogContent
      :show-close-button="false"
      class="flex h-[36rem] max-w-4xl flex-col gap-0 p-0 sm:max-w-[52rem]"
    >
      <!-- 头栏：标题 + 关闭按钮 -->
      <div class="flex shrink-0 items-center justify-between border-b px-4 py-3">
        <DialogTitle class="text-base font-semibold">重复照片</DialogTitle>
        <DialogClose as-child>
          <Button variant="ghost" size="icon-sm" aria-label="关闭">
            <XIcon />
          </Button>
        </DialogClose>
      </div>

      <!-- 工具栏：阈值 + 检测按钮 + 进度 / 错误 -->
      <div class="flex shrink-0 flex-wrap items-center gap-3 border-b px-4 py-2.5">
        <label class="flex items-center gap-1.5 text-xs text-muted-foreground">
          阈值
          <select
            v-model="threshold"
            class="rounded border border-border bg-background px-1.5 py-0.5 text-xs text-foreground"
            :disabled="duplicates.running"
          >
            <option v-for="t in THRESHOLDS" :key="t" :value="t">{{ t }}</option>
          </select>
        </label>
        <Button size="sm" :disabled="duplicates.running || !captures.directory" @click="run">
          <ScanSearchIcon data-icon="inline-start" />
          {{ duplicates.running ? '检测中…' : '开始检测' }}
        </Button>
        <div v-if="duplicates.running && duplicates.progress" class="text-xs text-muted-foreground">
          已哈希 {{ duplicates.progress.done }} / {{ duplicates.progress.total }}
        </div>
        <div v-if="duplicates.error" class="text-xs text-destructive">{{ duplicates.error }}</div>
      </div>

      <!-- 分组列表 -->
      <div class="min-h-0 flex-1 overflow-y-auto p-4">
        <!-- 空态：未检测 / 未发现重复 -->
        <div
          v-if="!duplicates.running && !duplicates.error && duplicates.groups.length === 0"
          class="flex h-full flex-col items-center justify-center gap-1 text-sm text-muted-foreground"
        >
          <ScanSearchIcon class="h-8 w-8 opacity-40" />
          <p>
            {{
              duplicates.hasRun
                ? '未发现重复照片'
                : '点击「开始检测」对当前目录全部照片计算内容级重复（dHash）'
            }}
          </p>
        </div>

        <div v-for="(group, gi) in duplicates.groups" :key="gi" class="panel-card mb-3 p-3">
          <!-- 组头：序号 + 张数 + 保留首张按钮 -->
          <div class="mb-2 flex items-center justify-between gap-2">
            <div class="text-xs font-medium">重复组 {{ gi + 1 }} · {{ group.length }} 张</div>
            <Button
              size="sm"
              variant="outline"
              :disabled="handledGroups.has(gi)"
              @click="keepFirst(group, gi)"
            >
              {{ handledGroups.has(gi) ? '已标 Rejected' : '保留第一张，其余标 Rejected' }}
            </Button>
          </div>
          <!-- 组内缩略图横排（ptimgUrl thumb；thumb:ready 后 ?v= 递增自动刷新） -->
          <div class="flex gap-2 overflow-x-auto pb-1">
            <figure
              v-for="(path, pi) in group"
              :key="path"
              class="flex w-28 shrink-0 flex-col gap-1"
            >
              <div class="relative aspect-[4/3] overflow-hidden rounded border border-border bg-muted">
                <img
                  :src="ptimgUrl('thumb', path, captures.thumbVersions[path])"
                  class="h-full w-full object-cover"
                  loading="lazy"
                  :alt="fileName(path)"
                />
                <span class="absolute left-1 top-1 rounded bg-black/60 px-1 text-[10px] text-white">
                  {{ pi + 1 }}
                </span>
                <span
                  v-if="pi === 0"
                  class="absolute right-1 top-1 rounded bg-primary px-1 text-[10px] text-primary-foreground"
                >
                  保留
                </span>
              </div>
              <figcaption class="truncate text-[10px] text-muted-foreground" :title="path">
                {{ fileName(path) }}
              </figcaption>
            </figure>
          </div>
        </div>
      </div>
    </DialogContent>
  </Dialog>
</template>
