// 地图视图状态：M 键开关全屏 GPS 地图 overlay（MapView.vue）。
// 纯本地 UI 开关态——GPS 数据直接读 captures store 的 gpsLat/gpsLon（CaptureMeta
// 十进制坐标，Rust 侧 enrich_with_exif 已回填），无事件订阅，故无 init() 防重复
// listen 需求（对照 captures store 的事件接线模式，本 store 不消费任何事件）。
import { defineStore } from 'pinia'

export const useMapViewStore = defineStore('mapView', {
  state: () => ({
    /** 地图 overlay 是否打开（M 键切换 / 顶部关闭按钮 / Esc 关闭） */
    isOpen: false,
  }),
  actions: {
    open() {
      this.isOpen = true
    },
    close() {
      this.isOpen = false
    },
    toggle() {
      this.isOpen = !this.isOpen
    },
  },
})
