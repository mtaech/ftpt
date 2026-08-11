<script setup lang="ts">
// 统计视图（SpeciesIndex T1 批次）：全局鸟种索引（跨文件夹聚合，数据源 exe 同级
// data/global.db）。布局：顶部汇总条（鸟种数/照片数/文件夹数）+ 左栏鸟种列表
// （名称/张数/首见日期/平均锐度，搜索框过滤）+ 右栏选中鸟种照片网格。
// 交互：单击鸟种 → 右栏照片；双击照片 → 切到所在目录并选中该张（切目录复用
// captures.openPath 扫描流程，选中经 selection store）；Esc/G/退出按钮关闭
// （视图路由在 preview store，对齐 compare 模式语义）。
import { computed, onMounted, ref, watch } from 'vue'
import { ChevronDownIcon, ImageOffIcon, SearchIcon, XIcon } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import { useCapturesStore } from '@/stores/captures'
import { useSelectionStore } from '@/stores/selection'
import { usePreviewStore } from '@/stores/preview'
import { useStatsStore } from '@/stores/stats'
import { ptimgUrl } from '@/lib/ipc'
import type { SpeciesPhoto } from '@/lib/bindings'

const captures = useCapturesStore()
const selection = useSelectionStore()
const preview = usePreviewStore()
const stats = useStatsStore()

/** 退出统计视图：复位本地态（下次进入重新拉取，识别/删除后数据不过期） */
function exitStats() {
  stats.clear()
  preview.closeStats()
}

/** 照片绝对路径（folder + '/' + relPath；folder 为后端完整路径，Windows 反斜杠） */
function absPath(p: SpeciesPhoto): string {
  return `${p.folder}/${p.relPath}`
}

/** 路径比较（归一化分隔符：后端 primaryPath 为反斜杠，relPath 拼接为正斜杠） */
function samePath(a: string, b: string): boolean {
  return a.replace(/\\/g, '/') === b.replace(/\\/g, '/')
}

/** 首见/末见日期显示：取 ISO/EXIF 日期串前 10 位（YYYY-MM-DD） */
function shortDate(s: string | null): string {
  if (!s) return '—'
  return s.length >= 10 ? s.slice(0, 10) : s
}

/** 平均锐度显示：一位小数；无锐度数据（全 NULL）显示 — */
function sharpText(v: number | null): string {
  return v === null ? '—' : v.toFixed(1)
}

/** 选中鸟种（单击左栏条目） */
function pickSpecies(name: string) {
  void stats.selectSpecies(name)
}

/**
 * 双击照片：切到所在目录并选中该张（任务契约）。
 * 同目录 → 直接按主路径定位选中；跨目录 → captures.openPath 复用现有扫描流程，
 * 扫描完成（scan:done → reload）后按主路径定位。完成后退出统计视图回到网格，
 * 让选中态可见（对齐「切目录 + 选中」语义）。
 */
async function jumpToPhoto(p: SpeciesPhoto) {
  const path = absPath(p)
  if (captures.directory !== p.folder) {
    await captures.openPath(p.folder)
  }
  const idx = captures.items.findIndex((c) => samePath(c.primaryPath, path))
  if (idx >= 0) selection.select(idx)
  exitStats()
}

/** 缩略图加载失败的路径集合（img error 后显示占位图标，成功加载的不遮挡） */
const failedPaths = ref<Set<string>>(new Set())

/** 识别命中率区块展开态（默认展开；表在鸟种列表下方，折叠省出列表空间） */
const accuracyOpen = ref(true)

/** 缩略图加载失败（文件被移走/无缓存且后端降级）：登记后显示占位图标 */
function onThumbError(path: string, e: Event) {
  failedPaths.value = new Set(failedPaths.value).add(path)
  ;(e.target as HTMLImageElement).style.display = 'none'
}

/** 切换鸟种时清空失败登记（避免旧鸟种路径残留） */
watch(
  () => stats.photos,
  () => {
    failedPaths.value = new Set()
  },
)

onMounted(() => {
  void stats.load()
})

// 空态提示（无全局库数据时也显示，避免白屏）
const hasAny = computed(() => stats.overview.stats.length > 0)
</script>

<template>
  <div class="flex h-full flex-col bg-background">
    <!-- 顶部汇总条 -->
    <div class="flex shrink-0 items-center gap-4 border-b border-border bg-card px-3 py-1.5 text-sm">
      <span class="font-medium">统计视图</span>
      <span class="text-muted-foreground tabular-nums">鸟种 {{ stats.overview.stats.length }}</span>
      <span class="text-muted-foreground tabular-nums">照片 {{ stats.totalPhotos }}</span>
      <span class="text-muted-foreground tabular-nums">文件夹 {{ stats.overview.folderCount }}</span>
      <div class="ml-auto">
        <Button size="sm" variant="ghost" title="退出统计视图 (Esc / G)" @click="exitStats">
          <XIcon class="size-3.5" />
          退出
        </Button>
      </div>
    </div>

    <div class="flex min-h-0 flex-1">
      <!-- 左栏：搜索 + 鸟种列表 -->
      <aside class="flex w-64 shrink-0 flex-col border-r border-border">
        <div class="relative shrink-0 border-b border-border p-1.5">
          <SearchIcon class="pointer-events-none absolute left-3 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
          <input
            v-model="stats.search"
            type="text"
            placeholder="搜索鸟种"
            class="w-full rounded-md border border-border bg-background py-1 pl-7 pr-2 text-sm outline-none placeholder:text-muted-foreground/60 focus:border-primary"
          />
        </div>
        <div class="min-h-0 flex-1 overflow-y-auto">
          <!-- 空态：无全局索引数据 -->
          <div
            v-if="!hasAny && !stats.loading"
            class="flex h-full flex-col items-center justify-center gap-2 px-4 text-center text-xs text-muted-foreground"
          >
            <div>暂无鸟种数据</div>
            <div class="leading-relaxed">识别照片后（B / Ctrl+B）自动汇总</div>
          </div>
          <!-- 鸟种列表（张数降序，后端排序；搜索词过滤） -->
          <button
            v-for="s in stats.filteredStats"
            :key="s.birdName"
            class="flex w-full items-center gap-2 border-b border-border/60 px-2.5 py-1.5 text-left transition-colors hover:bg-accent/60"
            :class="stats.selectedSpecies === s.birdName ? 'bg-accent text-accent-foreground' : ''"
            @click="pickSpecies(s.birdName)"
          >
            <span class="min-w-0 flex-1 truncate text-sm" :title="s.birdName">{{ s.birdName }}</span>
            <span class="shrink-0 text-xs tabular-nums text-muted-foreground">{{ s.photoCount }} 张</span>
            <span class="hidden shrink-0 text-xs tabular-nums text-muted-foreground lg:inline">
              {{ shortDate(s.firstDate) }}
            </span>
            <span class="hidden shrink-0 text-xs tabular-nums text-muted-foreground lg:inline">
              锐 {{ sharpText(s.avgSharpness) }}
            </span>
          </button>
        </div>
        <!-- 识别命中率（T 批次 Wave 2）：按鸟种聚合，命中率升序（弱的在前）便于优先复核。
             样本 < 3 张标注「样本少」；折叠省出鸟种列表空间 -->
        <div class="shrink-0 border-t border-border">
          <button
            type="button"
            class="flex w-full items-center justify-between px-2.5 py-1.5 text-xs font-medium text-muted-foreground transition-colors hover:text-foreground"
            @click="accuracyOpen = !accuracyOpen"
          >
            <span>识别命中率</span>
            <ChevronDownIcon
              class="size-3.5 transition-transform"
              :class="accuracyOpen ? '' : '-rotate-90'"
            />
          </button>
          <div v-if="accuracyOpen" class="max-h-44 overflow-y-auto border-t border-border/60">
            <table class="w-full text-xs">
              <thead>
                <tr class="text-muted-foreground/70">
                  <th class="px-2.5 py-1 text-left font-normal">鸟种</th>
                  <th class="py-1 text-right font-normal tabular-nums">预测</th>
                  <th class="py-1 text-right font-normal tabular-nums">被改</th>
                  <th class="px-2.5 py-1 text-right font-normal tabular-nums">命中率</th>
                </tr>
              </thead>
              <tbody>
                <tr
                  v-for="s in stats.correctionSorted"
                  :key="s.birdName"
                  class="border-t border-border/40 hover:bg-accent/40"
                >
                  <td class="max-w-28 truncate px-2.5 py-1" :title="s.birdName">
                    {{ s.birdName }}
                    <span
                      v-if="s.predictedCount < 3"
                      class="ml-1 shrink-0 rounded-sm bg-muted px-1 py-px align-middle text-[9px] leading-none text-muted-foreground"
                      title="样本不足 3 张，命中率仅供参考"
                    >样本少</span>
                  </td>
                  <td class="py-1 text-right tabular-nums text-muted-foreground">{{ s.predictedCount }}</td>
                  <td class="py-1 text-right tabular-nums text-muted-foreground">{{ s.correctedAwayCount }}</td>
                  <td class="px-2.5 py-1 text-right tabular-nums">{{ Math.round((s.accuracy ?? 0) * 100) }}%</td>
                </tr>
                <tr v-if="stats.correctionStats.length === 0 && !stats.loading">
                  <td colspan="4" class="px-2.5 py-2 text-center text-muted-foreground/70">暂无命中率数据</td>
                </tr>
              </tbody>
            </table>
          </div>
        </div>
      </aside>

      <!-- 右栏：选中鸟种照片网格（只读浏览，双击跳转） -->
      <div class="min-h-0 flex-1 overflow-y-auto p-2">
        <div
          v-if="!stats.selectedSpecies"
          class="flex h-full flex-col items-center justify-center gap-2 text-sm text-muted-foreground"
        >
          <ImageOffIcon class="size-10 text-muted-foreground/30" />
          <div>选择左侧鸟种查看照片</div>
        </div>
        <div v-else-if="stats.photos.length === 0" class="flex h-full items-center justify-center text-sm text-muted-foreground">
          该鸟种暂无照片
        </div>
        <div v-else class="grid grid-cols-4 gap-1.5">
          <div
            v-for="p in stats.photos"
            :key="absPath(p)"
            class="group relative flex aspect-square flex-col overflow-hidden rounded-md border border-border bg-card shadow-sm transition-colors select-none hover:border-primary/50"
            :title="absPath(p)"
            @dblclick="jumpToPhoto(p)"
          >
            <img
              :src="ptimgUrl('thumb', absPath(p))"
              :alt="p.relPath"
              class="h-full w-full object-cover"
              loading="lazy"
              draggable="false"
              @error="onThumbError(absPath(p), $event)"
            />
            <!-- 缩略图缺失占位（img error 后显示；点击同样跳转） -->
            <div
              v-if="failedPaths.has(absPath(p))"
              class="absolute inset-0 flex items-center justify-center bg-muted"
              @dblclick.stop="jumpToPhoto(p)"
            >
              <ImageOffIcon class="size-6 text-muted-foreground/40" />
            </div>
            <div
              class="pointer-events-none absolute inset-x-0 bottom-0 truncate bg-gradient-to-t from-black/60 to-transparent px-1.5 pb-0.5 pt-3 text-[10px] text-white opacity-0 transition-opacity group-hover:opacity-100"
            >
              {{ p.relPath }}
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
