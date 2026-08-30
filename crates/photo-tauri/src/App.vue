<script setup lang="ts">
// 根布局（对齐 GPUI layout.rs 三栏结构）：
//   顶栏（全宽）→ 主区 h_flex：左 rail | 左栏(LeftPanel：目录/批量双 tab) | 内容区 | 右栏(InfoPanel) | 右 rail
//   → 底部 StatusBar。
// 左/右栏可独立隐藏（Ctrl+[ / Ctrl+] 切换，右栏初始值跟随后端配置），
// 拖宽把手在各面板内部（宽度 localStorage 持久化，范围 200–480）。全局快捷键见 keymap.ts。
import { nextTick, onMounted, onUnmounted, ref } from 'vue'
import {
  FolderOpenIcon,
  GalleryVerticalEndIcon,
  ImageIcon,
  PanelLeftIcon,
  PanelLeftOpenIcon,
  RefreshCwIcon,
  ScanSearchIcon,
  SettingsIcon,
  CrownIcon,
} from '@lucide/vue'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import PhotoGrid from '@/components/PhotoGrid.vue'
import TitleBar from '@/components/TitleBar.vue'
import WindowResizeEdges from '@/components/WindowResizeEdges.vue'
import PhotoPreview from '@/components/PhotoPreview.vue'
import CompareView from '@/components/CompareView.vue'
import SlideshowView from '@/components/SlideshowView.vue'
import StatsView from '@/components/StatsView.vue'
import FilterBar from '@/components/FilterBar.vue'
import LeftPanel from '@/components/LeftPanel.vue'
import InfoPanel from '@/components/InfoPanel.vue'
import RightRail from '@/components/RightRail.vue'
import StatusBar from '@/components/StatusBar.vue'
import ContextMenu from '@/components/ContextMenu.vue'
import SettingsModal from '@/components/SettingsModal.vue'
import DuplicatesPanel from '@/components/DuplicatesPanel.vue'
import ImportDialog from '@/components/ImportDialog.vue'
import ExportDialog from '@/components/ExportDialog.vue'
import MapView from '@/components/MapView.vue'
import SpeciesCorrectDialog from '@/components/SpeciesCorrectDialog.vue'
import { useCapturesStore } from '@/stores/captures'
import { useSelectionStore } from '@/stores/selection'
import { usePreviewStore } from '@/stores/preview'
import { useCompareStore } from '@/stores/compare'
import { useFilterStore } from '@/stores/filter'
import { useRecognitionStore } from '@/stores/recognition'
import { useConfigStore } from '@/stores/config'
import { useStatsStore } from '@/stores/stats'
import { useBatchStore } from '@/stores/batch'
import { useImportDialogStore } from '@/stores/importDialog'
import { useMapViewStore } from '@/stores/mapView'
import { useQualityStore } from '@/stores/quality'
import { useDuplicatesStore } from '@/stores/duplicates'
import { installKeymap, type KeymapHandlers } from '@/keymap'
import { zoomHost } from '@/lib/zoomHost'
import { nonBestPaths } from '@/lib/bestFrame'
import type { CaptureMeta } from '@/lib/bindings'
import { deleteCaptures, undoBatchOperation } from '@/lib/ipc'

const captures = useCapturesStore()
const selection = useSelectionStore()
const preview = usePreviewStore()
const compare = useCompareStore()
const filter = useFilterStore()
const recognition = useRecognitionStore()
const configStore = useConfigStore()
const stats = useStatsStore()
const batch = useBatchStore()
const mapView = useMapViewStore()
const quality = useQualityStore()
const duplicates = useDuplicatesStore()

/** 左栏可见（对齐 GPUI sidebar_visible，运行时状态，默认可见） */
const sidebarVisible = ref(true)
/** 左栏当前 tab（'dir' 目录 / 'batch' 批量；顶栏「批量操作」按钮切换） */
const leftTab = ref('dir')
/** 右栏可见（对齐 GPUI config.right_panel_visible，初始值跟随后端配置，默认可见） */
const rightPanelVisible = ref(true)
/** 设置弹窗开关（顶栏齿轮按钮打开、Esc 分支关闭，v-model 传给 SettingsModal） */
const settingsOpen = ref(false)
/** 导入弹窗显隐（store 驱动：文件树 tab「导入」按钮打开，×/Esc/遮罩关闭） */
const importDialog = useImportDialogStore()
/** K 键确认弹窗快照：打开时记录组数/将标记张数/目标路径，确认后按快照应用（避免等待期状态漂移） */
const keepBest = ref<{ groupCount: number; markCount: number; paths: string[] } | null>(null)

/**
 * 标记键目标路径：对比模式（视图态）作用于聚焦格（单张），幻灯片态作用于
 * 当前显示张，其余作用于选中集。评分/色标/旗标/Delete 共用，保证对比/幻灯片
 * 模式不误伤未显示的项。以 preview.isCompare/isSlideshow 判定（而非 store 的
 * active 字段），退出后立即恢复选中集语义。
 */
function markPaths(): string[] {
  const p = compare.focusedPath
  if (preview.isCompare && p) return [p]
  // 幻灯片态：当前张 = 筛选序 slideshowIndex 对应项（越界安全）
  if (preview.isSlideshow) {
    const i = filter.filteredIndices[preview.slideshowIndex]
    const item = i === undefined ? null : captures.items[i]
    return item ? [item.primaryPath] : []
  }
  return selection.selectedPaths
}

/**
 * 进入对比（C 键）：多选 2–4 张 → 对比选中集；
 * 不足 2 张时当前项属连拍组 → 取组内前 4 张（按显示序）。
 */
function enterCompare() {
  const sel = selection.selectedIndices
  if (sel.length >= 2 && sel.length <= 4) {
    compare.open(sel)
    preview.openCompare()
    return
  }
  const idx = selection.selectedIndex
  if (idx === null || sel.length > 4) return
  const entry = filter.burstGroups.get(idx)
  if (!entry) return
  const members = [...filter.burstGroups.entries()]
    .filter(([, e]) => e.groupId === entry.groupId)
    .map(([i]) => i)
    .slice(0, 4)
  if (members.length < 2) return
  compare.open(members)
  preview.openCompare()
}

/**
 * K 键目标连拍组（组元数据数组，captures.items 下标 → CaptureMeta）：
 * 有选中 → 仅当前选中照片所在组；无选中 → 当前筛选结果内全部 size≥2 组
 * （filter.burstGroups 按显示序登记，天然只含当前筛选内的 size≥2 组）。
 */
function collectBurstGroups(): CaptureMeta[][] {
  const byGroup = new Map<string, number[]>()
  for (const [i, e] of filter.burstGroups) {
    const arr = byGroup.get(e.groupId)
    if (arr) arr.push(i)
    else byGroup.set(e.groupId, [i])
  }
  const idx = selection.selectedIndex
  if (idx !== null) {
    const entry = filter.burstGroups.get(idx)
    if (!entry) return []
    const members = byGroup.get(entry.groupId) ?? []
    return [members.map((i) => captures.items[i])]
  }
  return [...byGroup.values()].map((members) => members.map((i) => captures.items[i]))
}

/** K 键：连拍组自动选优。确认后把各组非最优帧批量标 Flag::Rejected（复用现有旗标 mutation）。 */
function keepBestFrame() {
  const groups = collectBurstGroups()
  const paths = groups.flatMap(nonBestPaths)
  if (paths.length === 0) return
  keepBest.value = { groupCount: groups.length, markCount: paths.length, paths }
}

/** 确认弹窗「标记 Reject」：按打开时快照应用（乐观更新 + 失败重拉回滚由 captures.applyFlag 承担） */
function confirmKeepBest() {
  const pending = keepBest.value
  if (!pending) return
  keepBest.value = null
  void captures.applyFlag(pending.paths, 'Reject')
}

/** 回到网格：对比模式先清对比集，再走预览关闭（顶栏「网格」按钮 / G 共用） */
function closeToGrid() {
  if (compare.active) compare.close()
  preview.closePreview()
}

/**
 * 统计视图开关（右 rail 图标 / t 键）：进入前退出对比（对比集失效语义同
 * closeToGrid），从任意视图硬切到统计；退出回网格（对齐 compare 语义）。
 */
function toggleStats() {
  if (preview.isStats) {
    exitStats()
    return
  }
  if (compare.active) compare.close()
  preview.openStats()
}

/** 退出统计视图（Esc/G/退出按钮共用）：复位本地态，下次进入重新拉取 */
function exitStats() {
  stats.clear()
  preview.closeStats()
}

function toggleLeftPanel() {
  sidebarVisible.value = !sidebarVisible.value
}
function toggleRightPanel() {
  rightPanelVisible.value = !rightPanelVisible.value
}

/**
 * 键位 → 动作分发表：键位解析（焦点隔离/Ctrl 修饰键/键名）全在 keymap.ts，
 * 此处只把 action 名接到真实 store 调用（识别/删除已接 recognition store 与 ipc）。
 */
const keymapHandlers: KeymapHandlers = {
  // 评分：1–5 评分，0 清除。对比模式下经 markPaths 作用于聚焦格单张（选片主路径），
  // 聚焦切换用 ←/→ 或点击（见 prev/next 的 isCompare 分支）。
  rate1: () => void captures.applyRating(markPaths(), 1),
  rate2: () => void captures.applyRating(markPaths(), 2),
  rate3: () => void captures.applyRating(markPaths(), 3),
  rate4: () => void captures.applyRating(markPaths(), 4),
  rate5: () => void captures.applyRating(markPaths(), 5),
  rate0: () => void captures.applyRating(markPaths(), 0),
  // 色标：6红 7黄 8绿 9蓝，Ctrl+6 紫
  labelRed: () => void captures.applyColorLabel(markPaths(), 'Red'),
  labelYellow: () => void captures.applyColorLabel(markPaths(), 'Yellow'),
  labelGreen: () => void captures.applyColorLabel(markPaths(), 'Green'),
  labelBlue: () => void captures.applyColorLabel(markPaths(), 'Blue'),
  labelPurple: () => void captures.applyColorLabel(markPaths(), 'Purple'),
  // 旗标：P/X 标记，U 清除
  flagPick: () => void captures.applyFlag(markPaths(), 'Pick'),
  flagReject: () => void captures.applyFlag(markPaths(), 'Reject'),
  flagNone: () => void captures.applyFlag(markPaths(), null),
  // 连拍选优：K 保留组内最优帧（其余标 Reject；有选中仅作用其所在组，无选中作用于当前筛选全部连拍组）
  keepBestFrame,
  // 识别：B 单张/所选（多选时批量）、Ctrl+B 全部未识别、Ctrl+Shift+B 重新识别全部
  recognize: () => void recognition.recognize(selection.selectedPaths),
  recognizeUnrecognized: () => recognition.recognizeUnrecognized(),
  recognizeAll: () => recognition.recognizeAll(),
  // 预览：V 检测框开关
  toggleBbox: () => preview.toggleBbox(),
  // 预览：F 对焦点叠加开关（仅预览态生效，独立于 V 检测框；对比视图无叠加体系）
  toggleFocus: () => {
    if (preview.isPreview) preview.toggleFocus()
  },
  // 剪切警告叠加：'o' 键（仅预览态生效；红 = 高光溢出、蓝 = 死黑）
  toggleClipping: () => {
    if (preview.isPreview) preview.toggleClipOverlay()
  },
  // 视图：G 网格/预览切换（对比/幻灯片/统计模式下 G = 退出回到之前视图，不继续切换）
  toggleGridPreview: () => {
    if (preview.isStats) {
      stats.clear()
      preview.closeStats()
      return
    }
    if (preview.isSlideshow) {
      preview.closeSlideshow()
      return
    }
    if (preview.isCompare) {
      compare.close()
      preview.closeCompare()
      return
    }
    if (!preview.isPreview && selection.selectedIndex === null) {
      if (captures.count === 0) return
      selection.select(0)
    }
    preview.toggleView()
  },
  // 对比：C 进入（多选 2–4 张 / 连拍组前 4 张）
  compare: enterCompare,
  // 统计视图：t 进入/退出（全局鸟种索引，T1 批次）
  stats: toggleStats,
  // 缩放：= 放大 / - 缩小。锚点由视图组件注册的宿主决定（预览=视图中心、
  // 对比=聚焦格中心）；网格/幻灯片态无宿主 → no-op。
  zoomIn: () => zoomHost()?.zoomIn(),
  zoomOut: () => zoomHost()?.zoomOut(),
  // 幻灯片：s 进入（从当前选中张开始，按筛选结果顺序；对比态不进入）。
  // 退出走 Esc/G（closePreview/toggleGridPreview 的 slideshow 分支）。
  slideshow: () => {
    if (preview.isCompare || preview.isSlideshow) return
    if (selection.selectedIndex === null) {
      if (captures.count === 0) return
      selection.select(0)
    }
    preview.openSlideshow()
  },
  // 幻灯片：空格暂停/继续（仅幻灯片态生效）
  slideshowTogglePlay: () => {
    if (preview.isSlideshow) preview.toggleSlideshowPlay()
  },
  // 导航：方向键在堆叠组间 ±1 移动（堆叠后网格显示项 = 堆叠组，方向键移动到目标组
  // 激活成员；对齐 GPUI 扁平移动的「网格可见项逐个走」语义）；Home/End 跳首尾组。
  // 对比模式下 ←/→ 改为移动聚焦格（网格选择不可见，移动无意义）；
  // 幻灯片模式下 ←/→ 改为切张（切换即重置计时，见 SlideshowView watch）。
  prev: () => {
    if (preview.isSlideshow) {
      preview.slideshowStep(-1)
      return
    }
    if (preview.isCompare) compare.setFocus(compare.focus - 1)
    else selection.moveInStacks(-1)
  },
  next: () => {
    if (preview.isSlideshow) {
      preview.slideshowStep(1)
      return
    }
    if (preview.isCompare) compare.setFocus(compare.focus + 1)
    else selection.moveInStacks(1)
  },
  first: () => selection.moveToStack(0),
  last: () => selection.moveToStack(selection.stackCount - 1),
  // 堆叠：Q/E 在组内循环切换激活成员并选中（E=下一个 Q=上一个；网格态，
  // 预览/对比/幻灯片/统计下 no-op——这些视图有各自的 Q/E 需求空间，避免误触）。
  stackPrev: () => {
    if (preview.isPreview || preview.isCompare || preview.isSlideshow || preview.isStats) return
    if (selection.selectedIndex !== null) selection.cycleStackFrom(selection.selectedIndex, -1)
  },
  stackNext: () => {
    if (preview.isPreview || preview.isCompare || preview.isSlideshow || preview.isStats) return
    if (selection.selectedIndex !== null) selection.cycleStackFrom(selection.selectedIndex, 1)
  },
  // 删除：Delete 单张/所选进回收站，无确认（对齐 GPUI layout.rs Delete 键）。
  // 后端 delete_captures 删除后 emit scan:done，captures store 自动 reload，这里不重复拉取；
  // 对比模式下删聚焦格后对比集失效，直接退出对比。
  delete: () => {
    const paths = markPaths()
    if (paths.length === 0) return
    void deleteCaptures(paths)
    if (preview.isCompare) closeToGrid()
  },
  // 撤销批量操作：Ctrl+Z（后端 undo_batch_operation 执行逆操作并触发重扫；
  // 结果经状态栏 undoNotice 展示，跳过原因含「源不存在/原位置占用/副本保留」等）
  undoBatch: () => void handleUndoBatch(),
  // 选择：Ctrl+A 全选 / Ctrl+D 取消选择
  selectAll: () => selection.selectAll(),
  deselectAll: () => selection.clear(),
  // Esc：优先级对齐 GPUI layout.rs escape 分支（settings > 批量识别取消 > 对比退出 > 框选清除 > 预览退出）
  closePreview: () => {
    // 地图 overlay 分支：全屏浮层置顶，Esc 优先关地图（再次按 M 同效果）
    if (mapView.isOpen) {
      mapView.close()
      return
    }
    // settings 分支：设置弹窗打开时 Esc 优先关闭（对齐 GPUI：settings > 批量识别取消 > 框选清除 > 预览退出）
    if (settingsOpen.value) {
      settingsOpen.value = false
      return
    }
    if (recognition.running) {
      void recognition.cancel()
      return
    }
    if (preview.isCompare) {
      compare.close()
      preview.closeCompare()
      return
    }
    if (preview.isSlideshow) {
      preview.closeSlideshow()
      return
    }
    if (preview.isStats) {
      stats.clear()
      preview.closeStats()
      return
    }
    if (preview.pendingBox) {
      preview.setPendingBox(null)
      return
    }
    if (preview.isPreview) preview.closePreview()
  },
  // F5 重扫当前目录（无目录时 no-op）
  refresh: () => void captures.rescan(),
  // 面板开关：Ctrl+[ / Ctrl+]（对齐 GPUI Action::ToggleLeftPanel / ToggleRightPanel）
  toggleLeftPanel,
  toggleRightPanel,
  // 地图：M 进入/退出全屏 GPS 地图 overlay（无事件订阅，纯本地开关）
  toggleMap: () => mapView.toggle(),
}

/**
 * 撤销最近一次批量操作（Ctrl+Z）：调后端 undo_batch_operation（逆操作执行 +
 * 元数据反向同步 + 全量重扫），结果经状态栏 undoNotice 展示；跳过原因
 * （源不存在 / 原位置占用防覆盖 / 副本保留）随成功数一并提示。
 */
async function handleUndoBatch() {
  try {
    const r = await undoBatchOperation()
    const skippedText =
      r.skipped.length > 0 ? `，跳过 ${r.skipped.length} 条（${r.skipped[0][1]}${r.skipped.length > 1 ? ' 等' : ''}）` : ''
    batch.undoNotice = `撤销成功：${r.reverted} 条${skippedText}`
  } catch (e) {
    // 无日志/未打开目录等：轻提示（不弹错误框，撤销是尽力而为的操作）
    batch.undoNotice = `撤销失败：${String(e)}`
  }
}

/** keymap 安装后的卸载函数（onUnmounted 调用） */
let disposeKeymap: (() => void) | null = null

onMounted(async () => {
  captures.init()
  recognition.init()
  quality.init()
  disposeKeymap = installKeymap(keymapHandlers)
  // 主题/字体/右栏可见性跟随后端配置（config store 集中处理 DOM 应用；GPUI 版默认 Light）
  await configStore.load()
  rightPanelVisible.value = configStore.config.rightPanelVisible ?? true
})
onUnmounted(() => {
  disposeKeymap?.()
  disposeKeymap = null
})

/** 目录显示名（路径末段） */
function dirName(dir: string | null): string {
  if (!dir) return ''
  return dir.split(/[\\/]/).filter(Boolean).pop() ?? dir
}

/** 从活动栏进预览：无选中时先选首项（对齐 G 键语义） */
function toPreview() {
  if (selection.selectedIndex === null) {
    if (captures.count === 0) return
    selection.select(0)
  }
  preview.openPreview()
}

/**
 * 顶栏「网格」tab：从任意非网格视图回网格。
 * 退出分支复用 G 键 handler（toggleGridPreview）同款 store 调用与 closeToGrid，不引入新状态。
 */
function showGrid() {
  if (preview.isGrid) return
  if (preview.isStats) {
    stats.clear()
    preview.closeStats()
    return
  }
  if (preview.isSlideshow) {
    preview.closeSlideshow()
    if (preview.isGrid) return
  }
  closeToGrid()
}

/**
 * 顶栏「预览」tab：进入单图预览（无选中先选首项，对齐 G 键/左 rail 语义）。
 * 从统计/幻灯片/对比进入时先走 G 键同款退出分支，再复用 toPreview 打开预览。
 */
function showPreview() {
  if (preview.isPreview) return
  if (preview.isStats) {
    stats.clear()
    preview.closeStats()
  }
  if (preview.isSlideshow) preview.closeSlideshow()
  if (preview.isCompare) {
    compare.close()
    preview.closeCompare()
  }
  toPreview()
}

/**
 * 顶栏「统计」tab：从任意视图进统计（已统计态 no-op；
 * 对比/幻灯片退出分支与 toggleStats 同语义，不引入新状态）。
 */
function showStats() {
  if (preview.isStats) return
  if (preview.isSlideshow) preview.closeSlideshow()
  if (compare.active) compare.close()
  preview.openStats()
}

/**
 * 地图 popup「定位到网格」：关闭地图 → 回网格视图（复用 showGrid 退出分支，
 * 统计/幻灯片/对比/预览态全部收敛）→ 选中该照片。PhotoGrid 监听
 * selectedIndex 的 watch 自动滚动到可见区；若刚从其他视图切回网格，等
 * nextTick 让 PhotoGrid 挂载后再选中，保证滚动逻辑拿到真实滚动容器。
 */
async function locateFromMap(item: CaptureMeta) {
  mapView.close()
  showGrid()
  const i = captures.items.findIndex((c) => c.primaryPath === item.primaryPath)
  if (i < 0) return
  await nextTick()
  selection.select(i)
}
</script>

<template>
  <div class="flex h-screen flex-col overflow-hidden bg-background text-foreground">
    <!-- 自绘标题栏（decorations:false 后接管原生标题栏；居顶，全宽，拖拽 + 三窗口按钮） -->
    <TitleBar />

    <!-- 顶栏（全宽，对齐 GPUI toolbar 置顶，高 44px）：左 = 操作按钮 + 目录名/计数；中央 = 网格/预览下划线 tab；右 = 扫描进度/刷新/设置 -->
    <header class="flex h-11 shrink-0 items-center gap-2 border-b bg-card px-2">
      <!-- 左组：目录名（粗体截断，max-w 11rem）+ 计数（muted）。
           打开/导入/批量操作已收归左栏文件树/批量 tab 操作区 -->
      <div class="flex min-w-0 flex-1 items-center gap-2">
        <span v-if="captures.directory" class="max-w-44 truncate font-semibold" :title="captures.directory">
          {{ dirName(captures.directory) }}
        </span>
        <span v-else class="truncate font-semibold text-muted-foreground">未打开目录</span>
        <span v-if="captures.count > 0" class="shrink-0 text-xs text-muted-foreground tabular-nums">
          {{ captures.count }} 项
        </span>
      </div>

      <!-- 中央：网格/预览 下划线 tab（GPUI toolbar 签名元素；点击走 G 键同款视图切换路径） -->
      <div class="flex h-full shrink-0 items-stretch" role="tablist" aria-label="视图切换">
        <button
          type="button"
          role="tab"
          :aria-selected="preview.isGrid"
          class="relative flex h-full items-center px-3 text-sm transition-colors"
          :class="preview.isGrid ? 'text-foreground' : 'text-muted-foreground hover:text-foreground'"
          @click="showGrid"
        >
          网格
          <!-- 底部 2px 下划线；未激活 = 透明占位，防跳动 -->
          <span class="absolute inset-x-0 bottom-0 h-0.5" :class="preview.isGrid ? 'bg-primary' : 'bg-transparent'" />
        </button>
        <button
          type="button"
          role="tab"
          :aria-selected="preview.isPreview"
          class="relative flex h-full items-center px-3 text-sm transition-colors"
          :class="preview.isPreview ? 'text-foreground' : 'text-muted-foreground hover:text-foreground'"
          @click="showPreview"
        >
          预览
          <span class="absolute inset-x-0 bottom-0 h-0.5" :class="preview.isPreview ? 'bg-primary' : 'bg-transparent'" />
        </button>
        <button
          type="button"
          role="tab"
          :aria-selected="preview.isStats"
          class="relative flex h-full items-center px-3 text-sm transition-colors"
          :class="preview.isStats ? 'text-foreground' : 'text-muted-foreground hover:text-foreground'"
          @click="showStats"
        >
          统计
          <span class="absolute inset-x-0 bottom-0 h-0.5" :class="preview.isStats ? 'bg-primary' : 'bg-transparent'" />
        </button>
      </div>

      <!-- 右组：扫描进度 + 刷新（outline 小按钮）+ 设置（ghost 吸最右） -->
      <div class="flex min-w-0 flex-1 items-center justify-end gap-2">
        <!-- 扫描进度条（细条，吸右组） -->
        <div v-if="captures.scanning" class="flex items-center gap-2">
        <span class="text-xs text-muted-foreground">
          {{ captures.progress?.stage === 'scan' ? '扫描' : captures.progress?.stage === 'exif' ? 'EXIF' : '缩略图' }}
          <template v-if="captures.progress && captures.progress.total > 0">
            {{ captures.progress.done }}/{{ captures.progress.total }}
          </template>
        </span>
        <div class="h-1 w-32 overflow-hidden rounded bg-muted">
          <div
            class="h-full bg-primary transition-[width]"
            :style="{
              width:
                captures.progress && captures.progress.total > 0
                  ? `${(captures.progress.done / captures.progress.total) * 100}%`
                  : '100%',
            }"
            :class="{ 'animate-pulse': !captures.progress || captures.progress.total === 0 }"
          />
        </div>
      </div>
        <!-- 刷新目录（对齐 GPUI toolbar refresh-btn；无目录/扫描中禁用，F5 同义） -->
        <Button
          size="sm"
          variant="outline"
          class="shrink-0"
          :disabled="!captures.directory || captures.scanning"
          title="刷新目录 (F5)"
          @click="captures.rescan()"
        >
          <RefreshCwIcon data-icon="inline-start" :class="{ 'animate-spin': captures.scanning }" />
          刷新目录
        </Button>
        <!-- 设置入口（齿轮按钮，吸最右；对齐 GPUI rail 设置按钮） -->
        <Button
          size="icon-sm"
          variant="ghost"
          title="设置"
          aria-label="设置"
          @click="settingsOpen = true"
        >
          <SettingsIcon />
        </Button>
      </div>
    </header>

    <!-- 主区三栏：左 rail | 左栏 | 内容区 | 右栏 | 右 rail（对齐 GPUI layout.rs h_resizable） -->
    <div class="flex min-h-0 flex-1">
      <!-- 左 Activity Rail：48px 竖排图标（对齐 GPUI RAIL_WIDTH；打开目录在左栏文件树 tab 操作区，不重复）。
           首按钮 = 左栏显隐（与右 rail 面板切换同款交互/样式） -->
      <nav class="flex w-12 shrink-0 flex-col items-center gap-1 border-r bg-card pt-2" aria-label="左侧活动栏">
        <Button
          size="icon-sm"
          variant="ghost"
          class="text-muted-foreground hover:bg-accent"
          :title="sidebarVisible ? '隐藏侧栏 (Ctrl+[)' : '显示侧栏 (Ctrl+[)'"
          :aria-pressed="sidebarVisible"
          :class="{ 'bg-accent text-accent-foreground': sidebarVisible }"
          @click="toggleLeftPanel"
        >
          <PanelLeftIcon v-if="sidebarVisible" class="size-4" />
          <PanelLeftOpenIcon v-else class="size-4" />
        </Button>
        <Button
          size="icon-sm"
          variant="ghost"
          class="text-muted-foreground hover:bg-accent"
          title="预览"
          :class="{ 'bg-accent text-accent-foreground': preview.isPreview }"
          @click="toPreview"
        >
          <ImageIcon class="size-4" />
        </Button>
        <Button
          size="icon-sm"
          variant="ghost"
          class="text-muted-foreground hover:bg-accent"
          title="识别所选 (B)"
          :disabled="recognition.running || captures.count === 0"
          @click="recognition.recognize(selection.selectedPaths)"
        >
          <ScanSearchIcon class="size-4" />
        </Button>
      </nav>

      <!-- 左栏（目录 / 批量 双 tab，可拖宽；Ctrl+[ 隐藏/显示） -->
      <LeftPanel v-show="sidebarVisible" v-model="leftTab" />

      <!-- 内容区：筛选栏（仅 grid + 有目录）+ 空态/网格/预览 -->
      <main class="flex min-w-0 flex-1 flex-col">
        <!-- 筛选栏：仅网格视图 + 已有目录时显示（对齐 GPUI layout.rs 条件） -->
        <FilterBar v-if="captures.directory && preview.isGrid" />

        <div class="min-h-0 flex-1">
          <!-- 主区视图单链互斥：统计 → 空态 → 幻灯片 → 对比 → 预览 → 网格。
               统计/空态优先于其余视图；无目录时非统计视图一律落到空态。
               注意 v-if 链必须连续，否则 v-else 的 PhotoGrid 会在统计态下误渲染 -->
          <StatsView v-if="preview.isStats" />
          <!-- 空态：无目录时（对齐 GPUI layout.rs empty state；统计态优先于空态） -->
          <div
            v-else-if="!captures.directory"
            class="flex h-full flex-col items-center justify-center gap-3"
          >
            <GalleryVerticalEndIcon class="size-12 text-muted-foreground/20" />
            <div class="text-sm text-muted-foreground">打开目录开始浏览照片</div>
            <Button :disabled="captures.scanning" @click="captures.openDirectory()">
              <FolderOpenIcon data-icon="inline-start" />
              打开目录
            </Button>
          </div>
          <SlideshowView v-else-if="preview.isSlideshow" />
          <CompareView v-else-if="preview.isCompare" />
          <PhotoPreview v-else-if="preview.isPreview" />
          <PhotoGrid v-else />
        </div>
      </main>

      <!-- 右栏（可拖宽；Ctrl+] 隐藏/显示；内部含关闭按钮） -->
      <InfoPanel
        v-show="rightPanelVisible"
      />

      <!-- 右 Activity Rail：48px（对齐 GPUI right_rail.rs） -->
      <RightRail :visible="rightPanelVisible" @toggle="toggleRightPanel" />
    </div>

    <!-- 底部状态栏 24px -->
    <StatusBar />
    <!-- 全局右键菜单层（单一实例，store 驱动显隐；Teleport 到 body，z 高于一切浮层） -->
    <ContextMenu />
    <!-- 设置弹窗（Teleport 到 body；齿轮按钮打开、Esc/×/遮罩关闭） -->
    <SettingsModal v-model:open="settingsOpen" />
    <!-- 重复照片面板（Sidebar 底部按钮打开；自身 store 管理显隐，对齐 ExportDialog 模式） -->
    <DuplicatesPanel />
    <!-- 导入弹窗（文件树 tab「导入」按钮打开；选源 → 计划预览 → 执行） -->
    <ImportDialog v-model:open="importDialog.open" />
    <!-- 导出弹窗（右键菜单「导出…」/ 批量面板「导出」打开；自身 store 管理显隐） -->
    <ExportDialog />
    <!-- K 键确认弹窗：连拍组自动选优 → 非最优帧批量标 Reject（组数 + 将标记张数确认） -->
    <Dialog :open="keepBest !== null" @update:open="(v: boolean) => !v && (keepBest = null)">
      <DialogContent class="sm:max-w-md">
        <DialogHeader>
          <DialogTitle class="flex items-center gap-1.5 text-amber-300">
            <CrownIcon class="size-4" />
            连拍组自动选优
          </DialogTitle>
          <DialogDescription>
            将按眼锐度（并列取文件更大、再取路径序，无锐度信息时垫底）保留
            {{ keepBest?.groupCount }} 个连拍组的最优帧，并把
            {{ keepBest?.markCount }} 张非最优帧标记为 Reject（可被 U 键清除）。确认继续？
          </DialogDescription>
        </DialogHeader>
        <DialogFooter class="justify-between" :show-close-button="false">
          <DialogClose as-child>
            <Button size="sm" variant="outline">取消</Button>
          </DialogClose>
          <Button size="sm" variant="destructive" @click="confirmKeepBest">标记 Reject</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
    <!-- 地图 overlay（M 键开关；全屏浮层 z-50 参照设置弹窗；Esc/关闭按钮/M 键退出） -->
    <MapView v-if="mapView.isOpen" @locate="locateFromMap" />
    <!-- 识别纠错弹窗（InfoPanel「纠正…」/ 网格右键「纠正鸟种…」打开；自身 store 管理显隐） -->
    <SpeciesCorrectDialog />
    <!-- 窗口边缘缩放热区（decorations:false 无原生边框；8 向手柄，顶层覆盖全部浮层之下） -->
    <WindowResizeEdges />
  </div>
</template>
