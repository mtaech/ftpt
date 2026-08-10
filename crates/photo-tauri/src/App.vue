<script setup lang="ts">
// 根布局（对齐 GPUI layout.rs 三栏结构）：
//   顶栏（全宽）→ 主区 h_flex：左 rail | 左栏(Sidebar) | 内容区 | 右栏(InfoPanel) | 右 rail
//   → 底部 StatusBar。
// 左/右栏可独立隐藏（Ctrl+[ / Ctrl+] 切换，右栏初始值跟随后端配置），
// 拖宽把手在各面板内部（宽度 localStorage 持久化，范围 200–480）。全局快捷键见 keymap.ts。
import { onMounted, onUnmounted, ref } from 'vue'
import {
  FolderOpenIcon,
  GalleryVerticalEndIcon,
  ImageIcon,
  LayoutGridIcon,
  ListChecksIcon,
  ScanSearchIcon,
  SettingsIcon,
} from '@lucide/vue'
import { Button } from '@/components/ui/button'
import PhotoGrid from '@/components/PhotoGrid.vue'
import PhotoPreview from '@/components/PhotoPreview.vue'
import FilterBar from '@/components/FilterBar.vue'
import Sidebar from '@/components/Sidebar.vue'
import BatchOpsPanel from '@/components/BatchOpsPanel.vue'
import InfoPanel from '@/components/InfoPanel.vue'
import RightRail from '@/components/RightRail.vue'
import StatusBar from '@/components/StatusBar.vue'
import ContextMenu from '@/components/ContextMenu.vue'
import SettingsModal from '@/components/SettingsModal.vue'
import { useCapturesStore } from '@/stores/captures'
import { useSelectionStore } from '@/stores/selection'
import { usePreviewStore } from '@/stores/preview'
import { useRecognitionStore } from '@/stores/recognition'
import { useConfigStore } from '@/stores/config'
import { installKeymap, type KeymapHandlers } from '@/keymap'
import { deleteCaptures } from '@/lib/ipc'

const captures = useCapturesStore()
const selection = useSelectionStore()
const preview = usePreviewStore()
const recognition = useRecognitionStore()
const configStore = useConfigStore()

/** 左栏可见（对齐 GPUI sidebar_visible，运行时状态，默认可见） */
const sidebarVisible = ref(true)
/** 批量操作面板可见（顶栏按钮切换；独立面板，默认隐藏） */
const batchPanelVisible = ref(false)
/** 右栏可见（对齐 GPUI config.right_panel_visible，初始值跟随后端配置，默认可见） */
const rightPanelVisible = ref(true)
/** 设置弹窗开关（顶栏齿轮按钮打开、Esc 分支关闭，v-model 传给 SettingsModal） */
const settingsOpen = ref(false)

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
  // 评分：1–5 评分，0 清除
  rate1: () => void captures.applyRating(selection.selectedPaths, 1),
  rate2: () => void captures.applyRating(selection.selectedPaths, 2),
  rate3: () => void captures.applyRating(selection.selectedPaths, 3),
  rate4: () => void captures.applyRating(selection.selectedPaths, 4),
  rate5: () => void captures.applyRating(selection.selectedPaths, 5),
  rate0: () => void captures.applyRating(selection.selectedPaths, 0),
  // 色标：6红 7黄 8绿 9蓝
  labelRed: () => void captures.applyColorLabel(selection.selectedPaths, 'Red'),
  labelYellow: () => void captures.applyColorLabel(selection.selectedPaths, 'Yellow'),
  labelGreen: () => void captures.applyColorLabel(selection.selectedPaths, 'Green'),
  labelBlue: () => void captures.applyColorLabel(selection.selectedPaths, 'Blue'),
  // 旗标：P/X 标记，U 清除
  flagPick: () => void captures.applyFlag(selection.selectedPaths, 'Pick'),
  flagReject: () => void captures.applyFlag(selection.selectedPaths, 'Reject'),
  flagNone: () => void captures.applyFlag(selection.selectedPaths, null),
  // 识别：B 单张/所选（多选时批量）、Ctrl+B 全部未识别、Ctrl+Shift+B 重新识别全部
  recognize: () => void recognition.recognize(selection.selectedPaths),
  recognizeUnrecognized: () => recognition.recognizeUnrecognized(),
  recognizeAll: () => recognition.recognizeAll(),
  // 预览：V 检测框开关
  toggleBbox: () => preview.toggleBbox(),
  // 视图：G 网格/预览切换（无选中时先选首项再进预览，保留原逻辑）
  toggleGridPreview: () => {
    if (!preview.isPreview && selection.selectedIndex === null) {
      if (captures.count === 0) return
      selection.select(0)
    }
    preview.toggleView()
  },
  // 导航：方向键扁平 ±1 移动（4 列网格下跨行自然发生，对齐 GPUI）；Home/End 跳首尾
  prev: () => selection.move(-1),
  next: () => selection.move(1),
  first: () => selection.moveTo(0),
  last: () => selection.moveTo(captures.count - 1),
  // 删除：Delete 单张/所选进回收站，无确认（对齐 GPUI layout.rs Delete 键）。
  // 后端 delete_captures 删除后 emit scan:done，captures store 自动 reload，这里不重复拉取
  delete: () => {
    const paths = selection.selectedPaths
    if (paths.length === 0) return
    void deleteCaptures(paths)
  },
  // 选择：Ctrl+A 全选 / Ctrl+D 取消选择
  selectAll: () => selection.selectAll(),
  deselectAll: () => selection.clear(),
  // Esc：优先级对齐 GPUI layout.rs escape 分支（settings > 批量识别取消 > 框选清除 > 预览退出）
  closePreview: () => {
    // settings 分支：设置弹窗打开时 Esc 优先关闭（对齐 GPUI：settings > 批量识别取消 > 框选清除 > 预览退出）
    if (settingsOpen.value) {
      settingsOpen.value = false
      return
    }
    if (recognition.running) {
      void recognition.cancel()
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
}

/** keymap 安装后的卸载函数（onUnmounted 调用） */
let disposeKeymap: (() => void) | null = null

onMounted(async () => {
  captures.init()
  recognition.init()
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
</script>

<template>
  <div class="flex h-screen flex-col overflow-hidden bg-background text-foreground">
    <!-- 顶栏（全宽，对齐 GPUI toolbar 置顶）：打开目录 / 目录名 / 计数 / 扫描进度 -->
    <header class="flex h-9 shrink-0 items-center gap-2 border-b px-2">
      <Button size="sm" variant="secondary" :disabled="captures.scanning" @click="captures.openDirectory()">
        <FolderOpenIcon data-icon="inline-start" />
        打开目录
      </Button>
      <!-- 批量操作面板开关（BatchOpsUi 区域） -->
      <Button
        size="sm"
        variant="ghost"
        :class="{ 'bg-accent text-accent-foreground': batchPanelVisible }"
        @click="batchPanelVisible = !batchPanelVisible"
      >
        <ListChecksIcon data-icon="inline-start" />
        批量操作
      </Button>
      <span v-if="captures.directory" class="truncate text-sm" :title="captures.directory">
        {{ dirName(captures.directory) }}
      </span>
      <span v-else class="truncate text-sm text-muted-foreground">未打开目录</span>
      <span v-if="captures.count > 0" class="shrink-0 text-xs text-muted-foreground font-mono-num">
        {{ captures.count }} 项
      </span>
      <!-- 扫描进度条（细条，吸顶栏右侧） -->
      <div v-if="captures.scanning" class="ml-auto flex items-center gap-2">
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
      <!-- 设置入口（齿轮按钮，吸顶栏最右；对齐 GPUI rail 设置按钮，入口放顶栏右侧） -->
      <Button
        size="icon-sm"
        variant="ghost"
        class="ml-auto"
        title="设置"
        aria-label="设置"
        @click="settingsOpen = true"
      >
        <SettingsIcon />
      </Button>
    </header>

    <!-- 主区三栏：左 rail | 左栏 | 内容区 | 右栏 | 右 rail（对齐 GPUI layout.rs h_resizable） -->
    <div class="flex min-h-0 flex-1">
      <!-- 左 Activity Rail：48px 竖排图标（对齐 GPUI RAIL_WIDTH） -->
      <nav class="flex w-12 shrink-0 flex-col items-center gap-1 border-r bg-sidebar pt-2">
        <Button
          size="icon-sm"
          variant="ghost"
          title="打开目录"
          :disabled="captures.scanning"
          @click="captures.openDirectory()"
        >
          <FolderOpenIcon class="size-4" />
        </Button>
        <Button
          size="icon-sm"
          variant="ghost"
          title="浏览 / 网格"
          :class="{ 'bg-accent text-accent-foreground': !preview.isPreview }"
          @click="preview.closePreview()"
        >
          <LayoutGridIcon class="size-4" />
        </Button>
        <Button
          size="icon-sm"
          variant="ghost"
          title="预览"
          :class="{ 'bg-accent text-accent-foreground': preview.isPreview }"
          @click="toPreview"
        >
          <ImageIcon class="size-4" />
        </Button>
        <Button
          size="icon-sm"
          variant="ghost"
          title="识别所选 (B)"
          :disabled="recognition.running || captures.count === 0"
          @click="recognition.recognize(selection.selectedPaths)"
        >
          <ScanSearchIcon class="size-4" />
        </Button>
      </nav>

      <!-- 左栏（可拖宽；Ctrl+[ 隐藏/显示） -->
      <Sidebar v-show="sidebarVisible" />

      <!-- 批量操作面板（BatchOpsUi 区域；顶栏按钮切换） -->
      <BatchOpsPanel v-show="batchPanelVisible" />

      <!-- 内容区：筛选栏（仅 grid + 有目录）+ 空态/网格/预览 -->
      <main class="flex min-w-0 flex-1 flex-col">
        <!-- 筛选栏：仅网格视图 + 已有目录时显示（对齐 GPUI layout.rs 条件） -->
        <FilterBar v-if="captures.directory" />

        <div class="min-h-0 flex-1">
          <!-- 空态：无目录时（对齐 GPUI layout.rs empty state） -->
          <div
            v-if="!captures.directory"
            class="flex h-full flex-col items-center justify-center gap-3"
          >
            <GalleryVerticalEndIcon class="size-12 text-muted-foreground/20" />
            <div class="text-sm text-muted-foreground">打开目录开始浏览照片</div>
            <Button :disabled="captures.scanning" @click="captures.openDirectory()">
              <FolderOpenIcon data-icon="inline-start" />
              打开目录
            </Button>
          </div>
          <PhotoPreview v-else-if="preview.isPreview" />
          <PhotoGrid v-else />
        </div>
      </main>

      <!-- 右栏（可拖宽；Ctrl+] 隐藏/显示；内部含关闭按钮） -->
      <InfoPanel
        v-show="rightPanelVisible"
        :visible="rightPanelVisible"
        @toggle="toggleRightPanel"
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
  </div>
</template>
