// 全局右键菜单状态：显隐 + 定位 + 菜单项。菜单项为自绘数据（含动作闭包），
// 由触发组件（网格/预览/侧栏）构建，ContextMenu.vue 渲染（
// PopupMenu 的 menu/menu_with_check/submenu/separator 形态）。
import { defineStore } from 'pinia'
import type { CaptureMeta, ColorLabel, Flag } from '@/lib/bindings'
import { deleteCaptures } from '@/lib/ipc'
import { ratingToNumber } from '@/lib/format'
import { useCapturesStore } from './captures'
import { useSelectionStore } from './selection'
import { usePreviewStore } from './preview'
import { useRecognitionStore } from './recognition'
import { useExportStore } from './export'

/** 菜单项（menu 变体映射） */
export type ContextMenuItem =
  | { kind: 'item'; label: string; action: () => void; danger?: boolean }
  | { kind: 'check'; label: string; checked: boolean; action: () => void }
  | { kind: 'submenu'; label: string; items: ContextMenuItem[] }
  | { kind: 'sep' }

export const useContextMenuStore = defineStore('contextMenu', {
  state: () => ({
    /** 是否可见（单一全局实例，store 驱动显隐） */
    open: false,
    /** 弹出位置（屏幕坐标，clientX/clientY） */
    x: 0,
    y: 0,
    items: [] as ContextMenuItem[],
  }),
  actions: {
    /** 在指定屏幕坐标打开菜单 */
    openMenu(items: ContextMenuItem[], x: number, y: number) {
      this.items = items
      this.x = x
      this.y = y
      this.open = true
    },
    /** 关闭并清空菜单 */
    closeMenu() {
      this.open = false
      this.items = []
    },
  },
})

// ── 图片右键菜单构建 ───────────────────────────────

/** 评分子菜单：无评分 + 1–5 星（勾选当前值，Rate0..Rate5） */
function ratingSubmenu(meta: CaptureMeta, paths: string[]): ContextMenuItem[] {
  const captures = useCapturesStore()
  const ratings: [string, number][] = [
    ['无评分', 0],
    ['1 星', 1],
    ['2 星', 2],
    ['3 星', 3],
    ['4 星', 4],
    ['5 星', 5],
  ]
  return ratings.map(([label, n]) => ({
    kind: 'check',
    label,
    checked: ratingToNumber(meta.rating) === n,
    action: () => void captures.applyRating(paths, n),
  }))
}

/** 颜色标签子菜单：无标签 + 红黄绿蓝紫（勾选当前值，LabelNone..LabelPurple） */
function colorLabelSubmenu(meta: CaptureMeta, paths: string[]): ContextMenuItem[] {
  const captures = useCapturesStore()
  const colors: [string, ColorLabel][] = [
    ['无标签', 'None'],
    ['红色', 'Red'],
    ['黄色', 'Yellow'],
    ['绿色', 'Green'],
    ['蓝色', 'Blue'],
    ['紫色', 'Purple'],
  ]
  return colors.map(([label, c]) => ({
    kind: 'check',
    label,
    checked: meta.colorLabel === c,
    action: () => void captures.applyColorLabel(paths, c === 'None' ? null : c),
  }))
}

/** 标记子菜单：无标记 / 留用 / 排除（勾选当前值，FlagNone/FlagPick/FlagReject） */
function flagSubmenu(meta: CaptureMeta, paths: string[]): ContextMenuItem[] {
  const captures = useCapturesStore()
  const flags: [string, Flag | null][] = [
    ['无标记', null],
    ['留用', 'Pick'],
    ['排除', 'Reject'],
  ]
  return flags.map(([label, f]) => ({
    kind: 'check',
    label,
    checked: meta.flag === f,
    action: () => void captures.applyFlag(paths, f),
  }))
}

/** 删除所选（回收站，无确认）：删除后全量重拉 + 清空选中 */
async function deleteSelected(paths: string[]) {
  if (paths.length === 0) return
  const captures = useCapturesStore()
  const selection = useSelectionStore()
  const preview = usePreviewStore()
  // 先判断「当前预览的这张是否在被删集合里」：selection.clear() 之后 selectedIndex
  // 恒为 null，原判断会无条件退出预览（哪怕删的是别的图）
  const idx = selection.selectedIndex
  const wasCurrent =
    preview.isPreview && idx !== null && paths.includes(captures.items[idx]?.primaryPath ?? '')
  try {
    await deleteCaptures(paths)
    await captures.reload()
  } catch (e) {
    console.error('删除失败', e)
  }
  selection.clear()
  if (wasCurrent) preview.closePreview()
}

/**
 * 图片右键菜单（网格 cell 与预览图片共用，差异）：
 * - 网格变体：首项「在预览中打开」，无缩放组；
 * - 预览变体：首项「返回网格」，含放大/缩小/适应窗口/实际像素缩放组；
 * - 多选时识别项文案变为「识别所选照片 (N张)」。
 * `zoom` 闭包由预览组件提供（需要容器尺寸/原图尺寸做锚点计算）。
 */
export function captureMenuItems(opts: {
  /** 右键目标拍摄（决定各子菜单勾选值；null = 无目标时只保留首项 + 识别） */
  meta: CaptureMeta | null
  inPreview: boolean
  /** 多选数量（识别项文案用） */
  selectedCount: number
  /** 动作作用的目标路径集合（选中集） */
  paths: string[]
  /** 视图切换：网格 → 进预览；预览 → 返回网格 */
  onToggleView: () => void
  /** 预览变体专属缩放项（以容器中心为锚点） */
  zoom?: { in: () => void; out: () => void; fit: () => void; actual: () => void }
  /** 复制当前照片到剪贴板（预览变体专属；仅 inPreview 时渲染此项） */
  onCopyImage?: (meta: CaptureMeta) => void
}): ContextMenuItem[] {
  const { meta, inPreview, selectedCount, paths, onToggleView, zoom, onCopyImage } = opts
  const recognition = useRecognitionStore()
  const items: ContextMenuItem[] = [
    { kind: 'item', label: inPreview ? '返回网格' : '在预览中打开', action: onToggleView },
    {
      kind: 'item',
      label:
        selectedCount > 1 ? `识别所选照片 (${selectedCount}张) (b)` : '识别此照片 (b)',
      action: () => void recognition.recognize(paths),
    },
    {
      kind: 'item',
      label: selectedCount > 1 ? `纠正鸟种 (${selectedCount}张)…` : '纠正鸟种…',
      action: () => recognition.openCorrection(paths),
    },
  ]
  if (!meta) return items
  items.push(
    { kind: 'sep' },
    { kind: 'submenu', label: '评分', items: ratingSubmenu(meta, paths) },
    { kind: 'submenu', label: '颜色标签', items: colorLabelSubmenu(meta, paths) },
    { kind: 'submenu', label: '标记', items: flagSubmenu(meta, paths) },
    { kind: 'sep' },
    {
      kind: 'item',
      label: selectedCount > 1 ? `导出所选照片 (${selectedCount}张)…` : '导出此照片…',
      action: () => void useExportStore().openDialog(paths),
    },
  )
  // 预览变体：复制图片项（调用方传入的 onCopyImage；多选时不渲染，仅复制当前图）
  if (inPreview && onCopyImage && selectedCount <= 1) {
    items.push({
      kind: 'item',
      label: '复制图片',
      action: () => onCopyImage(meta),
    })
  }
  if (inPreview && zoom) {
    items.push(
      { kind: 'sep' },
      { kind: 'item', label: '放大', action: zoom.in },
      { kind: 'item', label: '缩小', action: zoom.out },
      { kind: 'item', label: '适应窗口', action: zoom.fit },
      { kind: 'item', label: '实际像素 (100%)', action: zoom.actual },
    )
  }
  items.push(
    { kind: 'sep' },
    { kind: 'item', label: '删除（移至回收站）', danger: true, action: () => void deleteSelected(paths) },
  )
  return items
}
/**
 * 胶片条缩略图右键菜单（轻量变体）：
 * 在预览中打开 / 复制图片 / 复制路径。仅对单张缩略图触发，无评分/标记等选片子菜单。
 */
export function filmstripMenuItems(opts: {
  /** 缩略图对应的拍摄项 */
  meta: CaptureMeta
  /** 打开该照片（选中 + 进入预览） */
  onOpen: () => void
  /** 复制图片（全尺寸 RGBA 到剪贴板） */
  onCopyImage: (meta: CaptureMeta) => void
  /** 复制文件绝对路径（文本） */
  onCopyPath: (meta: CaptureMeta) => void
}): ContextMenuItem[] {
  const { meta, onOpen, onCopyImage, onCopyPath } = opts
  return [
    { kind: 'item', label: '在预览中打开', action: onOpen },
    { kind: 'sep' },
    { kind: 'item', label: '复制图片', action: () => onCopyImage(meta) },
    { kind: 'item', label: '复制路径', action: () => onCopyPath(meta) },
  ]
}
