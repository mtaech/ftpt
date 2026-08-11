// 对比模式状态（T0 批次）：2–4 张多窗格并排，缩放/平移全窗格同步。
// 同步语义：缩放系数相对各图自身 fit（zoom=1 → 适应，×1.25 步进，沿用 preview store
// 语义）；平移以「占显示尺寸比例」归一化存储——不同图片按各自 fit × 自然尺寸 × 同一
// zoom 得到各自显示尺寸，再乘同一归一化 pan 得各自像素平移，比例天然同步。
// 数学（fitScale/panAfterCursorZoom/clampPanAxis）复用 previewMath，与 GPUI 一致。
import { defineStore } from 'pinia'
import type { CaptureMeta } from '@/lib/bindings'
import type { Vec2 } from '@/lib/previewMath'
import { clampPanAxis, fitScale, panAfterCursorZoom } from '@/lib/previewMath'
import { useCapturesStore } from './captures'
import { ZOOM_STEP } from './preview'

export const useCompareStore = defineStore('compare', {
  state: () => ({
    /** 对比集（captures.items 下标，按对比顺序；2–4 张） */
    indices: [] as number[],
    /** 聚焦格（indices 内下标；评分/色标/旗标/Delete 作用于它） */
    focus: 0,
    /** 缩放系数（相对各图自身 fit；1 = 适应） */
    zoom: 1,
    /** 平移（归一化：像素平移 ÷ 对应图显示尺寸；各图共用同一比例） */
    pan: [0, 0] as Vec2,
  }),
  getters: {
    count: (s) => s.indices.length,
    /** 对比模式是否在途（≥2 张才生效；视图路由另看 preview store 的 isCompare） */
    active: (s) => s.indices.length >= 2,
    /** 聚焦格对应的拍摄（越界安全） */
    focusedItem(): CaptureMeta | null {
      return useCapturesStore().items[this.indices[this.focus]] ?? null
    },
    /** 聚焦格路径（标记键的目标） */
    focusedPath(): string | null {
      return this.focusedItem?.primaryPath ?? null
    },
  },
  actions: {
    /** 进入对比：indices 为 captures.items 下标（2–4 张），复位缩放平移与聚焦 */
    open(indices: number[]) {
      this.indices = indices.slice(0, 4)
      this.focus = 0
      this.zoom = 1
      this.pan = [0, 0]
    },
    /** 退出对比：清空对比集并复位 */
    close() {
      this.indices = []
      this.focus = 0
      this.zoom = 1
      this.pan = [0, 0]
    },
    /** 聚焦某格（越界钳制） */
    setFocus(i: number) {
      if (this.indices.length === 0) return
      this.focus = Math.min(Math.max(i, 0), this.indices.length - 1)
    },
    /** 某格显示尺寸（像素）：fit × 自然尺寸 × zoom；natural 未加载时返回 [0,0] */
    paneDisp(natural: Vec2, container: Vec2): Vec2 {
      if (natural[0] <= 0 || natural[1] <= 0) return [0, 0]
      const s = fitScale(container[0], container[1], natural[0], natural[1]) * this.zoom
      return [natural[0] * s, natural[1] * s]
    },
    /** 某格图片左上角在容器内的坐标（居中偏移 + 该格像素平移） */
    paneOrigin(disp: Vec2, container: Vec2): Vec2 {
      const px = this.clampedPan(disp, container)
      return [
        (container[0] - disp[0]) / 2 + px[0],
        (container[1] - disp[1]) / 2 + px[1],
      ]
    },
    /** 归一化 pan → 该格像素 pan（并钳制到该格合法范围，图片 ≤ 容器时归零） */
    clampedPan(disp: Vec2, container: Vec2): Vec2 {
      return [
        clampPanAxis(disp[0], container[0], this.pan[0] * disp[0]),
        clampPanAxis(disp[1], container[1], this.pan[1] * disp[1]),
      ]
    },
    /** 滚轮缩放（以光标为中心，×1.25 步进）：共享 zoom + 归一化 pan → 全部窗格同步 */
    zoomBy(direction: 1 | -1, container: Vec2, natural: Vec2, cursor: Vec2) {
      const oldDisp = this.paneDisp(natural, container)
      if (oldDisp[0] <= 0 || oldDisp[1] <= 0) return
      this.zoom = direction > 0 ? this.zoom * ZOOM_STEP : this.zoom / ZOOM_STEP
      const newDisp = this.paneDisp(natural, container)
      // 光标中心缩放：像素 pan 走 panAfterCursorZoom，再归一化回共享比例
      const oldPanPx: Vec2 = [this.pan[0] * oldDisp[0], this.pan[1] * oldDisp[1]]
      const newPanPx = panAfterCursorZoom(oldDisp, newDisp, container, oldPanPx, cursor)
      this.pan = [newPanPx[0] / newDisp[0], newPanPx[1] / newDisp[1]]
      this.pan = [
        clampPanAxis(newDisp[0], container[0], this.pan[0] * newDisp[0]) / newDisp[0],
        clampPanAxis(newDisp[1], container[1], this.pan[1] * newDisp[1]) / newDisp[1],
      ]
    },
    /** 拖拽平移（增量）：当前窗格的像素增量 → 归一化增量，作用于全部窗格 */
    panBy(dx: number, dy: number, disp: Vec2, container: Vec2) {
      if (disp[0] <= 0 || disp[1] <= 0) return
      this.pan = [this.pan[0] + dx / disp[0], this.pan[1] + dy / disp[1]]
      this.pan = [
        clampPanAxis(disp[0], container[0], this.pan[0] * disp[0]) / disp[0],
        clampPanAxis(disp[1], container[1], this.pan[1] * disp[1]) / disp[1],
      ]
    },
    /** 适应（复位缩放平移；双击窗格触发） */
    zoomFit() {
      this.zoom = 1
      this.pan = [0, 0]
    },
  },
})
