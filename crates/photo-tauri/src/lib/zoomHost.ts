// 缩放键宿主注册：`=`/`-` 键的缩放动作需要当前视图的容器/原图尺寸，而这些是
// 视图组件（PhotoPreview / CompareView）的局部状态。拥有它们的组件在挂载时
// 注册宿主函数，App.vue 键位层分发时调用；网格/幻灯片视图无宿主 → 键位不生效。
export interface ZoomHost {
  zoomIn(): void
  zoomOut(): void
}

let host: ZoomHost | null = null

/** 注册/注销缩放宿主（组件 onMounted/onUnmounted 调用；单槽，互斥视图安全） */
export function registerZoomHost(h: ZoomHost | null): void {
  host = h
}

/** 当前缩放宿主（无宿主返回 null，调用方 no-op） */
export function zoomHost(): ZoomHost | null {
  return host
}
