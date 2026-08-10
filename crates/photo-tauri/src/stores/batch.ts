// 批量文件操作状态（ADR 0006：筛选驱动 + 画面粒度）
//
// 交互模型（移植 GPUI state/batch_ops.rs）：
// - 操作对象 = 当前筛选结果（filter.filteredIndices），纯筛选驱动；无筛选时禁用
// - 两阶段：干跑预览（batch_op_preview，只算不动文件）→ 确认执行（batch_op_execute）
// - 移动/复制需目标目录（Delete 忽略）；「同步同名文件」开关 + 格式多选按 stem 并入兄弟文件
// - 执行完成后 Move/Delete 触发全量重扫刷新网格（Copy 不动源列表）
import { defineStore } from 'pinia'
import type {
  BatchOpOptions,
  BatchOpPreview,
  BatchOpResult,
  BatchOpType,
  CaptureMeta,
  ImageFormat,
} from '@/lib/bindings'
import {
  batchOpExecute,
  batchOpPreview,
  onBatchProgress,
  pickDirectory,
  type BatchProgressPayload,
} from '@/lib/ipc'
import { useCapturesStore } from './captures'
import { useFilterStore } from './filter'

// ── 操作类型文案（对齐 domain.rs BatchOpType：action_label / 语义描述）──

/** 动作标签（对齐 GPUI action_label） */
export function opLabel(op: BatchOpType): string {
  switch (op) {
    case 'Move':
      return '移动'
    case 'Copy':
      return '复制'
    case 'Delete':
      return '删除'
  }
}

/** 操作语义描述（对齐 domain.rs 枚举注释：移动到目标目录 / 复制到目标目录 / 删除（回收站）） */
export function opDescription(op: BatchOpType): string {
  switch (op) {
    case 'Move':
      return '移动到目标目录'
    case 'Copy':
      return '复制到目标目录'
    case 'Delete':
      return '删除（回收站）'
  }
}

// ── 格式工具 ──────────────────────────────────────────

/** ImageFormat 恒等比较（Raw 变体比较扩展名字符串） */
export function formatsEqual(a: ImageFormat, b: ImageFormat): boolean {
  if (typeof a === 'string' || typeof b === 'string') return a === b
  return a.Raw.toLowerCase() === b.Raw.toLowerCase()
}

/**
 * primaryFormat（Display 输出串，如 JPEG/NEF/png）→ ImageFormat。
 * 未知扩展名映射为 { Raw: 大写扩展名 }（对齐 formatToString 的 default 分支）。
 */
function formatFromPrimary(primary: string): ImageFormat | null {
  const p = primary.toLowerCase()
  const known: Record<string, ImageFormat> = {
    jpeg: 'Jpeg',
    jpg: 'Jpeg',
    png: 'Png',
    tiff: 'Tiff',
    tif: 'Tiff',
    heif: 'Heif',
    heic: 'Heif',
    webp: 'WebP',
    bmp: 'Bmp',
    gif: 'Gif',
    other: 'Other',
  }
  if (p in known) return known[p]
  if (p === 'raw') return { Raw: 'RAW' }
  return { Raw: p.toUpperCase() }
}

/** 目录实际出现的格式（去重排序；未知 primaryFormat 忽略） */
export function formatsInDirectory(items: CaptureMeta[]): ImageFormat[] {
  const seen = new Set<string>()
  const out: ImageFormat[] = []
  for (const it of items) {
    const fmt = formatFromPrimary(it.primaryFormat)
    if (!fmt) continue
    const key = typeof fmt === 'string' ? fmt : `raw:${fmt.Raw.toUpperCase()}`
    if (seen.has(key)) continue
    seen.add(key)
    out.push(fmt)
  }
  return out
}

/** 路径归一（Windows/Unix 分隔符统一 + 去尾部斜杠），用于源/目标目录相等比较 */
function normDir(p: string): string {
  return p.replace(/\\/g, '/').replace(/\/+$/, '')
}

export const useBatchStore = defineStore('batch', {
  state: () => ({
    /** 当前操作类型（三选：移动/复制/删除） */
    op: 'Move' as BatchOpType,
    /** 目标目录（Move/Copy 必填；Delete 忽略） */
    targetDir: null as string | null,
    /** 同步同名文件开关（默认关） */
    syncSiblings: false,
    /** 同步格式多选（默认全选：开启同步时按目录实际格式初始化） */
    formats: [] as ImageFormat[],
    /** 干跑预览（batch_op_preview 结果；切换目录/格式/操作类型即失效） */
    preview: null as BatchOpPreview | null,
    /** 执行中（进度弹窗显示依据） */
    running: false,
    /** 执行进度（batch:progress 事件） */
    progress: null as BatchProgressPayload | null,
    /** 最近一次执行结果（完成后保留，供面板结果明细展示） */
    result: null as BatchOpResult | null,
    /** 失败详情列表（可滚动；= result.failures 格式化为「路径：错误」） */
    errors: [] as string[],
    /** 面板 toast 瞬态提示（组件负责 4s 自动消失） */
    toast: null as string | null,
    /** 事件是否已接线（防重复 listen） */
    listening: false,
  }),
  getters: {
    /** 发送给后端的操作选项（BatchOpOptions 契约形态） */
    options(state): BatchOpOptions {
      return {
        targetDir: state.targetDir,
        syncSiblings: state.syncSiblings,
        formats: state.formats,
      }
    },
  },
  actions: {
    /** 事件接线：面板挂载后调用一次（对齐 captures.init 模式） */
    init() {
      if (this.listening) return
      this.listening = true
      void onBatchProgress((p) => {
        this.progress = p
      })
    },

    /** 切换操作类型：预览失效 */
    setOp(op: BatchOpType) {
      if (this.op === op) return
      this.op = op
      this.preview = null
    },

    /** 写入目标目录（一步式选择成功后调用）：预览失效 */
    setTarget(dir: string | null) {
      if (this.targetDir === dir) return
      this.targetDir = dir
      this.preview = null
    },

    /** 一步式选择目标目录：系统目录对话框 → 拒绝「目标 = 源目录」（toast） */
    async chooseTarget() {
      const dir = await pickDirectory()
      if (!dir) return
      const captures = useCapturesStore()
      if (captures.directory && normDir(dir) === normDir(captures.directory)) {
        this.toast = '目标目录不能与当前目录相同'
        return
      }
      this.setTarget(dir)
    },

    /** 切换「同步同名文件」开关：开启且格式为空时按目录实际格式默认全选；预览失效 */
    setSyncSiblings(enabled: boolean) {
      if (this.syncSiblings === enabled) return
      this.syncSiblings = enabled
      if (enabled && this.formats.length === 0) {
        this.formats = formatsInDirectory(useCapturesStore().items)
      }
      this.preview = null
    },

    /** 勾选/取消某个同步格式：预览失效 */
    toggleFormat(fmt: ImageFormat) {
      const idx = this.formats.findIndex((f) => formatsEqual(f, fmt))
      if (idx >= 0) this.formats.splice(idx, 1)
      else this.formats.push(fmt)
      this.preview = null
    },

    /** 全量替换同步格式（默认全选用）：预览失效 */
    setFormats(fmts: ImageFormat[]) {
      this.formats = [...fmts]
      this.preview = null
    },

    /** 干跑预览：只算不动文件（Delete 不需要 targetDir） */
    async runPreview() {
      if (this.running) return
      const filter = useFilterStore()
      if (!filter.hasActiveFilters) {
        this.toast = '未设置筛选条件，请先在筛选栏设置条件（防全文件误操作）'
        return
      }
      if (this.op !== 'Delete' && !this.targetDir) {
        this.toast = '请先选择目标目录'
        return
      }
      try {
        this.preview = await batchOpPreview(this.op, this.options)
      } catch (e) {
        this.toast = `干跑预览失败：${String(e)}`
      }
    },

    /**
     * 确认执行：进度事件驱动进度弹窗；完成后 Move/Delete 全量重扫刷新网格
     * （Copy 不影响源列表，不重扫）；结果/失败明细留在面板展示。
     */
    async confirmExecute() {
      if (this.running || !this.preview) return
      this.running = true
      this.result = null
      this.errors = []
      this.progress = { done: 0, total: this.preview.count, currentPath: '' }
      try {
        const result = await batchOpExecute(this.op, this.options)
        this.result = result
        this.errors = result.failures.map((f) => `${f.path}：${f.error}`)
        this.toast = `${opLabel(this.op)}完成：成功 ${result.success} / 失败 ${result.failed}`
        // 移动/删除改变目录内容 → 全量重扫刷新网格（复制不影响源列表）
        if (this.op !== 'Copy') {
          const captures = useCapturesStore()
          await captures.rescan()
        }
      } catch (e) {
        this.errors = [...this.errors, `执行失败：${String(e)}`]
        this.toast = `批量${opLabel(this.op)}执行失败：${String(e)}`
      } finally {
        this.running = false
      }
    },

    /** 复位：清空预览/进度/结果（不重置操作设置） */
    reset() {
      this.preview = null
      this.running = false
      this.progress = null
      this.result = null
      this.errors = []
    },
  },
})
