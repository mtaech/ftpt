// 扫描结果与标记状态：事件接线（进度/完成/EXIF 回填/缩略图就绪）+ 乐观更新。
import { defineStore } from 'pinia'
import type { CaptureMeta, ColorLabel, Flag } from '@/lib/bindings'
import {
  getAppConfig,
  getCaptures,
  onCaptureEnriched,
  onScanDone,
  onScanProgress,
  onThumbReady,
  pickDirectory,
  scanDirectory,
  setColorLabel,
  setFlag,
  setRating,
  type ScanProgressPayload,
} from '@/lib/ipc'
import { numberToRating } from '@/lib/format'

export const useCapturesStore = defineStore('captures', {
  state: () => ({
    /** 当前扫描结果（全量下推，前端权威副本） */
    items: [] as CaptureMeta[],
    /** 当前目录（扫描来源） */
    directory: null as string | null,
    /** 扫描进行中（含后台 EXIF/缩略图阶段） */
    scanning: false,
    /** 最近一条扫描进度 */
    progress: null as ScanProgressPayload | null,
    /** 缩略图版本号：thumb:ready 后递增加 ?v= 强制 img 刷新 */
    thumbVersions: {} as Record<string, number>,
    /** 事件是否已接线（防重复 listen） */
    listening: false,
  }),
  getters: {
    count: (s) => s.items.length,
  },
  actions: {
    /** 事件接线：store 创建后调用一次 */
    init() {
      if (this.listening) return
      this.listening = true
      void onScanProgress((p) => {
        this.progress = p
      })
      void onScanDone((p) => {
        this.scanning = false
        this.progress = null
        // 同步当前目录：启动自动恢复扫描走后端 setup（不经 openPath），前端借此补上目录状态
        if (p.directory) this.directory = p.directory
        void this.reload()
      })
      // EXIF 回填完成：重拉全量（Phase 1 排序键简单，直接重排）
      void onCaptureEnriched(() => {
        void this.reload()
      })
      void onThumbReady((p) => {
        this.thumbVersions[p.path] = Date.now()
      })
      // 启动自愈：自动恢复扫描（setup 内 spawn）在页面挂载前就完成（缓存命中仅 ~200ms），
      // scan:done 事件已错过——主动拉一次后端状态补齐 directory/items
      void (async () => {
        try {
          const cfg = await getAppConfig()
          if (cfg.lastDirectory && !this.directory) this.directory = cfg.lastDirectory
          await this.reload()
        } catch {
          // mock/后端未就绪：保持空态
        }
      })()
    },

    /** 打开目录：对话框 → 扫描（成功后事件驱动后续刷新） */
    async openDirectory() {
      const dir = await pickDirectory()
      if (!dir) return
      await this.openPath(dir)
    },

    /** 打开指定路径目录（侧栏收藏/最近单击复用）：扫描 + 更新哨兵 */
    async openPath(path: string) {
      // 已是当前目录则跳过重扫（收藏/最近卡片单击当前目录时无副作用）
      if (!path || path === this.directory) return
      this.directory = path
      this.scanning = true
      this.progress = { stage: 'scan', done: 0, total: 0 }
      this.thumbVersions = {}
      try {
        await scanDirectory(path)
        await this.reload()
      } catch (e) {
        // 扫描失败复位哨兵，等效 GPUI worker panic 兜底
        this.scanning = false
        this.progress = null
        console.error('scan_directory 失败', e)
      }
    },

    /** 全量重拉（扫描完成/EXIF 回填/失败回滚共用） */
    async reload() {
      this.items = await getCaptures()
    },

    /** 重扫当前目录（F5）：无目录时 no-op；哨兵复位语义同 openDirectory */
    async rescan() {
      if (!this.directory) return
      this.scanning = true
      this.progress = { stage: 'scan', done: 0, total: 0 }
      try {
        await scanDirectory(this.directory)
        await this.reload()
      } catch (e) {
        // 扫描失败复位哨兵，等效 GPUI worker panic 兜底
        this.scanning = false
        this.progress = null
        console.error('重扫失败', e)
      }
    },

    /** 选中路径集合的乐观更新骨架：先改本地，失败重拉回滚 */
    async mutateOptimistic(
      paths: string[],
      apply: (c: CaptureMeta) => void,
      remote: () => Promise<void>,
    ) {
      if (paths.length === 0) return
      for (const c of this.items) if (paths.includes(c.primaryPath)) apply(c)
      try {
        await remote()
      } catch (e) {
        console.error('标记写入失败，重拉回滚', e)
        await this.reload()
      }
    },

    async applyRating(paths: string[], rating: number) {
      const r = numberToRating(rating)
      await this.mutateOptimistic(paths, (c) => (c.rating = r), () => setRating(paths, rating))
    },

    async applyFlag(paths: string[], flag: Flag | null) {
      await this.mutateOptimistic(paths, (c) => (c.flag = flag), () => setFlag(paths, flag))
    },

    async applyColorLabel(paths: string[], label: ColorLabel | null) {
      await this.mutateOptimistic(
        paths,
        (c) => (c.colorLabel = label ?? 'None'),
        () => setColorLabel(paths, label),
      )
    },
  },
})
