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
import { computeBurstGroups, type BurstEntry } from '@/lib/burst'
import { groupByTime, groupSingles, groupStacks, type StackGroup } from '@/lib/stacks'
import { useCapturesStore } from './captures'
import { useConfigStore } from './config'

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
    /** 堆叠激活覆盖：stem → 激活成员下标（用户手动切换；空对象 = 全部默认主格式） */
    stackActive: {} as Record<string, number>,
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
    /**
     * 连拍组映射（captures.items 下标 → 组信息；仅 size≥2 的组）。
     * 按显示序分组（computeBurstGroups 输入即 filteredIndices 对应项），
     * 网格徽标与对比模式「取组内前 4 张」共用。
     */
    burstGroups(): Map<number, BurstEntry> {
      const captures = useCapturesStore()
      const order = this.filteredIndices
      const groups = computeBurstGroups(order.map((i) => captures.items[i]))
      const byIndex = new Map<number, BurstEntry>()
      groups.forEach((e, pos) => byIndex.set(order[pos], e))
      return byIndex
    },
    /**
     * 显示堆叠组：filteredIndices 按配置的堆叠模式分组（网格渲染依据）。
     * None = 每成员独立；ByFileName = 同 stem 合并；ByTime = 同组照片（连拍）合并。
     * 组位置 = 组内成员在显示序中的最小位置；active 默认主格式，stackActive 覆盖
     * （指向的成员不在组内时——如筛选变化——自动回退主格式，残留覆盖无害）。
     */
    stackGroups(): StackGroup[] {
      const captures = useCapturesStore()
      const config = useConfigStore()
      const indices = this.filteredIndices
      const mode = config.stackMode
      const groups =
        mode === 'None'
          ? groupSingles(indices)
          : mode === 'ByFileName'
            ? groupStacks(indices, captures.items)
            : groupByTime(indices, captures.items)
      for (const g of groups) {
        const override = this.stackActive[g.key]
        if (override !== undefined && g.members.includes(override)) g.active = override
      }
      return groups
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
    /** ISO 区间（闭区间；null = 该侧不限制） */
    setIsoRange(isoMin: number | null, isoMax: number | null) {
      this.criteria.isoMin = isoMin
      this.criteria.isoMax = isoMax
    },
    /** 焦距区间（mm，闭区间；null = 该侧不限制） */
    setFocalRange(focalMin: number | null, focalMax: number | null) {
      this.criteria.focalMin = focalMin
      this.criteria.focalMax = focalMax
    },
    /** 镜头多选（全量替换选中集） */
    setLensFilter(lensFilter: string[]) {
      this.criteria.lensFilter = [...lensFilter]
    },
    /** 关键词筛选（全量替换选中集） */
    setKeywordFilter(keywordFilter: string[]) {
      this.criteria.keywordFilter = [...keywordFilter]
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
    /** 手动指定堆叠激活成员（分组键不存在时写入无害——getter 落不到组上） */
    setStackActive(key: string, index: number) {
      this.stackActive = { ...this.stackActive, [key]: index }
    },
    /**
     * 堆叠内循环切换激活成员（±1 循环）。返回新激活成员下标（组不存在/单成员
     * 组返回 null，调用方据此跳过选中联动）。成员下标以 items 下标为键。
     */
    cycleStack(key: string, direction: 1 | -1): number | null {
      const g = this.stackGroups.find((x) => x.key === key)
      if (!g || g.members.length < 2) return null
      const pos = g.members.indexOf(g.active)
      const next = g.members[(pos + direction + g.members.length) % g.members.length]
      this.setStackActive(key, next)
      return next
    },
    /** 成员下标 → 堆叠组位置（网格行定位用）；不在任何组返回 -1 */
    stackPositionOf(i: number): number {
      const groups = this.stackGroups
      for (let p = 0; p < groups.length; p++) {
        if (groups[p].members.includes(i)) return p
      }
      return -1
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
