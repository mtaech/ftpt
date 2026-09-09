// 导入弹窗显隐：文件树 tab「导入」按钮（Sidebar）与挂载点（App.vue）共享，
// 对齐 export store 的自身 store 管理显隐模式。
import { defineStore } from 'pinia'

export const useImportDialogStore = defineStore('importDialog', {
  state: () => ({
    /** 对话框显隐 */
    open: false,
    /** 待预选的导入源（外部入口传入，如 KDE 设备动作 `--import <挂载点>`） */
    pendingSource: null as string | null,
  }),
  actions: {
    /** 外部入口：打开导入对话框并预选源路径（打开后自动扫描） */
    openWithSource(path: string) {
      this.pendingSource = path
      this.open = true
    },
    /** 取走待预选源（导入对话框打开时消费一次） */
    consumePendingSource(): string | null {
      const p = this.pendingSource
      this.pendingSource = null
      return p
    },
  },
})
