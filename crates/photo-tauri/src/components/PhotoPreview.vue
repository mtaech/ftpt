<script setup lang="ts">
// 单图预览：ptimg master 图源（1:1 时切 full），滚轮光标中心缩放（×1.25 步进，
// 数学走 previewMath 纯函数），左键拖拽平移，工具条 −/%/+/适应/1:1/返回网格。
import { computed, onMounted, onUnmounted, ref, useTemplateRef, watch } from 'vue'
import { MinusIcon, PlusIcon, MaximizeIcon, ScanIcon, ScanLineIcon, Grid2x2Icon } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import Filmstrip from '@/components/Filmstrip.vue'
import { useCapturesStore } from '@/stores/captures'
import { useSelectionStore } from '@/stores/selection'
import { usePreviewStore } from '@/stores/preview'
import { useContextMenuStore, captureMenuItems } from '@/stores/contextMenu'
import { ptimgUrl } from '@/lib/ipc'
import { displayName } from '@/lib/format'
import type { BBox } from '@/lib/bindings'
import type { Vec2 } from '@/lib/previewMath'

/** 图片区内边距（对齐 GPUI window_pos_to_image_norm 的 p_4 = 16px） */
const PAD = 16

const captures = useCapturesStore()
const selection = useSelectionStore()
const preview = usePreviewStore()
const contextMenu = useContextMenuStore()

const containerRef = useTemplateRef<HTMLElement>('container')
/** 扣除内边距后的可用容器尺寸 */
const containerSize = ref<Vec2>([0, 0])
/** 原图自然尺寸（img 加载完成后可读） */
const natural = ref<Vec2>([0, 0])
/** 当前图源加载中（切图/切 master↔full 时置真） */
const loading = ref(true)

const current = computed(() => selection.selected)

/** 显示尺寸与定位（渲染公式 = previewMath，与 GPUI 一致） */
const disp = computed<Vec2>(() => preview.displaySize(containerSize.value, natural.value))
const origin = computed<Vec2>(() => preview.imageOrigin(disp.value, containerSize.value))

// ── 检测框 / 框选叠加 ──────────────────────────────────────────────
/** Shift 是否按住（光标十字提示） */
const shiftHeld = ref(false)
/** 框选实时框（容器坐标，已钳制到图片范围；非空 = 正在拖拽画框） */
const drawBox = ref<{ x1: number; y1: number; x2: number; y2: number } | null>(null)

/**
 * 归一化框（0–1）→ 容器内像素矩形。基于同一 disp/origin，
 * 缩放平移时叠加框自动跟随。
 */
function normRectToPx(b: BBox): { left: number; top: number; width: number; height: number } {
  const [dw, dh] = disp.value
  const [ox, oy] = origin.value
  // specta 对 f32 字段防御性标为 number | null；后端实际恒有值，null 时按 0 兜底
  const x1 = Math.min(b.x1 ?? 0, b.x2 ?? 0)
  const y1 = Math.min(b.y1 ?? 0, b.y2 ?? 0)
  const x2 = Math.max(b.x1 ?? 0, b.x2 ?? 0)
  const y2 = Math.max(b.y1 ?? 0, b.y2 ?? 0)
  return {
    left: ox + PAD + x1 * dw,
    top: oy + PAD + y1 * dh,
    width: (x2 - x1) * dw,
    height: (y2 - y1) * dh,
  }
}

/** 容器坐标 → 图片归一化坐标（越界钳制 0–1；图未加载返回 null） */
function toNorm(p: Vec2): Vec2 | null {
  const [dw, dh] = disp.value
  const [ox, oy] = origin.value
  if (dw <= 0 || dh <= 0) return null
  return [
    Math.min(Math.max((p[0] - PAD - ox) / dw, 0), 1),
    Math.min(Math.max((p[1] - PAD - oy) / dh, 0), 1),
  ]
}

/** 容器坐标 → 钳制到图片显示范围（拖出图片时框停在边缘） */
function clampToImg(p: Vec2): Vec2 {
  const [dw, dh] = disp.value
  const [ox, oy] = origin.value
  return [
    Math.min(Math.max(p[0], ox + PAD), ox + PAD + dw),
    Math.min(Math.max(p[1], oy + PAD), oy + PAD + dh),
  ]
}

/** 检测框叠加：当前图有 birdBbox 且开启 bboxVisible 时显示 */
const bboxRect = computed(() => {
  const b = current.value?.birdBbox
  if (!b || !preview.bboxVisible) return null
  return normRectToPx(b)
})

/** 框选实时框（跟随拖拽，accent 色） */
const drawRect = computed(() => {
  const b = drawBox.value
  if (!b) return null
  return {
    left: Math.min(b.x1, b.x2),
    top: Math.min(b.y1, b.y2),
    width: Math.abs(b.x2 - b.x1),
    height: Math.abs(b.y2 - b.y1),
  }
})

/** 待确认框（虚线，pendingBox 归一化 → 像素） */
const pendingRect = computed(() => {
  const b = preview.pendingBox
  if (!b) return null
  return normRectToPx(b)
})

/** 图源：1:1 用 full 全尺寸母版，其余用 master 预览母版 */
const imgSrc = computed(() => {
  const c = current.value
  if (!c) return ''
  const kind = preview.isOneToOne ? 'full' : 'master'
  return ptimgUrl(kind, c.primaryPath, captures.thumbVersions[c.primaryPath])
})

function onImgLoad(e: Event) {
  const img = e.target as HTMLImageElement
  natural.value = [img.naturalWidth, img.naturalHeight]
  loading.value = false
}

// 切图：复位缩放平移 + 重新加载（待确认框属于当前图，一并清空）
watch(
  () => current.value?.primaryPath,
  () => {
    preview.resetView()
    preview.setPendingBox(null)
    natural.value = [0, 0]
    loading.value = true
  },
)
// 1:1 切换换图源，同样进入加载态
watch(
  () => preview.isOneToOne,
  () => {
    loading.value = true
  },
)
// 容器尺寸/原图尺寸变化时重新钳制平移（窗口缩放等场景）
watch([containerSize, natural], () => {
  preview.clampPan(disp.value, containerSize.value)
})

let resizeObserver: ResizeObserver | null = null
onMounted(() => {
  const el = containerRef.value
  if (!el) return
  resizeObserver = new ResizeObserver(() => {
    containerSize.value = [Math.max(el.clientWidth - PAD * 2, 1), Math.max(el.clientHeight - PAD * 2, 1)]
  })
  resizeObserver.observe(el)
  window.addEventListener('keydown', onKeyDown)
  window.addEventListener('keyup', onKeyUp)
})
onUnmounted(() => {
  resizeObserver?.disconnect()
  window.removeEventListener('keydown', onKeyDown)
  window.removeEventListener('keyup', onKeyUp)
})

/** Shift 光标提示 + Esc 清除待确认框（App.vue 键位层的 Esc 逻辑幂等兼容） */
function onKeyDown(e: KeyboardEvent) {
  if (e.key === 'Shift') shiftHeld.value = true
  else if (e.key === 'Escape' && preview.pendingBox) preview.setPendingBox(null)
}
function onKeyUp(e: KeyboardEvent) {
  if (e.key === 'Shift') shiftHeld.value = false
}

/** 滚轮缩放（光标为中心；e.target 可能是 img，故用容器 rect 换算光标坐标） */
function onWheel(e: WheelEvent) {
  const el = containerRef.value
  if (!el || natural.value[0] <= 0) return
  const rect = el.getBoundingClientRect()
  const cursor: Vec2 = [e.clientX - rect.left - PAD, e.clientY - rect.top - PAD]
  preview.zoomBy(e.deltaY < 0 ? 1 : -1, containerSize.value, natural.value, cursor)
}

// 左键拖拽平移；Shift+左键 = 框选识别（Phase 2 只画框不识别）
let dragging = false
let lastX = 0
let lastY = 0
/** 框选拖拽的原始起止点（容器坐标，用于 <8px 误触判定） */
let boxRawStart: Vec2 = [0, 0]
let boxRawCur: Vec2 = [0, 0]

function onPointerDown(e: PointerEvent) {
  if (e.button !== 0) return
  const el = e.currentTarget as HTMLElement
  if (e.shiftKey) {
    // 再次 Shift+拖拽：先清除旧待确认框，再开始新框选
    if (natural.value[0] <= 0) return
    preview.setPendingBox(null)
    const rect = el.getBoundingClientRect()
    boxRawStart = [e.clientX - rect.left, e.clientY - rect.top]
    boxRawCur = boxRawStart
    const c = clampToImg(boxRawStart)
    drawBox.value = { x1: c[0], y1: c[1], x2: c[0], y2: c[1] }
    el.setPointerCapture(e.pointerId)
    return
  }
  dragging = true
  lastX = e.clientX
  lastY = e.clientY
  el.setPointerCapture(e.pointerId)
}

function onPointerMove(e: PointerEvent) {
  const el = containerRef.value
  if (!el) return
  const rect = el.getBoundingClientRect()
  const p: Vec2 = [e.clientX - rect.left, e.clientY - rect.top]
  if (drawBox.value) {
    // 框选拖拽中：实时框跟随（钳制在图片范围内）
    boxRawCur = p
    const c = clampToImg(p)
    drawBox.value = { ...drawBox.value, x2: c[0], y2: c[1] }
    return
  }
  if (!dragging) return
  preview.panBy(e.clientX - lastX, e.clientY - lastY, disp.value, containerSize.value)
  lastX = e.clientX
  lastY = e.clientY
}

function onPointerUp(e: PointerEvent) {
  const el = e.currentTarget as HTMLElement
  el.releasePointerCapture(e.pointerId)
  const box = drawBox.value
  if (box) {
    // 框选结束：<8px 视为误触点击，不产生待确认框
    drawBox.value = null
    const [sx, sy] = boxRawStart
    const [cx, cy] = boxRawCur
    if (Math.hypot(cx - sx, cy - sy) < 8) return
    const a = toNorm(boxRawStart)
    const b = toNorm(boxRawCur)
    if (!a || !b) return
    preview.setPendingBox({
      x1: Math.min(a[0], b[0]),
      y1: Math.min(a[1], b[1]),
      x2: Math.max(a[0], b[0]),
      y2: Math.max(a[1], b[1]),
    })
    return
  }
  dragging = false
}

/** 工具条 ± 按钮：以容器中心为锚点缩放 */
function zoomStep(direction: 1 | -1) {
  const center: Vec2 = [containerSize.value[0] / 2, containerSize.value[1] / 2]
  preview.zoomBy(direction, containerSize.value, natural.value, center)
}

/**
 * 图片区右键菜单（预览变体，对齐 GPUI capture_menu(in_preview=true)）：
 * 首项返回网格 + 缩放组（以容器中心为锚点，同工具条 zoomStep）。
 */
function onImageContextMenu(e: MouseEvent) {
  const center: Vec2 = [containerSize.value[0] / 2, containerSize.value[1] / 2]
  contextMenu.openMenu(
    captureMenuItems({
      meta: selection.selected,
      inPreview: true,
      selectedCount: selection.selectedIndices.length,
      paths: selection.selectedPaths,
      onToggleView: () => preview.toggleView(),
      zoom: {
        in: () => preview.zoomBy(1, containerSize.value, natural.value, center),
        out: () => preview.zoomBy(-1, containerSize.value, natural.value, center),
        fit: () => preview.zoomFit(),
        actual: () => preview.zoomOneToOne(),
      },
    }),
    e.clientX,
    e.clientY,
  )
}
</script>

<template>
  <div class="relative flex h-full flex-col">
    <!-- 图片区：加载/渲染 + 检测框/框选叠加层，工具条浮动在其底部 -->
    <div
      ref="container"
      class="relative min-h-0 flex-1 touch-none overflow-hidden select-none"
      :class="drawBox || shiftHeld ? 'cursor-crosshair' : dragging ? 'cursor-grabbing' : 'cursor-grab'"
      @wheel.prevent="onWheel"
      @pointerdown="onPointerDown"
      @pointermove="onPointerMove"
      @pointerup="onPointerUp"
      @pointercancel="onPointerUp"
      @contextmenu.prevent="onImageContextMenu"
    >
      <!-- 加载中骨架 -->
      <Skeleton v-if="loading" class="absolute rounded-none" :style="{ inset: `${PAD}px` }" />

      <img
        v-if="current"
        :key="imgSrc"
        :src="imgSrc"
        :alt="displayName(current)"
        draggable="false"
        class="absolute top-0 left-0 max-w-none"
        :class="{ invisible: loading }"
        :style="{
          width: `${disp[0]}px`,
          height: `${disp[1]}px`,
          transform: `translate3d(${origin[0] + PAD}px, ${origin[1] + PAD}px, 0)`,
        }"
        @load="onImgLoad"
      />

      <!-- 检测框叠加（birdBbox 归一化 × 显示尺寸，随缩放平移跟随） -->
      <div
        v-if="bboxRect"
        class="pointer-events-none absolute border-2 border-primary bg-primary/20"
        :style="{
          left: `${bboxRect.left}px`,
          top: `${bboxRect.top}px`,
          width: `${bboxRect.width}px`,
          height: `${bboxRect.height}px`,
        }"
      />

      <!-- 框选实时框（Shift+拖拽中，accent 色跟随） -->
      <div
        v-if="drawRect"
        class="pointer-events-none absolute border-2 border-primary bg-primary/10"
        :style="{
          left: `${drawRect.left}px`,
          top: `${drawRect.top}px`,
          width: `${drawRect.width}px`,
          height: `${drawRect.height}px`,
        }"
      />

      <!-- 待确认框（框选完成后的虚线框，Esc 清除） -->
      <div
        v-if="pendingRect"
        class="pointer-events-none absolute border-2 border-dashed border-primary"
        :style="{
          left: `${pendingRect.left}px`,
          top: `${pendingRect.top}px`,
          width: `${pendingRect.width}px`,
          height: `${pendingRect.height}px`,
        }"
      />

      <!-- 工具条（stop 冒泡：不触发拖拽/框选/缩放） -->
      <div
        class="absolute bottom-3 left-1/2 flex -translate-x-1/2 items-center gap-1 rounded-md border bg-card/90 p-1 backdrop-blur"
        @pointerdown.stop
        @wheel.stop
      >
        <Button size="sm" variant="ghost" title="缩小" @click="zoomStep(-1)">
          <MinusIcon />
        </Button>
        <span class="w-12 text-center text-xs text-muted-foreground font-mono-num">
          {{ preview.zoomPercent() }}%
        </span>
        <Button size="sm" variant="ghost" title="放大" @click="zoomStep(1)">
          <PlusIcon />
        </Button>
        <Button size="sm" variant="ghost" title="适应窗口" @click="preview.zoomFit()">
          <MaximizeIcon />
          适应
        </Button>
        <Button
          size="sm"
          :variant="preview.isOneToOne ? 'secondary' : 'ghost'"
          title="实际像素"
          @click="preview.zoomOneToOne()"
        >
          <ScanIcon />
          1:1
        </Button>
        <Button
          size="sm"
          :variant="preview.bboxVisible ? 'secondary' : 'ghost'"
          title="检测框（V）"
          @click="preview.toggleBbox()"
        >
          <ScanLineIcon />
          检测框
        </Button>
        <Button size="sm" variant="ghost" title="返回网格（Esc/G）" @click="preview.closePreview()">
          <Grid2x2Icon />
          网格
        </Button>
      </div>
    </div>

    <!-- 底部胶片条（全部拍摄缩略图，点击跳转选中） -->
    <Filmstrip />
  </div>
</template>
