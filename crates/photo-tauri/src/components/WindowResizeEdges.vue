<script setup lang="ts">
// 窗口边缘缩放把手（decorations:false 无原生边框，需自绘 4 边 8 向缩放热区）。
// Tauri window.startResizeDragging(direction) 按方向拖拽缩放；mock 模式降级 no-op。
// 覆盖 6px 边缘热区 8 个方向，保持最小可命中尺寸；仅真实 webview 生效。
import { onMounted, onUnmounted, ref } from 'vue'

/** startResizeDragging 的方向参数（@tauri-apps/api/window 未导出该类型，用字面量联合对齐） */
type ResizeDirection =
  | 'East'
  | 'North'
  | 'NorthEast'
  | 'NorthWest'
  | 'South'
  | 'SouthEast'
  | 'SouthWest'
  | 'West'

/** 是否运行在真实 Tauri webview（false = mock 模式，缩放手柄不可用） */
const isTauri =
  typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

/** 是否最大化（对齐原生行为：最大化时边缘不可再缩放，隐藏手柄） */
const isMaximized = ref(false)

async function getWindow() {
  const { getCurrentWindow } = await import('@tauri-apps/api/window')
  return getCurrentWindow()
}

async function refreshMaximized() {
  if (!isTauri) return
  try {
    isMaximized.value = await (await getWindow()).isMaximized()
  } catch {
    // 权限缺失静默
  }
}

async function resize(dir: ResizeDirection) {
  if (!isTauri) return
  try {
    await (await getWindow()).startResizeDragging(dir)
  } catch {
    // 权限缺失静默（capabilities 已含 allow-start-resize-dragging）
  }
}

let unlistenResize: (() => void) | null = null

onMounted(async () => {
  if (!isTauri) return
  try {
    const w = await getWindow()
    await refreshMaximized()
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
  <!-- 8 个方向的手柄，绝对定位于窗口四角/四边。class 用 fixed 覆盖而非仅 absolute，
       因根容器 overflow-hidden；fixed 挂在 viewport（即窗口）上与其对齐。
       仅真实 webview 渲染（mock 模式无窗口可缩，避免空手柄拦截边缘点击）；
       最大化时整体隐藏（v-if，避免边缘误拖拽）。 -->
  <div v-if="isTauri && !isMaximized" class="pointer-events-none fixed inset-0 z-[100]">
    <div class="pointer-events-auto absolute -top-0.5 left-3 right-3 h-1.5 cursor-n-resize" @pointerdown="resize('North')" />
    <div class="pointer-events-auto absolute -bottom-0.5 left-3 right-3 h-1.5 cursor-s-resize" @pointerdown="resize('South')" />
    <div class="pointer-events-auto absolute -left-0.5 top-3 bottom-3 w-1.5 cursor-w-resize" @pointerdown="resize('West')" />
    <div class="pointer-events-auto absolute -right-0.5 top-3 bottom-3 w-1.5 cursor-e-resize" @pointerdown="resize('East')" />
    <div class="pointer-events-auto absolute -top-0.5 -left-0.5 size-2 cursor-nw-resize" @pointerdown="resize('NorthWest')" />
    <div class="pointer-events-auto absolute -top-0.5 -right-0.5 size-2 cursor-ne-resize" @pointerdown="resize('NorthEast')" />
    <div class="pointer-events-auto absolute -bottom-0.5 -left-0.5 size-2 cursor-sw-resize" @pointerdown="resize('SouthWest')" />
    <div class="pointer-events-auto absolute -bottom-0.5 -right-0.5 size-2 cursor-se-resize" @pointerdown="resize('SouthEast')" />
  </div>
</template>
