// 筛选/排序状态：全量 captures 在前端，筛选零 IPC（迁移计划 Q5）。
// 消费方用 filteredIndices（captures.items 下标，即 Rust 的 display_order）
// 渲染网格并以此下标驱动 selection；无筛选时批量操作应禁用（hasActiveFilters）。
import { defineStore } from 'pinia'
import {
  applyFilterAndSort,
  defaultFilterCriteria,
  hasActiveFilters as hasActiveCriteria,
} from '@/lib/filter'
import type {
  CaptureMeta,
  ColorLabel,
  Flag,
  ImageFormat,
  Rating,
  RecognitionFilter,
  SortBy,
  SortDirection,
} from '@/lib/bindings'
import { listBirdSpecies } from '@/lib/ipc'
import { useCapturesStore } from './captures'

export const useFilterStore = defineStore('filter', {
  state: () => ({
    /** 筛选条件（defaultFilterCriteria 对齐 Rust FilterCriteria::default） */
    criteria: defaultFilterCriteria(),
    /** 排序方式 */
    sortBy: 'FileName' as SortBy,
    /** 排序方向 */
    sortDirection: 'Ascending' as SortDirection,
    /** 鸟种多选下拉候选（listBirdSpecies，拼音排序；目录打开后刷新） */
    speciesOptions: [] as string[],
  }),
  getters: {
    /** 过滤+排序后的下标数组（captures.items 下标，对齐 Rust display_order） */
    filteredIndices(): number[] {
      const captures = useCapturesStore()
      return applyFilterAndSort(captures.items, {
        criteria: this.criteria,
        sortBy: this.sortBy,
        sortDirection: this.sortDirection,
      })
    },
    /** 过滤+排序后的拍摄列表（下标映射） */
    filtered(): CaptureMeta[] {
      const captures = useCapturesStore()
      return this.filteredIndices.map((i) => captures.items[i])
    },
    /** 是否有任一筛选条件生效（批量操作禁用依据） */
    hasActiveFilters(): boolean {
      return hasActiveCriteria(this.criteria)
    },
  },
  actions: {
    /** 格式单选（null = 全部） */
    setFormat(formatFilter: ImageFormat | null) {
      this.criteria.formatFilter = formatFilter
    },
    /** 鸟种多选（全量替换选中集） */
    setBirdNames(birdNames: string[]) {
      this.criteria.birdNames = [...birdNames]
    },
    /** 日期范围（ISO YYYY-MM-DD，null = 不限制） */
    setDateRange(dateFrom: string | null, dateTo: string | null) {
      this.criteria.dateFrom = dateFrom
      this.criteria.dateTo = dateTo
    },
    /** 评分下限（null = 任意） */
    setMinRating(minRating: Rating | null) {
      this.criteria.minRating = minRating
    },
    /** 色标精确匹配（null = 任意） */
    setColorLabel(colorLabel: ColorLabel | null) {
      this.criteria.colorLabel = colorLabel
    },
    /** 旗标精确匹配；设置具体旗标时取消「未标记」互斥态（对齐 GPUI 芯片语义） */
    setFlagFilter(flagFilter: Flag | null) {
      this.criteria.flagFilter = flagFilter
      this.criteria.unflaggedFilter = false
    },
    /** 未标记筛选；开启时清空具体旗标（互斥，对齐 GPUI 芯片语义） */
    setUnflagged(unflagged: boolean) {
      this.criteria.unflaggedFilter = unflagged
      if (unflagged) this.criteria.flagFilter = null
    },
    /** 识别状态筛选 */
    setRecognition(recognitionFilter: RecognitionFilter) {
      this.criteria.recognitionFilter = recognitionFilter
    },
    /** 排序方式/方向（对齐 GPUI：改排序不清筛选） */
    setSort(sortBy: SortBy, sortDirection: SortDirection) {
      this.sortBy = sortBy
      this.sortDirection = sortDirection
    },
    /** 清除全部筛选条件（保留排序，对齐 GPUI clear_filters） */
    clearAll() {
      this.criteria = defaultFilterCriteria()
    },
    /** 拉取鸟种候选（listBirdSpecies 名录全量、拼音排序）；目录打开后调用 */
    async loadSpecies() {
      try {
        this.speciesOptions = await listBirdSpecies()
      } catch {
        // 后端不可用时保持空列表；已选中的鸟种仍由筛选栏并集展示
        this.speciesOptions = []
      }
    },
  },
})
