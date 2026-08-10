<script setup lang="ts">
// 筛选栏：折叠态摘要行（横向滚动）+ 展开态条件 chips/控件（移植 GPUI
// filter_bar.rs）。仅 grid 视图显示（预览态自行隐藏，对齐 GPUI 行为）。
// 有激活筛选时对应 chips 高亮，清除全部按钮显隐由 hasActiveFilters 驱动。
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { CheckIcon, ChevronDownIcon, ChevronRightIcon, XIcon } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import { useFilterStore } from '@/stores/filter'
import { useCapturesStore } from '@/stores/captures'
import { usePreviewStore } from '@/stores/preview'
import { formatToString } from '@/lib/filter'
import { ratingToNumber } from '@/lib/format'
import type {
  ColorLabel,
  Flag,
  ImageFormat,
  Rating,
  RecognitionFilter,
  SortBy,
  SortDirection,
} from '@/lib/bindings'

const filter = useFilterStore()
const captures = useCapturesStore()
const preview = usePreviewStore()

/** 折叠态（对齐 GPUI filter_bar_expanded；默认折叠） */
const expanded = ref(false)

// ── 格式 chips（固定集合，对齐 GPUI render_format_filter；RAW 用 { Raw: 'RAW' }）──
const FORMAT_CHIPS: { label: string; value: ImageFormat }[] = [
  { label: 'JPEG', value: 'Jpeg' },
  { label: 'PNG', value: 'Png' },
  { label: 'TIFF', value: 'Tiff' },
  { label: 'WebP', value: 'WebP' },
  { label: 'BMP', value: 'Bmp' },
  { label: 'GIF', value: 'Gif' },
  { label: 'HEIF', value: 'Heif' },
  { label: 'RAW', value: { Raw: 'RAW' } },
  { label: 'OTHER', value: 'Other' },
]

// ── 评分 chips：≥N 语义，点击当前值清除 ──
const RATING_CHIPS: { label: string; value: Rating }[] = [
  { label: '1★', value: 'One' },
  { label: '2★', value: 'Two' },
  { label: '3★', value: 'Three' },
  { label: '4★', value: 'Four' },
  { label: '5★', value: 'Five' },
]

// ── 旗标 chips（unflagged = 未标记互斥态）──
const FLAG_CHIPS: { label: string; flag: Flag | null; unflagged: boolean }[] = [
  { label: '任意', flag: null, unflagged: false },
  { label: '入选', flag: 'Pick', unflagged: false },
  { label: '淘汰', flag: 'Reject', unflagged: false },
  { label: '未标记', flag: null, unflagged: true },
]

// ── 识别状态 chips（label 对齐 GPUI recognition_filter_label）──
const RECOGNITION_CHIPS: { label: string; value: RecognitionFilter }[] = [
  { label: '全部', value: 'All' },
  { label: '已识别', value: 'Confirmed' },
  { label: '待复核', value: 'NeedsReview' },
  { label: '未检测到', value: 'Unrecognized' },
  { label: '未识别', value: 'NotRecognized' },
]

// ── 色标 chips（Q9 补齐的筛选入口，GPUI 无此控件）──
const COLOR_CHIPS: { label: string; value: ColorLabel }[] = [
  { label: '红', value: 'Red' },
  { label: '黄', value: 'Yellow' },
  { label: '绿', value: 'Green' },
  { label: '蓝', value: 'Blue' },
  { label: '紫', value: 'Purple' },
]

// ── 排序（label 对齐 GPUI sort_by_label）──
const SORT_OPTIONS: { value: SortBy; label: string }[] = [
  { value: 'FileName', label: '文件名' },
  { value: 'DateTaken', label: '拍摄日期' },
  { value: 'FileSize', label: '文件大小' },
  { value: 'Rating', label: '评分' },
  { value: 'Modified', label: '修改时间' },
]

/** 格式相等（null = 未激活；Raw 需比较载荷，对齐 GPUI format_chip 的 active 判断） */
function sameFormat(a: ImageFormat | null, b: ImageFormat): boolean {
  if (a === null) return false
  if (typeof a === 'object' && typeof b === 'object') return a.Raw === b.Raw
  return a === b
}

/** chips 高亮样式：激活 = primary 描边/文字/浅底（有激活筛选时高亮） */
function chipCls(active: boolean): string {
  return active
    ? 'border-primary bg-primary/10 text-primary'
    : 'border-border text-muted-foreground hover:border-muted-foreground/40 hover:text-foreground'
}

// ── 折叠态摘要 chips（× 可单独清除，对齐 GPUI summary_chip）──
const summaryChips = computed(() => {
  const chips: { key: string; label: string; clear: () => void }[] = []
  const c = filter.criteria
  if (c.birdNames.length > 0) {
    const label =
      c.birdNames.length === 1
        ? `鸟种: ${c.birdNames[0]}`
        : `鸟种: ${c.birdNames[0]} 等${c.birdNames.length}种`
    chips.push({ key: 'birds', label, clear: () => filter.setBirdNames([]) })
  }
  if (c.minRating) {
    chips.push({
      key: 'rating',
      label: `评分≥${ratingToNumber(c.minRating)}星`,
      clear: () => filter.setMinRating(null),
    })
  }
  if (c.flagFilter) {
    chips.push({
      key: 'flag',
      label: c.flagFilter === 'Pick' ? '旗标: 入选' : '旗标: 淘汰',
      clear: () => filter.setFlagFilter(null),
    })
  }
  if (c.unflaggedFilter) {
    chips.push({ key: 'unflagged', label: '旗标: 未标记', clear: () => filter.setUnflagged(false) })
  }
  if (c.recognitionFilter !== 'All') {
    const label =
      RECOGNITION_CHIPS.find((x) => x.value === c.recognitionFilter)?.label ?? c.recognitionFilter
    chips.push({ key: 'recognition', label: `识别: ${label}`, clear: () => filter.setRecognition('All') })
  }
  if (c.formatFilter) {
    chips.push({
      key: 'format',
      label: `格式: ${formatToString(c.formatFilter)}`,
      clear: () => filter.setFormat(null),
    })
  }
  if (c.colorLabel) {
    const label = COLOR_CHIPS.find((x) => x.value === c.colorLabel)?.label ?? c.colorLabel
    chips.push({ key: 'color', label: `色标: ${label}`, clear: () => filter.setColorLabel(null) })
  }
  if (c.dateFrom || c.dateTo) {
    chips.push({
      key: 'date',
      label: `日期: ${c.dateFrom ?? ''}~${c.dateTo ?? ''}`,
      clear: () => filter.setDateRange(null, null),
    })
  }
  return chips
})

// ── 旗标 chip 点击（互斥：未标记 ↔ 具体旗标）──
function onFlagChip(flag: Flag | null, unflagged: boolean) {
  if (unflagged) {
    filter.setUnflagged(!filter.criteria.unflaggedFilter)
    return
  }
  if (flag === null) {
    filter.setFlagFilter(null) // 任意：同时清具体旗标与未标记
  } else if (filter.criteria.flagFilter === flag) {
    filter.setFlagFilter(null)
  } else {
    filter.setFlagFilter(flag)
  }
}

function flagActive(flag: Flag | null, unflagged: boolean): boolean {
  if (unflagged) return filter.criteria.unflaggedFilter
  if (flag === null) return filter.criteria.flagFilter === null && !filter.criteria.unflaggedFilter
  return filter.criteria.flagFilter === flag
}

// ── 日期控件（Q9 补齐）──
function onDateChange(side: 'from' | 'to', e: Event) {
  const v = (e.target as HTMLInputElement).value || null
  if (side === 'from') filter.setDateRange(v, filter.criteria.dateTo)
  else filter.setDateRange(filter.criteria.dateFrom, v)
}

// ── 排序控件 ──
function onSortByChange(e: Event) {
  filter.setSort((e.target as HTMLSelectElement).value as SortBy, filter.sortDirection)
}

function onSortDirChange(e: Event) {
  filter.setSort(filter.sortBy, (e.target as HTMLSelectElement).value as SortDirection)
}

// ── 鸟种多选搜索下拉（自绘；TODO(替换点)：shadcn-vue Combobox 接入后替换）──
const birdOpen = ref(false)
const birdSearch = ref('')
const birdBoxEl = ref<HTMLElement | null>(null)

/** 候选 = 名录并集当前选中（选中项不因名录变化而消失），按搜索词过滤 */
const birdOptions = computed(() => {
  const all = [...new Set([...filter.speciesOptions, ...filter.criteria.birdNames])]
  const q = birdSearch.value.trim().toLowerCase()
  return q ? all.filter((n) => n.toLowerCase().includes(q)) : all
})

function toggleBird(name: string) {
  const cur = filter.criteria.birdNames
  filter.setBirdNames(cur.includes(name) ? cur.filter((n) => n !== name) : [...cur, name])
}

function onDocMouseDown(e: MouseEvent) {
  if (birdOpen.value && birdBoxEl.value && !birdBoxEl.value.contains(e.target as Node)) {
    birdOpen.value = false
    birdSearch.value = ''
  }
}

onMounted(() => document.addEventListener('mousedown', onDocMouseDown))
onUnmounted(() => document.removeEventListener('mousedown', onDocMouseDown))

// 目录打开后刷新鸟种候选（listBirdSpecies 名录全量、拼音排序）
watch(
  () => captures.directory,
  (dir) => {
    if (dir) void filter.loadSpecies()
  },
  { immediate: true },
)
</script>

<template>
  <div v-if="!preview.isPreview" class="shrink-0 border-b border-border bg-card">
    <!-- 折叠行：切换按钮 + 摘要 chips + 排序控件（横向滚动） -->
    <div class="flex items-center gap-1.5 overflow-x-auto px-2 py-1">
      <button
        type="button"
        class="flex shrink-0 cursor-pointer items-center gap-0.5 text-xs select-none"
        :class="filter.hasActiveFilters ? 'font-medium text-primary' : 'text-foreground'"
        @click="expanded = !expanded"
      >
        <ChevronDownIcon v-if="expanded" class="size-3.5" />
        <ChevronRightIcon v-else class="size-3.5" />
        筛选
      </button>

      <!-- 摘要 chips：激活筛选时高亮，× 单独清除 -->
      <span
        v-for="chip in summaryChips"
        :key="chip.key"
        class="flex shrink-0 items-center gap-0.5 rounded-sm border border-primary bg-primary/10 px-2 py-0.5 text-xs text-primary select-none"
      >
        {{ chip.label }}
        <button
          type="button"
          class="cursor-pointer text-primary/70 hover:text-primary"
          :aria-label="`清除${chip.label}`"
          @click="chip.clear()"
        >
          <XIcon class="size-3" />
        </button>
      </span>

      <div class="min-w-3 flex-1" />

      <!-- 排序下拉 + 方向（折叠态常驻，对齐 GPUI 折叠行） -->
      <select
        class="h-7 shrink-0 cursor-pointer rounded-sm border border-border bg-card px-1.5 text-xs text-foreground outline-none"
        :value="filter.sortBy"
        aria-label="排序方式"
        @change="onSortByChange"
      >
        <option v-for="o in SORT_OPTIONS" :key="o.value" :value="o.value">
          排序: {{ o.label }}
        </option>
      </select>
      <select
        class="h-7 shrink-0 cursor-pointer rounded-sm border border-border bg-card px-1.5 text-xs text-foreground outline-none"
        :value="filter.sortDirection"
        aria-label="排序方向"
        @change="onSortDirChange"
      >
        <option value="Ascending">升序</option>
        <option value="Descending">降序</option>
      </select>
    </div>

    <!-- 展开态：条件组（对齐 GPUI expanded_form） -->
    <div
      v-if="expanded"
      class="flex flex-wrap items-start gap-x-4 gap-y-2 border-t border-border px-2 py-1.5"
    >
      <!-- 格式（单选） -->
      <div class="flex items-center gap-1">
        <span class="shrink-0 text-xs text-muted-foreground select-none">格式</span>
        <button
          type="button"
          class="shrink-0 cursor-pointer rounded-sm border px-2 py-0.5 text-xs transition-colors select-none"
          :class="chipCls(filter.criteria.formatFilter === null)"
          @click="filter.setFormat(null)"
        >
          全部
        </button>
        <button
          v-for="c in FORMAT_CHIPS"
          :key="c.label"
          type="button"
          class="shrink-0 cursor-pointer rounded-sm border px-2 py-0.5 text-xs transition-colors select-none"
          :class="chipCls(sameFormat(filter.criteria.formatFilter, c.value))"
          @click="filter.setFormat(sameFormat(filter.criteria.formatFilter, c.value) ? null : c.value)"
        >
          {{ c.label }}
        </button>
      </div>

      <!-- 评分 ≥N（单选，点击当前值清除） -->
      <div class="flex items-center gap-1">
        <span class="shrink-0 text-xs text-muted-foreground select-none">评分≥</span>
        <button
          v-for="c in RATING_CHIPS"
          :key="c.value"
          type="button"
          class="shrink-0 cursor-pointer rounded-sm border px-2 py-0.5 text-xs transition-colors select-none"
          :class="chipCls(filter.criteria.minRating === c.value)"
          @click="filter.setMinRating(filter.criteria.minRating === c.value ? null : c.value)"
        >
          {{ c.label }}
        </button>
      </div>

      <!-- 旗标（互斥单选） -->
      <div class="flex items-center gap-1">
        <span class="shrink-0 text-xs text-muted-foreground select-none">旗标</span>
        <button
          v-for="c in FLAG_CHIPS"
          :key="c.label"
          type="button"
          class="shrink-0 cursor-pointer rounded-sm border px-2 py-0.5 text-xs transition-colors select-none"
          :class="chipCls(flagActive(c.flag, c.unflagged))"
          @click="onFlagChip(c.flag, c.unflagged)"
        >
          {{ c.label }}
        </button>
      </div>

      <!-- 识别状态（单选） -->
      <div class="flex items-center gap-1">
        <span class="shrink-0 text-xs text-muted-foreground select-none">识别</span>
        <button
          v-for="c in RECOGNITION_CHIPS"
          :key="c.value"
          type="button"
          class="shrink-0 cursor-pointer rounded-sm border px-2 py-0.5 text-xs transition-colors select-none"
          :class="chipCls(filter.criteria.recognitionFilter === c.value)"
          @click="filter.setRecognition(c.value)"
        >
          {{ c.label }}
        </button>
      </div>

      <!-- 色标（Q9 补齐，单选） -->
      <div class="flex items-center gap-1">
        <span class="shrink-0 text-xs text-muted-foreground select-none">色标</span>
        <button
          type="button"
          class="shrink-0 cursor-pointer rounded-sm border px-2 py-0.5 text-xs transition-colors select-none"
          :class="chipCls(filter.criteria.colorLabel === null)"
          @click="filter.setColorLabel(null)"
        >
          任意
        </button>
        <button
          v-for="c in COLOR_CHIPS"
          :key="c.value"
          type="button"
          class="shrink-0 cursor-pointer rounded-sm border px-2 py-0.5 text-xs transition-colors select-none"
          :class="chipCls(filter.criteria.colorLabel === c.value)"
          @click="filter.setColorLabel(filter.criteria.colorLabel === c.value ? null : c.value)"
        >
          {{ c.label }}
        </button>
      </div>

      <!-- 日期范围（Q9 补齐，ISO YYYY-MM-DD） -->
      <div class="flex items-center gap-1">
        <label class="flex shrink-0 items-center gap-1 text-xs text-muted-foreground select-none">
          日期 从
          <input
            type="date"
            class="h-7 rounded-sm border border-border bg-card px-1.5 text-xs text-foreground outline-none"
            :value="filter.criteria.dateFrom ?? ''"
            @change="onDateChange('from', $event)"
          />
        </label>
        <label class="flex shrink-0 items-center gap-1 text-xs text-muted-foreground select-none">
          至
          <input
            type="date"
            class="h-7 rounded-sm border border-border bg-card px-1.5 text-xs text-foreground outline-none"
            :value="filter.criteria.dateTo ?? ''"
            @change="onDateChange('to', $event)"
          />
        </label>
      </div>

      <!-- 鸟种多选搜索（自绘下拉；TODO(替换点)：shadcn-vue Combobox multiple 接入后替换） -->
      <div ref="birdBoxEl" class="relative shrink-0">
        <button
          type="button"
          class="flex h-7 min-w-36 max-w-56 cursor-pointer items-center gap-1 rounded-sm border px-2 text-xs transition-colors select-none"
          :class="
            filter.criteria.birdNames.length > 0
              ? 'border-primary bg-primary/10 text-primary'
              : 'border-border bg-card text-foreground'
          "
          @click="birdOpen = !birdOpen"
        >
          <span class="truncate">
            {{
              filter.criteria.birdNames.length === 0
                ? '选择鸟种...'
                : filter.criteria.birdNames.length === 1
                  ? filter.criteria.birdNames[0]
                  : `${filter.criteria.birdNames[0]} 等${filter.criteria.birdNames.length}种`
            }}
          </span>
          <ChevronDownIcon class="ml-auto size-3.5 shrink-0" />
        </button>
        <div
          v-if="birdOpen"
          class="absolute top-full left-0 z-30 mt-1 w-60 rounded-md border border-border bg-popover shadow-lg"
        >
          <input
            v-model="birdSearch"
            type="text"
            placeholder="搜索鸟种..."
            class="m-1.5 h-7 w-[calc(100%-0.75rem)] rounded-sm border border-input bg-card px-1.5 text-xs text-foreground outline-none"
            @keydown.esc="birdOpen = false"
          />
          <ul class="max-h-48 overflow-y-auto p-1">
            <li v-for="name in birdOptions" :key="name">
              <button
                type="button"
                class="flex w-full cursor-pointer items-center gap-1.5 rounded-sm px-1.5 py-1 text-left text-xs hover:bg-muted"
                @click="toggleBird(name)"
              >
                <CheckIcon v-if="filter.criteria.birdNames.includes(name)" class="size-3.5 text-primary" />
                <span v-else class="size-3.5" />
                {{ name }}
              </button>
            </li>
            <li v-if="birdOptions.length === 0" class="px-1.5 py-1 text-xs text-muted-foreground">
              无匹配鸟种
            </li>
          </ul>
        </div>
      </div>

      <!-- 清除全部（有激活筛选时显示） -->
      <div class="ml-auto flex items-center">
        <Button v-if="filter.hasActiveFilters" size="xs" variant="ghost" @click="filter.clearAll()">
          清除全部
        </Button>
      </div>
    </div>
  </div>
</template>
