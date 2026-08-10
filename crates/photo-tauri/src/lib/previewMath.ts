// 预览缩放/平移纯函数：自 crates/photo-tool-app/src/state/preview_math.rs 逐行移植，
// 与 GPUI 渲染公式严格一致，改动需双向同步。

export type Vec2 = [number, number]

/** 预览居中偏移：图片 ≤ 容器时居中（≥0），> 容器时为负（图片向左上溢出） */
export function previewCenterOffset(disp: number, container: number): number {
  return (container - disp) / 2
}

/** 单轴平移钳制：图片 ≤ 容器时不允许平移；> 容器时边缘不可进入视口 */
export function clampPanAxis(disp: number, container: number, pan: number): number {
  if (disp <= container) return 0
  const center = previewCenterOffset(disp, container)
  return Math.min(Math.max(pan, container - disp - center), -center)
}

/**
 * 光标中心缩放：保持光标下的图像点不动，返回新 pan。
 * oldDisp/newDisp 为缩放前后显示尺寸，container 为容器尺寸，cursor 为容器坐标。
 */
export function panAfterCursorZoom(
  oldDisp: Vec2,
  newDisp: Vec2,
  container: Vec2,
  pan: Vec2,
  cursor: Vec2,
): Vec2 {
  const axis = (oldD: number, newD: number, c: number, p: number, cur: number) => {
    const oldOrigin = previewCenterOffset(oldD, c) + p
    const r = oldD > 0 ? newD / oldD : 1
    const newOrigin = cur - (cur - oldOrigin) * r
    return newOrigin - previewCenterOffset(newD, c)
  }
  return [
    axis(oldDisp[0], newDisp[0], container[0], pan[0], cursor[0]),
    axis(oldDisp[1], newDisp[1], container[1], pan[1], cursor[1]),
  ]
}

/**
 * 适应缩放系数（fit）：图片完整放入容器，小图不放大（上限 1.0）。
 * 与 Rust window_pos_to_image_norm 的 scale 计算一致。
 */
export function fitScale(
  containerW: number,
  containerH: number,
  imgW: number,
  imgH: number,
): number {
  if (imgW <= 0 || imgH <= 0) return 1
  return Math.min(containerW / imgW, containerH / imgH, 1)
}
