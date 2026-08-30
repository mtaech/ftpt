// 技术质量评分状态（QualityScore 批次）：批量合成眼锐度 + 直方图剪切 + 检测
// 置信度 → 0..1 技术分。后端 compute_quality_scores spawn_blocking 逐张计算并
// emit quality:progress / quality:done；本 store 持有 scores Map（完整路径 →
// 技术分）+ 进度哨兵。PhotoGrid 徽标与 SortBy::Quality 比较器消费；徽标阈值
// 常量在 PhotoGrid.vue（≥0.75 绿点 / <0.4 红点）。分数只存内存（后端不入库），
// 切换目录后旧目录条目残留无害（键 = 完整路径，重新评分即整体覆盖）。
import { defineStore } from 'pinia'
import type { QualityProgressPayload } from '@/lib/ipc'
import {
  computeQualityScores,
  getQualityScores,
  onQualityDone,
  onQualityProgress,
} from '@/lib/ipc'

export const useQualityStore = defineStore('quality', {
  state: () => ({
    /** 完整路径 → 技术分（0..1；未评分的照片无条目，徽标/排序按缺省处理） */
    scores: {} as Record<string, number>,
    /** 批量评分进行中（后端逐张进度） */
    running: false,
    /** 最近一次进度（done/total/当前文件） */
    progress: null as QualityProgressPayload | null,
    /** 事件是否已接线（防重复 listen） */
    listening: false,
  }),
  getters: {
    /** 指定路径技术分（无 → null：未评分照片不显示徽标、排序 None 排最后） */
    scoreOf: (s) => (path: string): number | null => s.scores[path] ?? null,
  },
  actions: {
    /** 事件接线：store 创建后调用一次（App.vue onMounted） */
    init() {
      if (this.listening) return
      this.listening = true
      void onQualityProgress((p) => {
        this.progress = p
      })
      void onQualityDone((d) => {
        // done 即结束：复位哨兵 + 覆盖本地 Map（后端已整体更新内存）
        this.running = false
        this.progress = null
        for (const [path, score] of d.scores) this.scores[path] = score
      })
      // 启动自愈：后端 AppState 内存 Map 可能留有本次会话评分（事件早于挂载
      // 已错过），主动拉一次补齐（对齐 captures.init 自愈模式）
      void this.reload()
    },
    /** 拉取后端快照（整体覆盖本地 Map；mock/后端未就绪保持空态） */
    async reload() {
      try {
        const entries = await getQualityScores()
        this.scores = Object.fromEntries(entries)
      } catch {
        // mock/后端未就绪：保持空态
      }
    },
    /** 批量计算指定路径的技术分（进行中拒绝并发，对齐 recognition store 守卫） */
    async trigger(paths: string[]) {
      if (this.running || paths.length === 0) return
      this.running = true
      this.progress = { done: 0, total: paths.length, currentPath: paths[0] }
      try {
        await computeQualityScores(paths)
      } catch (e) {
        // 命令调用失败（非事件流）：复位哨兵，等效 GPUI worker 异常兜底
        this.running = false
        this.progress = null
        console.error('技术质量评分启动失败', e)
      }
    },
    /** 清空本地分数（切换目录后旧目录条目失效；重新评分即覆盖） */
    clear() {
      this.scores = {}
      this.running = false
      this.progress = null
    },
  },
})
