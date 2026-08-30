<script setup lang="ts">
// 自绘标题栏（Tauri v2，tauri.conf.json decorations:false 后接管原生标题栏）：
// 拖拽区（data-tauri-drag-region）+ 双击最大化/还原（Tauri drag.js 原生注入）+
// 最小化 / 最大化(还原) / 关闭 三按钮；最大化状态经 window API 同步图标。
// 浏览器纯 vite dev（无 __TAURI__）下降级为无操作容器，不影响 mock 调试。
import { onMounted, onUnmounted, ref } from 'vue'
import { MinusIcon, SquareIcon, CopyIcon, XIcon } from '@lucide/vue'

/** 是否运行在真实 Tauri webview（false = mock 模式，窗口控件不可用） */
const isTauri =
  typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

/** 当前是否最大化（影响还原按钮图标；Tauri 下初始化拉取并监听 resize 周期刷新） */
const isMaximized = ref(false)

// 动态 import：Tauri 环境才拉取 window API，mock 环境（无 __TAURI__）保持可用
async function getWindow() {
  const { getCurrentWindow } = await import('@tauri-apps/api/window')
  return getCurrentWindow()
}

/** 刷新最大化状态（双击/按钮切换后重读；resize 事件周期刷新兜底） */
async function refreshMaximized() {
  if (!isTauri) return
  try {
    isMaximized.value = await (await getWindow()).isMaximized()
  } catch {
    // 权限缺失或窗口不可用时静默，不阻塞标题栏交互
  }
}

function minimize() {
  if (!isTauri) return
  void getWindow().then((w) => w.minimize())
}

function toggleMaximize() {
  if (!isTauri) return
  void getWindow()
    .then((w) => w.toggleMaximize())
    .then(refreshMaximized)
}

function closeApp() {
  if (!isTauri) return
  void getWindow().then((w) => w.close())
}

let unlistenResize: (() => void) | null = null

onMounted(async () => {
  if (!isTauri) return
  try {
    const w = await getWindow()
    await refreshMaximized()
    // 最大化/还原都会触发 resize 事件（窗口内容尺寸变化），借此同步图标
    unlistenResize = await w.onResized(() => void refreshMaximized())
  } catch {
    // 权限缺失静默
  }
})

onUnmounted(() => {
  unlistenResize?.()
  unlistenResize = null
})
</script>

<template>
  <!-- 自绘标题栏：拖拽区（data-tauri-drag-region="deep"：子元素任意点击均可拖拽）+
       窗口控制按钮（吸右，clickable 元素自身不拖拽——drag.js 对 button 直接返回 false）。
       禁用文本选中（拖拽手感），与 body user-select:none 等同。 -->
  <header
    data-tauri-drag-region="deep"
    class="flex h-9 shrink-0 items-stretch justify-between border-b border-border bg-card select-none"
  >
    <!-- 左：应用名（拖拽区的视觉锚点；为空时保持拖拽手感） -->
    <div class="flex items-center px-3 text-xs font-medium text-muted-foreground">
      ftpt
    </div>
    <!-- 右：窗口控制按钮 -->
    <div class="flex shrink-0 items-stretch">
      <button
        type="button"
        class="flex w-11 items-center justify-center text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        title="最小化"
        aria-label="最小化"
        @click="minimize"
      >
        <MinusIcon class="size-4" />
      </button>
      <button
        type="button"
        class="flex w-11 items-center justify-center text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        :title="isMaximized ? '还原' : '最大化'"
        :aria-label="isMaximized ? '还原' : '最大化'"
        @click="toggleMaximize"
      >
        <CopyIcon v-if="isMaximized" class="size-4" />
        <SquareIcon v-else class="size-4" />
      </button>
      <button
        type="button"
        class="flex w-11 items-center justify-center text-muted-foreground transition-colors hover:bg-destructive hover:text-destructive-foreground"
        title="关闭"
        aria-label="关闭"
        @click="closeApp"
      >
        <XIcon class="size-4" />
      </button>
    </div>
  </header>
</template>
