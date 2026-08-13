<script setup lang="ts">
// 统计视图（SpeciesIndex T1 批次）：全局鸟种索引（跨文件夹聚合，数据源 exe 同级
// data/global.db）。布局：顶部统计卡（鸟种数/照片总数/文件夹数/平均命中率）+
// 左栏鸟种排行（排名徽标 + 张数占比条 + 首见~末见跨度 + 平均锐度，搜索框过滤）+
// 识别命中率条形图（弱项在前，按阈值绿/琥珀/红着色）+ 右栏选中鸟种照片网格。
// 交互：单击鸟种 → 右栏照片；双击照片 → 切到所在目录并选中该张（切目录复用
// captures.openPath 扫描流程，选中经 selection store）；Esc/G/退出按钮关闭
// （视图路由在 preview store，对齐 compare 模式语义）。
import { computed, onMounted, ref, watch } from 'vue'
import {
  BirdIcon,
  ChevronDownIcon,
  FolderIcon,
  ImageIcon,
  ImageOffIcon,
  SearchIcon,
  TargetIcon,
  XIcon,
} from '@lucide/vue'
import { Button } from '@/components/ui/button'
import { useCapturesStore } from '@/stores/captures'
import { useSelectionStore } from '@/stores/selection'
import { usePreviewStore } from '@/stores/preview'
import { useStatsStore } from '@/stores/stats'
import { ptimgUrl } from '@/lib/ipc'
import type { SpeciesPhoto, SpeciesStat } from '@/lib/bindings'

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

/** 首见~末见观测跨度（同日只显示单个日期） */
function dateRange(s: SpeciesStat): string {
  const f = shortDate(s.firstDate)
  const l = shortDate(s.lastDate)
  return f === l ? f : `${f} ~ ${l}`
}

/** 张数占比条宽度（相对全库最大张数；最小值 4% 保证小样本可见） */
function barPct(count: number): string {
  return `${Math.max((count / stats.maxPhotoCount) * 100, 4)}%`
}

/** 命中率占比条宽度（null → 0；0% 命中也给 4% 红条保持可见） */
function accPct(v: number | null): string {
  if (v === null) return '0%'
  return `${Math.max(v * 100, 4)}%`
}

/** 命中率条填充色：≥80% 绿 / ≥50% 琥珀 / 其余红；无数据灰 */
function accClass(v: number | null): string {
  if (v === null) return 'bg-muted-foreground/40'
  if (v >= 0.8) return 'bg-success'
  if (v >= 0.5) return 'bg-warning'
  return 'bg-destructive'
}

/** 命中率文字色（与条同阈值；百分比数字更醒目） */
function accTextClass(v: number | null): string {
  if (v === null) return 'text-muted-foreground'
  if (v >= 0.8) return 'text-success'
  if (v >= 0.5) return 'text-warning'
  return 'text-destructive'
}

/** 命中率百分比文本（null → —） */
function pct(v: number | null): string {
  return v === null ? '—' : `${Math.round(v * 100)}%`
}

/** 排名徽标：第一名用评级琥珀色，其余弱化（避免金/银/铜俗套） */
function rankClass(i: number): string {
  return i === 0 ? 'text-rating' : 'text-muted-foreground/50'
}

/** 平均命中率卡图标底色（同命中率阈值配色） */
const overallAccClass = computed(() => {
  const v = stats.overallAccuracy
  if (v === null) return 'bg-muted text-muted-foreground'
  if (v >= 0.8) return 'bg-success/10 text-success'
  if (v >= 0.5) return 'bg-warning/10 text-warning'
  return 'bg-destructive/10 text-destructive'
})

/** 选中鸟种覆盖文件夹数（右栏标题元信息） */
const selectedFolderCount = computed(() => new Set(stats.photos.map((p) => p.folder)).size)

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

/** 识别命中率区块展开态（默认展开；条形图在鸟种列表下方，折叠省出列表空间） */
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
    <!-- 顶部统计卡：鸟种 / 照片 / 文件夹 / 平均命中率 -->
    <div class="flex shrink-0 items-center gap-3 border-b border-border bg-card px-3 py-2">
      <div class="grid min-w-0 flex-1 grid-cols-2 gap-x-4 gap-y-2 sm:grid-cols-4">
        <div class="flex min-w-0 items-center gap-2.5">
          <div class="flex size-8 shrink-0 items-center justify-center rounded-md bg-primary/10 text-primary">
            <BirdIcon class="size-4" />
          </div>
          <div class="min-w-0">
            <div class="m3-label-small text-muted-foreground">鸟种</div>
            <div class="text-lg font-semibold leading-tight tabular-nums">{{ stats.overview.stats.length }}</div>
          </div>
        </div>
        <div class="flex min-w-0 items-center gap-2.5">
          <div class="flex size-8 shrink-0 items-center justify-center rounded-md bg-primary/10 text-primary">
            <ImageIcon class="size-4" />
          </div>
          <div class="min-w-0">
            <div class="m3-label-small text-muted-foreground">照片</div>
            <div class="text-lg font-semibold leading-tight tabular-nums">{{ stats.totalPhotos }}</div>
          </div>
        </div>
        <div class="flex min-w-0 items-center gap-2.5">
          <div class="flex size-8 shrink-0 items-center justify-center rounded-md bg-primary/10 text-primary">
            <FolderIcon class="size-4" />
          </div>
          <div class="min-w-0">
            <div class="m3-label-small text-muted-foreground">文件夹</div>
            <div class="text-lg font-semibold leading-tight tabular-nums">{{ stats.overview.folderCount }}</div>
          </div>
        </div>
        <div class="flex min-w-0 items-center gap-2.5">
          <div class="flex size-8 shrink-0 items-center justify-center rounded-md" :class="overallAccClass">
            <TargetIcon class="size-4" />
          </div>
          <div class="min-w-0">
            <div class="m3-label-small text-muted-foreground">识别命中率</div>
            <div class="text-lg font-semibold leading-tight tabular-nums">{{ pct(stats.overallAccuracy) }}</div>
          </div>
        </div>
      </div>
      <div class="shrink-0">
        <Button size="sm" variant="ghost" title="退出统计视图 (Esc / G)" @click="exitStats">
          <XIcon class="size-3.5" />
          退出
        </Button>
      </div>
    </div>

    <div class="flex min-h-0 flex-1">
      <!-- 左栏：搜索 + 鸟种排行 + 命中率条形图 -->
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
            <BirdIcon class="size-8 text-muted-foreground/30" />
            <div>暂无鸟种数据</div>
            <div class="leading-relaxed">识别照片后（B / Ctrl+B）自动汇总</div>
          </div>
          <!-- 搜索无匹配 -->
          <div v-else-if="stats.filteredStats.length === 0" class="flex h-full items-center justify-center text-xs text-muted-foreground">
            无匹配鸟种
          </div>
          <!-- 鸟种排行（张数降序，后端排序；搜索词过滤；排名 + 占比条 + 观测跨度 + 锐度） -->
          <button
            v-for="(s, i) in stats.filteredStats"
            :key="s.birdName"
            class="flex w-full flex-col gap-1 border-b border-border/60 px-2.5 py-1.5 text-left transition-colors hover:bg-accent/60"
            :class="stats.selectedSpecies === s.birdName ? 'bg-accent' : ''"
            @click="pickSpecies(s.birdName)"
          >
            <span class="flex w-full items-center gap-2">
              <span class="w-4 shrink-0 text-[10px] font-semibold tabular-nums" :class="rankClass(i)">{{ i + 1 }}</span>
              <span class="min-w-0 flex-1 truncate text-sm" :title="s.birdName">{{ s.birdName }}</span>
              <span class="shrink-0 text-xs font-medium tabular-nums">{{ s.photoCount }} 张</span>
            </span>
            <span class="flex w-full items-center gap-2 pl-6">
              <span class="h-1 min-w-8 flex-1 overflow-hidden rounded-full bg-muted">
                <span
                  class="block h-full rounded-full"
                  :class="stats.selectedSpecies === s.birdName ? 'bg-primary' : 'bg-primary/60'"
                  :style="{ width: barPct(s.photoCount) }"
                />
              </span>
              <span
                class="shrink-0 text-[10px] leading-none text-muted-foreground tabular-nums"
                :title="`首见 ${shortDate(s.firstDate)} · 末见 ${shortDate(s.lastDate)}`"
              >
                {{ dateRange(s) }} · 锐 {{ sharpText(s.avgSharpness) }}
              </span>
            </span>
          </button>
        </div>
        <!-- 识别命中率（T 批次 Wave 2）：按鸟种聚合，命中率升序（弱的在前）便于优先复核。
             条形图按阈值着色（≥80% 绿 / ≥50% 琥珀 / 其余红）；样本 < 3 张标注「样本少」；
             折叠省出排行空间 -->
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
          <div v-if="accuracyOpen" class="max-h-44 overflow-y-auto border-t border-border/60 px-2.5 py-1">
            <div
              v-for="s in stats.correctionSorted"
              :key="s.birdName"
              class="flex flex-col gap-1 py-1"
            >
              <span class="flex w-full items-center gap-2">
                <span class="min-w-0 flex-1 truncate text-xs" :title="s.birdName">{{ s.birdName }}</span>
                <span
                  v-if="s.predictedCount < 3"
                  class="shrink-0 rounded-sm bg-muted px-1 py-px text-[9px] leading-none text-muted-foreground"
                  title="样本不足 3 张，命中率仅供参考"
                >样本少</span>
                <span class="shrink-0 text-xs font-medium tabular-nums" :class="accTextClass(s.accuracy)">{{ pct(s.accuracy) }}</span>
              </span>
              <span class="flex w-full items-center gap-2 pl-1">
                <span class="h-1 min-w-8 flex-1 overflow-hidden rounded-full bg-muted">
                  <span class="block h-full rounded-full" :class="accClass(s.accuracy)" :style="{ width: accPct(s.accuracy) }" />
                </span>
                <span class="shrink-0 text-[10px] leading-none text-muted-foreground tabular-nums">
                  预测 {{ s.predictedCount }} · 被改 {{ s.correctedAwayCount }}
                </span>
              </span>
            </div>
            <div v-if="stats.correctionStats.length === 0 && !stats.loading" class="py-2 text-center text-xs text-muted-foreground/70">
              暂无命中率数据
            </div>
          </div>
        </div>
      </aside>

      <!-- 右栏：选中鸟种照片网格（只读浏览，双击跳转） -->
      <div class="flex min-h-0 flex-1 flex-col p-2">
        <!-- 选中鸟种标题：名称 + 张数/覆盖文件夹 + 操作提示 -->
        <div v-if="stats.selectedSpecies" class="mb-2 flex shrink-0 items-baseline gap-2 px-0.5">
          <span class="min-w-0 truncate text-sm font-medium">{{ stats.selectedSpecies }}</span>
          <span class="shrink-0 text-xs text-muted-foreground tabular-nums">
            {{ stats.photos.length }} 张 · {{ selectedFolderCount }} 个文件夹
          </span>
          <span class="ml-auto hidden shrink-0 text-[10px] text-muted-foreground/60 sm:inline">双击照片跳转所在目录</span>
        </div>
        <div class="min-h-0 flex-1 overflow-y-auto">
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
          <div v-else class="grid grid-cols-[repeat(auto-fill,minmax(6.5rem,1fr))] gap-1.5">
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
  </div>
</template>
