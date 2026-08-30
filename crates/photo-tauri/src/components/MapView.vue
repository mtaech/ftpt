<script setup lang="ts">
// 全屏 GPS 地图 overlay（M 键开关，参照 SettingsModal 的 Dialog overlay z-50 层级）：
// 带 GPS 坐标的照片上图（circleMarker，accent 色 = --primary），点击 marker 弹 popup
// 显示缩略图 + 文件名 + 「定位到网格」按钮；无 GPS 照片数量在头部角落提示。
// 数据源 = captures store 的 gpsLat/gpsLon（CaptureMeta 十进制坐标，Rust 侧
// enrich_with_exif 已回填，无任何 Rust 改动）。地图生命周期 onBeforeUnmount 销毁防泄漏。
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import * as L from 'leaflet'
import 'leaflet/dist/leaflet.css'
import { MapPinIcon, XIcon } from '@lucide/vue'
import { useCapturesStore } from '@/stores/captures'
import { useMapViewStore } from '@/stores/mapView'
import { ptimgUrl } from '@/lib/ipc'
import type { CaptureMeta } from '@/lib/bindings'

/** 「定位到网格」请求：交 App.vue 回网格视图 + selection 选中（复用现有 showGrid/select 路径） */
const emit = defineEmits<{ locate: [item: CaptureMeta] }>()

const captures = useCapturesStore()
const mapView = useMapViewStore()

const mapEl = ref<HTMLElement | null>(null)
let map: L.Map | null = null
/** 下标 → marker（syncMarkers 全量重建时清理） */
let markers = new Map<number, L.CircleMarker>()

/** 带 GPS 的照片（captures.items 下标 + 引用；gpsLat/gpsLon 均非空） */
const gpsItems = computed(() => {
  const out: Array<{ index: number; item: CaptureMeta }> = []
  captures.items.forEach((item, i) => {
    if (item.gpsLat !== null && item.gpsLon !== null) out.push({ index: i, item })
  })
  return out
})
/** 无 GPS 照片数（角落提示） */
const noGpsCount = computed(() => captures.items.length - gpsItems.value.length)

/** popup 内容 HTML 转义（文件名可能含引号/尖括号） */
function escapeHtml(s: string): string {
  return s.replace(/[&<>"']/g, (ch) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[ch] ?? ch)
}

/** popup 内容：缩略图 + 文件名 + 「定位到网格」按钮（动态 DOM，用全局类而非 scoped） */
function buildPopup(item: CaptureMeta): HTMLElement {
  const wrap = document.createElement('div')
  wrap.className = 'flex w-44 flex-col gap-1.5'
  wrap.innerHTML = `
    <img class="h-24 w-full rounded object-cover" src="${ptimgUrl('thumb', item.primaryPath)}" alt="" />
    <div class="truncate text-xs font-medium text-popover-foreground" title="${escapeHtml(item.baseName)}">${escapeHtml(item.baseName)}</div>
    <button type="button" class="mt-0.5 rounded bg-primary px-2 py-1 text-xs font-medium text-primary-foreground hover:opacity-90">定位到网格</button>
  `
  wrap.querySelector('button')?.addEventListener('click', () => emit('locate', item))
  return wrap
}

/** 全量重建 marker（初始挂载 / captures.items 变更时） */
function syncMarkers() {
  if (!map) return
  markers.forEach((m) => m.remove())
  markers.clear()
  if (gpsItems.value.length === 0) {
    // 无坐标照片：默认世界视图，留角落提示
    map.setView([35, 105], 3)
    return
  }
  const pts: L.LatLngExpression[] = []
  for (const { index, item } of gpsItems.value) {
    const ll: L.LatLngExpression = [item.gpsLat!, item.gpsLon!]
    pts.push(ll)
    const m = L.circleMarker(ll, {
      radius: 6,
      color: '#ffffff',
      weight: 1,
      fillColor: 'var(--primary)',
      fillOpacity: 1,
    })
    m.bindPopup(() => buildPopup(item), { className: 'map-popup' })
    m.addTo(map)
    markers.set(index, m)
  }
  // 单点视图拉到合适缩放（fitBounds 对单点会顶到 maxZoom），多点自适应取景
  if (pts.length === 1) map.setView(pts[0], 6)
  else map.fitBounds(L.latLngBounds(pts), { padding: [48, 48] })
}

onMounted(() => {
  if (!mapEl.value) return
  // 显式初始视图（默认 zoom-0 瞬态，syncMarkers 会立即按数据修正）
  map = L.map(mapEl.value, { center: [35, 105], zoom: 3 })
  L.tileLayer('https://tile.openstreetmap.org/{z}/{x}/{y}.png', {
    maxZoom: 19,
    attribution: '&copy; <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a> contributors',
  }).addTo(map)
  syncMarkers()
  // overlay 挂载后容器尺寸可能尚未稳定，下一帧校准一次（防瓦片/居中错位）
  setTimeout(() => map?.invalidateSize(), 0)
})

onBeforeUnmount(() => {
  map?.remove()
  map = null
  markers.clear()
})

// 扫描重载（scan:done 后 captures.items 替换）时同步 marker 集
watch(gpsItems, () => syncMarkers())
</script>

<template>
  <div class="fixed inset-0 z-50 flex flex-col bg-background text-foreground">
    <!-- 头部：标题 + 无 GPS 计数（角落提示）+ 关闭 -->
    <header class="flex h-11 shrink-0 items-center gap-2 border-b bg-card px-3">
      <MapPinIcon class="size-4 text-primary" />
      <span class="text-sm font-medium">照片地图</span>
      <span class="text-xs text-muted-foreground">（{{ gpsItems.length }} 张有 GPS / {{ noGpsCount }} 张无 GPS）</span>
      <div class="min-w-0 flex-1" />
      <button
        type="button"
        class="inline-flex size-8 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground"
        aria-label="关闭地图"
        @click="mapView.close()"
      >
        <XIcon class="size-4" />
      </button>
    </header>
    <!-- 地图容器：OSM 瓦片（attribution 已配置） -->
    <div ref="mapEl" class="min-h-0 flex-1" />
    <!-- 空态居中提示（仅全部无坐标时） -->
    <div
      v-if="gpsItems.length === 0"
      class="pointer-events-none absolute inset-x-0 bottom-8 flex justify-center"
    >
      <div class="rounded-md border border-border bg-card px-3 py-1.5 text-sm text-muted-foreground shadow-lg">
        当前目录没有带 GPS 坐标的照片
      </div>
    </div>
  </div>
</template>

<style>
/* 地图 popup 内容：收窄 leaflet 默认内边距（动态 DOM 不受 scoped 约束，用 popup className 限定） */
.map-popup .leaflet-popup-content {
  margin: 10px;
}
.map-popup .leaflet-popup-content-wrapper {
  border-radius: 8px;
  padding: 0;
}
/* 地图控件字体跟随应用主题（leaflet 默认 Roboto 系列） */
.leaflet-container {
  font: inherit;
}
</style>
