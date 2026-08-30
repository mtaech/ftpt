// 连拍组自动选优纯逻辑（T2 批次）：组内按眼锐度选最优帧，供 K 键一键标 Reject
// 与网格最优帧徽标复用。纯前端实现，无后端改动；确定性全序保证相同输入
// 永远得到相同结果（并列全部打破，不依赖输入顺序）。
import type { CaptureMeta } from './bindings'

/** 眼锐度分值：null（无识别记录/无眼/评分失败）视为最低（-1），与 filter.ts 排序语义一致 */
function sharpnessOf(m: CaptureMeta): number {
  return m.eyeSharpness ?? -1
}

/** 文件大小分值：null（无大小信息）视为 0，排在有大小的帧之后（并列取「文件更大者」） */
function sizeOf(m: CaptureMeta): number {
  return m.fileSize ?? 0
}

/**
 * 两帧比较（选优全序）：返回 <0 表示 a 优于 b。
 * 1. eye_sharpness 降序（None 垫底）；
 * 2. 并列取 fileSize 更大者；
 * 3. 再并列取 primaryPath 字典序小者（确定性收尾，杜绝依赖输入顺序）。
 */
function compareBest(a: CaptureMeta, b: CaptureMeta): number {
  const dSharp = sharpnessOf(b) - sharpnessOf(a)
  if (dSharp !== 0) return dSharp
  const dSize = sizeOf(b) - sizeOf(a)
  if (dSize !== 0) return dSize
  return a.primaryPath < b.primaryPath ? -1 : a.primaryPath > b.primaryPath ? 1 : 0
}

/**
 * 连拍组内选最优帧（纯函数、确定性）：按 eye_sharpness 降序（None 视为最低），
 * 并列取 fileSize 更大者，再并列取 primaryPath 字典序小者。
 * 空组/单成员组返回 null（选优仅在 size≥2 的连拍组内有意义）。
 */
export function pickBestFrame(members: CaptureMeta[]): string | null {
  if (members.length < 2) return null
  let best = members[0]
  for (let i = 1; i < members.length; i++) {
    if (compareBest(members[i], best) < 0) best = members[i]
  }
  return best.primaryPath
}

/**
 * 组内非最优帧路径列表（保持原组序；供 K 键批量标 Reject）。
 * 空组/单成员组返回空数组（无「非最优」概念）。
 */
export function nonBestPaths(group: CaptureMeta[]): string[] {
  if (group.length < 2) return []
  const best = pickBestFrame(group)
  if (best === null) return []
  return group.filter((m) => m.primaryPath !== best).map((m) => m.primaryPath)
}
