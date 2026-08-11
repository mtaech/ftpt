// 预览态：视图切换 + 缩放/平移。缩放语义对齐 GPUI：
// zoom = 1 → 适应窗口（fit）；zoom = 0 → 1:1（显示尺寸 = 原图像素，此时图源切 ptimg full）。
// 视图三态：grid / preview / compare（对比模式多窗格，缩放平移状态在 compare store，
// 这里只做视图路由，见 openCompare/closeCompare）。
import { defineStore } from 'pinia'
import type { BBox } from '@/lib/bindings'
import type { Vec2 } from '@/lib/previewMath'
import { clampPanAxis, fitScale, panAfterCursorZoom, previewCenterOffset } from '@/lib/previewMath'
import { useFilterStore } from './filter'
import { useSelectionStore } from './selection'

/** 滚轮缩放步进（对齐 GPUI ×1.25/÷1.25） */
export const ZOOM_STEP = 1.25

export const usePreviewStore = defineStore('preview', {
  state: () => ({
    /** 当前视图：网格 / 单图预览 / 对比 / 幻灯片 / 统计 */
    view: 'grid' as 'grid' | 'preview' | 'compare' | 'slideshow' | 'stats',
    /** 进入对比前的视图（Esc/G 退出对比时还原；对比只能从 grid/preview 进入） */
    viewBeforeCompare: 'grid' as 'grid' | 'preview',
    /** 进入幻灯片前的视图（Esc/G 退出时还原；幻灯片只能从 grid/preview 进入） */
    viewBeforeSlideshow: 'grid' as 'grid' | 'preview',
    /** 幻灯片当前张在筛选序中的位置（filteredIndices 下标；←/→ 与自动播放共用） */
    slideshowIndex: 0,
    /** 幻灯片自动播放中（空格暂停/继续） */
    slideshowPlaying: true,
    /** 缩放系数（相对 fit）；0 = 1:1 */
    zoom: 1,
    /** 平移量（相对居中位的像素偏移） */
    pan: [0, 0] as Vec2,
    /** 检测框是否可见（默认关，对齐 GPUI；V 键 / 工具条「检测框」切换） */
    bboxVisible: false,
    /** 剪切警告叠加是否可见（默认关；'o' 键切换：红 = 高光溢出、蓝 = 死黑） */
    clipOverlayVisible: false,
    /**
     * 框选识别待确认区域（归一化 0–1，Phase 2 只画框不识别）。
     * 由 Shift+拖拽产生，Esc 或再次 Shift+拖拽清除；切图/退出预览一并清空。
     */
    pendingBox: null as BBox | null,
  }),
  getters: {
    isPreview: (s) => s.view === 'preview',
    isCompare: (s) => s.view === 'compare',
    isSlideshow: (s) => s.view === 'slideshow',
    isStats: (s) => s.view === 'stats',
    isGrid: (s) => s.view === 'grid',
    isOneToOne: (s) => s.zoom === 0,
  },
  actions: {
    openPreview() {
      this.view = 'preview'
      this.resetView()
    },
    closePreview() {
      this.view = 'grid'
      // 待确认框只在预览内有效，退出预览一并清空
      this.pendingBox = null
      this.clipOverlayVisible = false
    },
    toggleView() {
      if (this.isPreview) this.closePreview()
      else this.openPreview()
    },
    /** 进入对比：记住来源视图（Esc/G 退出时还原），视图切到 compare */
    openCompare() {
      this.viewBeforeCompare = this.isPreview ? 'preview' : 'grid'
      this.view = 'compare'
    },
    /** 退出对比：回到进入前的视图（grid 或 preview），清空待确认框 */
    closeCompare() {
      this.view = this.viewBeforeCompare
      this.viewBeforeCompare = 'grid'
      this.pendingBox = null
    },
    /** 进入幻灯片：记住来源视图（Esc/G 退出时还原），从当前选中张在筛选序中的位置开始 */
    openSlideshow() {
      this.viewBeforeSlideshow = this.isPreview ? 'preview' : 'grid'
      const order = useFilterStore().filteredIndices
      const sel = useSelectionStore().selectedIndex
      const pos = sel === null ? 0 : order.indexOf(sel)
      this.slideshowIndex = pos < 0 ? 0 : pos
      this.slideshowPlaying = true
      this.view = 'slideshow'
    },
    /** 退出幻灯片：回到进入前的视图（grid 或 preview），清空待确认框 */
    closeSlideshow() {
      this.view = this.viewBeforeSlideshow
      this.viewBeforeSlideshow = 'grid'
      this.pendingBox = null
    },
    /** 进入统计视图（SpeciesIndex）：主区视图路由，Esc/G/图标退出回 grid（对齐 compare 语义） */
    openStats() {
      this.view = 'stats'
    },
    /** 退出统计视图：回到网格，清空待确认框 */
    closeStats() {
      this.view = 'grid'
      this.pendingBox = null
    },
    /** 幻灯片切张（±1 循环；←/→ 手动切换与自动播放共用，组件据此重置计时） */
    slideshowStep(delta: 1 | -1) {
      const n = useFilterStore().filteredIndices.length
      if (n === 0) return
      this.slideshowIndex = (this.slideshowIndex + delta + n) % n
    },
    /** 暂停/继续自动播放（空格） */
    toggleSlideshowPlay() {
      this.slideshowPlaying = !this.slideshowPlaying
    },
    /** 切图/进预览时复位缩放平移 */
    resetView() {
      this.zoom = 1
      this.pan = [0, 0]
    },

    /**
     * 显示尺寸（像素）：zoom=0 → 原图 1:1；否则 fit × zoom。
     * container 为扣除内边距后的可用区。
     */
    displaySize(container: Vec2, natural: Vec2): Vec2 {
      if (natural[0] <= 0 || natural[1] <= 0) return [0, 0]
      if (this.zoom === 0) return [natural[0], natural[1]]
      const s = fitScale(container[0], container[1], natural[0], natural[1]) * this.zoom
      return [natural[0] * s, natural[1] * s]
    },

    /** 图片左上角在容器内的坐标（渲染定位用） */
    imageOrigin(disp: Vec2, container: Vec2): Vec2 {
      return [
        previewCenterOffset(disp[0], container[0]) + this.pan[0],
        previewCenterOffset(disp[1], container[1]) + this.pan[1],
      ]
    },

    /** 钳制当前 pan 到合法范围 */
    clampPan(disp: Vec2, container: Vec2) {
      this.pan = [
        clampPanAxis(disp[0], container[0], this.pan[0]),
        clampPanAxis(disp[1], container[1], this.pan[1]),
      ]
    },

    /**
     * 滚轮缩放（以光标为中心）：direction=+1 放大 / -1 缩小。
     * 1:1 态起步时以「原图/fit」为当前倍率继续步进。
     */
    zoomBy(direction: 1 | -1, container: Vec2, natural: Vec2, cursor: Vec2) {
      const oldDisp = this.displaySize(container, natural)
      if (oldDisp[0] <= 0) return
      const fit = fitScale(container[0], container[1], natural[0], natural[1])
      if (fit <= 0) return
      const curRatio = this.zoom === 0 ? 1 / fit : this.zoom
      const next = direction > 0 ? curRatio * ZOOM_STEP : curRatio / ZOOM_STEP
      // 越过 1:1 点不吸附，直接以倍率表达（zoom=0 仅由 1:1 按钮进入）
      this.zoom = next
      const newDisp = this.displaySize(container, natural)
      this.pan = panAfterCursorZoom(oldDisp, newDisp, container, this.pan, cursor)
      this.clampPan(newDisp, container)
    },

    /** 拖拽平移（增量） */
    panBy(dx: number, dy: number, disp: Vec2, container: Vec2) {
      this.pan = [this.pan[0] + dx, this.pan[1] + dy]
      this.clampPan(disp, container)
    },

    /** 适应窗口 */
    zoomFit() {
      this.zoom = 1
      this.pan = [0, 0]
    },

    /** 1:1（显示尺寸 = 原图像素；组件据此把图源切到 ptimg full） */
    zoomOneToOne() {
      this.zoom = 0
      this.pan = [0, 0]
    },

    /** 当前缩放百分比（1:1 = 100%） */
    zoomPercent(): number {
      return this.zoom === 0 ? 100 : Math.round(this.zoom * 100)
    },

    /** 切换检测框叠加显示（V 键 / 工具条「检测框」） */
    toggleBbox() {
      this.bboxVisible = !this.bboxVisible
    },

    /** 切换剪切警告叠加（'o' 键；红 = 高光溢出、蓝 = 死黑） */
    toggleClipOverlay() {
      this.clipOverlayVisible = !this.clipOverlayVisible
    },

    /** 设置/清除框选待确认区域（归一化 0–1；null = 清除） */
    setPendingBox(box: BBox | null) {
      this.pendingBox = box
    },
  },
})
