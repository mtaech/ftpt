// 近重复检测（pHash）：事件接线（进度/完成）+ 分组结果状态 + 触发检测。
// 结果只存内存（后端 find_duplicates 已约定禁止写库）；「保留第一张标其余」
// 走 captures store 的现有旗标 mutation（applyFlag），此处不重复实现。
import { defineStore } from 'pinia'
import {
  findDuplicates,
  onDuplicatesDone,
  onDuplicatesProgress,
  type DuplicatesDonePayload,
  type DuplicatesProgressPayload,
} from '@/lib/ipc'

export const useDuplicatesStore = defineStore('duplicates', {
  state: () => ({
    /** 面板显隐（Sidebar 底部按钮打开；Esc/×/遮罩关闭） */
    open: false,
    /** 检测进行中（done 事件到来前为 true；防重复触发由后端守卫 + 本地 running 双保险） */
    running: false,
    /** 最近一条哈希进度 */
    progress: null as DuplicatesProgressPayload | null,
    /** 近重复分组（每组 = 组内照片完整路径列表，组内首张为保留锚点） */
    groups: [] as string[][],
    /** 整体失败原因（后端 done 事件 error 字段；非 null 时 groups 为空） */
    error: null as string | null,
    /** 是否已执行过检测（空态文案区分「未检测」与「未发现重复」） */
    hasRun: false,
    /** 事件是否已接线（防重复 listen） */
    listening: false,
  }),
  actions: {
    /** 事件接线：面板组件挂载时调用一次（DuplicatesPanel setup） */
    init() {
      if (this.listening) return
      this.listening = true
      void onDuplicatesProgress((p) => {
        this.running = true
        this.progress = p
      })
      void onDuplicatesDone((p: DuplicatesDonePayload) => {
        this.running = false
        this.progress = null
        this.hasRun = true
        if (p.error) {
          this.error = p.error
          this.groups = []
        } else {
          this.error = null
          this.groups = p.groups
        }
      })
    },

    openPanel() {
      this.open = true
    },

    closePanel() {
      this.open = false
    },

    /** 切换目录时丢弃旧目录结果，避免对已移动/删除文件执行操作。 */
    clear() {
      this.running = false
      this.progress = null
      this.groups = []
      this.error = null
      this.hasRun = false
    },

    /**
     * 触发近重复检测（threshold 缺省用后端默认 10）。
     * 命令本身是 fire-and-forget（返回即完成，进度/结果经事件推送）：
     * 仅命令级失败（未开目录/无照片/并发守卫）在此 catch 展示。
     */
    async run(threshold?: number) {
      if (this.running) return
      this.error = null
      this.groups = []
      this.hasRun = false
      this.progress = null
      try {
        await findDuplicates(threshold ?? null)
      } catch (e) {
        this.running = false
        this.error = String(e)
      }
    },
  },
})
