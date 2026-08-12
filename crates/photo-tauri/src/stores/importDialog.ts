// 导入弹窗显隐：文件树 tab「导入」按钮（Sidebar）与挂载点（App.vue）共享，
// 对齐 export store 的自身 store 管理显隐模式。
import { defineStore } from 'pinia'

export const useImportDialogStore = defineStore('importDialog', {
  state: () => ({
    /** 对话框显隐 */
    open: false,
  }),
})
