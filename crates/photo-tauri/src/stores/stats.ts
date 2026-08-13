// 统计视图（SpeciesIndex T1 批次）状态：鸟种列表 + 选中鸟种的照片网格。
// 数据源 = 后端全局鸟种索引库（exe 同级 data/global.db，跨文件夹聚合）：
//   getSpeciesStats() → 鸟种列表（张数降序）+ 覆盖文件夹数
//   getSpeciesPhotos(birdName) → 该鸟种全部照片定位（folder + relPath）
//   getCorrectionStats() → 各鸟种识别命中率（T 批次 Wave 2，左栏下方区块）
// 视图路由在 preview store（view === 'stats'，Esc/G 退出），本 store 只持有数据。
import { defineStore } from 'pinia'
import { getCorrectionStats, getSpeciesPhotos, getSpeciesStats } from '@/lib/ipc'
import type { CorrectionStat, SpeciesOverview, SpeciesPhoto } from '@/lib/bindings'

export const useStatsStore = defineStore('stats', {
  state: () => ({
    /** 鸟种聚合统计（张数降序）+ 覆盖文件夹数（汇总条） */
    overview: { stats: [], folderCount: 0 } as SpeciesOverview,
    /** 各鸟种识别命中率（命中率升序排序在 getter 侧，弱项在前） */
    correctionStats: [] as CorrectionStat[],
    /** 选中鸟种的照片定位（folder + relPath，拼绝对路径渲染缩略图） */
    photos: [] as SpeciesPhoto[],
    /** 当前选中的鸟种名（null = 未选中，右栏显示空态） */
    selectedSpecies: null as string | null,
    /** 左栏搜索词（按鸟名子串过滤） */
    search: '',
    /** 加载中（首次进入视图时拉取） */
    loading: false,
    /** 是否已拉取过全量统计（防重复加载；识别/修正后由外部调用 refresh 重新拉取） */
    loaded: false,
  }),
  getters: {
    /** 按搜索词过滤后的鸟种列表（保持后端张数降序） */
    filteredStats(s) {
      const q = s.search.trim().toLowerCase()
      if (!q) return s.overview.stats
      return s.overview.stats.filter((x) => x.birdName.toLowerCase().includes(q))
    },
    /** 总照片数 = 各鸟种张数之和（汇总条） */
    totalPhotos(s) {
      return s.overview.stats.reduce((acc, x) => acc + x.photoCount, 0)
    },
    /** 张数条比例基准：全量鸟种最大张数（搜索过滤不改变条长基准，排行条可比） */
    maxPhotoCount(s) {
      return s.overview.stats.reduce((m, x) => Math.max(m, x.photoCount), 0) || 1
    },
    /** 全库平均识别命中率 = 1 - Σ被改 / Σ预测（汇总卡；无预测数据 → null） */
    overallAccuracy(s) {
      const pred = s.correctionStats.reduce((a, x) => a + x.predictedCount, 0)
      if (pred === 0) return null
      const corr = s.correctionStats.reduce((a, x) => a + x.correctedAwayCount, 0)
      return 1 - corr / pred
    },
    /** 选中鸟种的照片绝对路径（folder + '/' + relPath，ptimgUrl 输入形态） */
    photoPaths(s) {
      return s.photos.map((p) => `${p.folder}/${p.relPath}`)
    },
    /** 命中率升序（弱的在前，便于优先复核）；同值按鸟名升序保证确定性。
     *  accuracy 理论恒非 null（predicted ≥ 1），specta 防御性导出 number|null，按 0 兜底 */
    correctionSorted(s): CorrectionStat[] {
      return [...s.correctionStats].sort(
        (a, b) => (a.accuracy ?? 0) - (b.accuracy ?? 0) || a.birdName.localeCompare(b.birdName),
      )
    },
  },
  actions: {
    /** 全量拉取鸟种统计 + 命中率（进入统计视图时调用；加载中防重入） */
    async load() {
      if (this.loading) return
      this.loading = true
      try {
        const [overview, correction] = await Promise.all([
          getSpeciesStats(),
          getCorrectionStats(),
        ])
        this.overview = overview
        this.correctionStats = correction
        this.loaded = true
      } catch (e) {
        // 后端未就绪（mock/全局库降级）：保持空态，不弹错
        console.error('加载鸟种统计失败', e)
      } finally {
        this.loading = false
      }
    },
    /** 选中鸟种 → 拉取该鸟种全部照片（右栏网格数据源） */
    async selectSpecies(name: string) {
      if (this.selectedSpecies === name) return
      this.selectedSpecies = name
      this.photos = await getSpeciesPhotos(name)
    },
    /** 统计视图关闭时复位本地态（下次进入重新拉取，识别/删除后数据不过期） */
    clear() {
      this.selectedSpecies = null
      this.photos = []
      this.search = ''
      this.correctionStats = []
      this.loaded = false
    },
  },
})
