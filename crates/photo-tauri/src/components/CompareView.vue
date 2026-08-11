<script setup lang="ts">
// 对比模式（T0 批次）：2–4 张多窗格并排，每格 ptimg master 图源。
// 滚轮以光标为中心缩放（×1.25）、左键拖拽平移，作用于全部窗格（缩放系数相对各图
// fit、平移按显示尺寸占比归一化，数学走 previewMath / compare store，与 GPUI 一致）。
// 点击/←→ 方向键聚焦某格（高亮边框，方向键经 App.vue 键位层路由到 compare.setFocus），
// 评分 1–5/0、色标、旗标、Delete 等标记键作用于聚焦格（App.vue markPaths 路由）。
// 布局/主题对齐现有预览（Catppuccin CSS 变量 + 半透明黑底徽标 + 加载脉冲浮层）。
import { nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { ImageIcon, LayersIcon, XIcon } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import { useCapturesStore } from '@/stores/captures'
import { useCompareStore } from '@/stores/compare'
import { usePreviewStore } from '@/stores/preview'
import { ptimgUrl } from '@/lib/ipc'
import { registerZoomHost } from '@/lib/zoomHost'
import { displayName } from '@/lib/format'
import type { Vec2 } from '@/lib/previewMath'

const captures = useCapturesStore()
const compare = useCompareStore()
const preview = usePreviewStore()

/** 窗格网格容器（ResizeObserver 观察它，实测各 pane 尺寸） */
const gridEl = ref<HTMLElement | null>(null)
/** v-for 收集的 pane 元素（下标 = 对比序，顺序稳定） */
const paneEls = ref<HTMLElement[]>([])
/** 每格容器尺寸（pane clientWidth/Height） */
const paneSizes = ref<Vec2[]>([])
/** 每格原图自然尺寸（img load 后回填） */
const naturalSizes = ref<Vec2[]>([])
/** 每格图源加载中（切集/切图源时置真，加载完成淡出） */
const loading = ref<boolean[]>([])

/** 对比集变化（进入/换集）：复位每格容器/自然尺寸/加载态，重测尺寸 */
watch(
  () => compare.indices,
  async () => {
    const n = compare.indices.length
    paneSizes.value = new Array(n).fill([0, 0])
    naturalSizes.value = new Array(n).fill([0, 0])
    loading.value = new Array(n).fill(true)
    await nextTick()
    measurePanes()
  },
  { immediate: true },
)

/** 实测各 pane 尺寸（grid 容器 resize / 挂载 / 换集后调用） */
function measurePanes() {
  for (let k = 0; k < paneEls.value.length; k++) {
    const el = paneEls.value[k]
    if (el) paneSizes.value[k] = [el.clientWidth, el.clientHeight]
  }
}

let ro: ResizeObserver | null = null
onMounted(() => {
  ro = new ResizeObserver(measurePanes)
  if (gridEl.value) ro.observe(gridEl.value)
  // 缩放键（=/−）宿主：聚焦格中心锚点（App.vue 键位层分发）
  registerZoomHost({ zoomIn: () => zoomByFocus(1), zoomOut: () => zoomByFocus(-1) })
})
onUnmounted(() => {
  ro?.disconnect()
  registerZoomHost(null)
})

/** 第 k 格拍摄 */
function paneItem(k: number) {
  return captures.items[compare.indices[k]] ?? null
}

/** 第 k 格图源：master 母版 + 缩略图版本号做缓存破坏 */
function imgSrc(k: number): string {
  const c = paneItem(k)
  if (!c) return ''
  return ptimgUrl('master', c.primaryPath, captures.thumbVersions[c.primaryPath])
}

/** 第 k 格图片渲染定位（显示尺寸 + 归一化平移 → 像素，公式 = previewMath） */
function imgStyle(k: number): Record<string, string> {
  const container = paneSizes.value[k] ?? [0, 0]
  const natural = naturalSizes.value[k] ?? [0, 0]
  const disp = compare.paneDisp(natural, container)
  const origin = compare.paneOrigin(disp, container)
  return {
    width: `${disp[0]}px`,
    height: `${disp[1]}px`,
    transform: `translate3d(${origin[0]}px, ${origin[1]}px, 0)`,
  }
}

function onPaneImgLoad(k: number, e: Event) {
  const img = e.target as HTMLImageElement
  naturalSizes.value[k] = [img.naturalWidth, img.naturalHeight]
  loading.value[k] = false
}

/** 滚轮缩放（光标为中心）：只读当前 pane 的容器/自然尺寸，缩放状态全窗格共享 */
function onPaneWheel(k: number, e: WheelEvent) {
  const el = paneEls.value[k]
  const container = paneSizes.value[k] ?? [0, 0]
  if (!el || container[0] <= 0) return
  const rect = el.getBoundingClientRect()
  const cursor: Vec2 = [e.clientX - rect.left, e.clientY - rect.top]
  compare.zoomBy(e.deltaY < 0 ? 1 : -1, container, naturalSizes.value[k] ?? [0, 0], cursor)
}

/** 缩放键（=/−）宿主：以聚焦格容器中心为锚点（App.vue 键位层分发；滚轮语义同 zoomBy） */
function zoomByFocus(direction: 1 | -1) {
  const k = compare.focus
  const container = paneSizes.value[k] ?? [0, 0]
  const natural = naturalSizes.value[k] ?? [0, 0]
  if (container[0] <= 0 || natural[0] <= 0) return
  compare.zoomBy(direction, container, natural, [container[0] / 2, container[1] / 2])
}

// 左键拖拽平移（作用于全部窗格；指针捕获保证拖出窗格仍持续）
let dragging = false
let lastX = 0
let lastY = 0

function onPanePointerDown(e: PointerEvent) {
  if (e.button !== 0) return
  const el = e.currentTarget as HTMLElement
  dragging = true
  lastX = e.clientX
  lastY = e.clientY
  el.setPointerCapture(e.pointerId)
}

function onPanePointerMove(k: number, e: PointerEvent) {
  if (!dragging) return
  const disp = compare.paneDisp(
    naturalSizes.value[k] ?? [0, 0],
    paneSizes.value[k] ?? [0, 0],
  )
  compare.panBy(e.clientX - lastX, e.clientY - lastY, disp, paneSizes.value[k] ?? [0, 0])
  lastX = e.clientX
  lastY = e.clientY
}

function onPanePointerUp(e: PointerEvent) {
  const el = e.currentTarget as HTMLElement
  el.releasePointerCapture(e.pointerId)
  dragging = false
}

/** 退出对比（顶部条按钮；Esc/G 由 App.vue 键位层处理，同样清 compare store） */
function exitCompare() {
  compare.close()
  preview.closeCompare()
}
</script>

<template>
  <div class="flex h-full flex-col bg-background">
    <!-- 顶部条：标题 + 操作提示 + 退出（对齐顶栏卡片底色） -->
    <div class="flex h-9 shrink-0 items-center gap-2 border-b bg-card px-2">
      <LayersIcon class="size-3.5 text-muted-foreground" />
      <span class="text-xs font-medium">{{ compare.count }} 张对比</span>
      <span class="hidden truncate text-xs text-muted-foreground md:inline">
        滚轮缩放 · 拖拽平移 · ←/→ 或点击聚焦 · 1–5 评分聚焦格 · Esc / G 退出
      </span>
      <Button size="sm" variant="ghost" class="ml-auto" @click="exitCompare">
        <XIcon data-icon="inline-start" />
        退出对比
      </Button>
    </div>

    <!-- 窗格网格：2 张左右并排，3–4 张 2×2 -->
    <div ref="gridEl" class="min-h-0 flex-1 p-1.5">
      <div
        class="grid h-full gap-1.5"
        :class="compare.count === 2 ? 'grid-cols-2' : 'grid-cols-2 grid-rows-2'"
      >
        <div
          v-for="(idx, k) in compare.indices"
          :key="idx"
          ref="paneEls"
          class="group relative min-h-0 touch-none overflow-hidden rounded-md border bg-muted select-none"
          :class="k === compare.focus ? 'border-primary ring-2 ring-primary/60' : 'border-border'"
          @wheel.prevent="onPaneWheel(k, $event)"
          @pointerdown="onPanePointerDown($event)"
          @pointermove="onPanePointerMove(k, $event)"
          @pointerup="onPanePointerUp"
          @pointercancel="onPanePointerUp"
          @click="compare.setFocus(k)"
          @dblclick="compare.zoomFit()"
          @contextmenu.prevent
        >
          <!-- 加载中占位（居中图标脉冲，对齐 PhotoPreview 加载浮层风格） -->
          <Transition name="loading-fade">
            <div v-if="loading[k]" class="absolute inset-0 flex items-center justify-center">
              <ImageIcon class="size-8 animate-pulse text-muted-foreground/40" />
            </div>
          </Transition>

          <img
            v-if="paneItem(k)"
            :key="imgSrc(k)"
            :src="imgSrc(k)"
            :alt="displayName(paneItem(k)!)"
            draggable="false"
            class="absolute top-0 left-0 max-w-none transition-opacity duration-200 ease-out"
            :class="loading[k] ? 'opacity-0' : 'opacity-100'"
            :style="imgStyle(k)"
            @load="onPaneImgLoad(k, $event)"
          />

          <!-- 文件名标签（左上，半透明黑底，对齐网格格式/旗标徽标风格） -->
          <span
            class="absolute top-1 left-1 max-w-[70%] truncate rounded-sm bg-black/70 px-1 text-[0.625rem] leading-4 text-white"
            :title="paneItem(k) ? displayName(paneItem(k)!) : ''"
          >
            {{ k + 1 }} · {{ paneItem(k) ? displayName(paneItem(k)!) : '' }}
          </span>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* 加载占位淡入淡出（对齐 PhotoPreview loading-fade：200ms 强 ease-out） */
.loading-fade-enter-active,
.loading-fade-leave-active {
  transition: opacity 200ms cubic-bezier(0.23, 1, 0.32, 1);
}
.loading-fade-enter-from,
.loading-fade-leave-to {
  opacity: 0;
}
</style>
