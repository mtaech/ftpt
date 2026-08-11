// 连拍分组纯逻辑（T0 批次）：按显示顺序的 CaptureMeta 数组，将相邻两项拍摄时间
// （EXIF dateTaken）差 ≤ gapMs 的归为同一连拍组。纯前端实现，无后端改动。
// 组信息以「输入数组下标 → { groupId, size, pos }」映射返回，只登记 size≥2 的组；
// 调用方按需映射到 captures.items 下标（如过滤+排序后的显示序，见 filter store）。
import type { CaptureMeta } from './bindings'

/** 连拍组成员信息：groupId 组标识、size 组总张数、pos 组内序号（0 起，按显示序） */
export interface BurstEntry {
  groupId: string
  size: number
  pos: number
}

/** 组信息映射：key = 输入数组下标（显示序位置）；仅含 size≥2 的组 */
export type BurstGroupMap = Map<number, BurstEntry>

/**
 * EXIF 拍摄时间 → 毫秒时间戳（UTC）。
 * 兼容两种形态：EXIF「2026:06:28 10:15:30」与 mock/ISO「2026-08-01T10:15:30」；
 * 字段间分隔符不限定（年份 4 位 + 月/日/时/分/秒各 2 位）。
 * 用 Date.UTC 构造：连拍分组只关心时间差，时区在差值中相消，不受本地时区影响。
 * 解析失败/字段越界（如 13 月）返回 null。
 * （filter.ts 的 DATE_TAKEN_RE 只做 YYYY-MM-DD 日期串提取，语义不同，不复用。）
 */
export function parseExifDate(s: string | null): number | null {
  if (!s) return null
  const m = /^(\d{4})\D(\d{2})\D(\d{2})\D(\d{2})\D(\d{2})\D(\d{2})$/.exec(s.trim())
  if (!m) return null
  const y = Number(m[1])
  const mo = Number(m[2])
  const d = Number(m[3])
  const h = Number(m[4])
  const mi = Number(m[5])
  const se = Number(m[6])
  // Date.UTC 对越界字段会静默进位（如 13 月 → 次年 1 月），需先拦截，避免错误分组
  if (mo < 1 || mo > 12 || d < 1 || d > 31 || h > 23 || mi > 59 || se > 59) return null
  return Date.UTC(y, mo - 1, d, h, mi, se)
}

/**
 * 连拍分组：按传入顺序（调用方传显示顺序）遍历，相邻两项 dateTaken 时间差
 * ≤ gapMs 归入同组；dateTaken 为 null 或解析失败 = 独立单张（不登记，且切断组链）。
 * 返回 Map<下标, BurstEntry>，只包含 size≥2 的组（单张不成组、空数组返回空 Map）。
 */
export function computeBurstGroups(items: CaptureMeta[], gapMs = 2000): BurstGroupMap {
  // 第一遍：切分组边界（start/end 半开区间，按下标）
  const bounds: { start: number; end: number }[] = []
  let start = -1
  let prev: number | null = null
  for (let i = 0; i < items.length; i++) {
    const t = parseExifDate(items[i].dateTaken)
    if (t === null) {
      // null/解析失败：结算当前组并切断链（前后两段即使时间接近也不合并）
      if (start >= 0) bounds.push({ start, end: i })
      start = -1
      prev = null
      continue
    }
    if (start < 0) {
      start = i
    } else if (prev !== null && Math.abs(t - prev) <= gapMs) {
      // 与上一项时间差在阈值内 → 延续当前组
    } else {
      bounds.push({ start, end: i })
      start = i
    }
    prev = t
  }
  if (start >= 0) bounds.push({ start, end: items.length })

  // 第二遍：只登记 size≥2 的组（groupId 按组序编号，size/pos 为组结算后的终值）
  const map: BurstGroupMap = new Map()
  bounds.forEach((b, gi) => {
    const size = b.end - b.start
    if (size < 2) return
    const groupId = `burst-${gi}`
    for (let i = b.start; i < b.end; i++) {
      map.set(i, { groupId, size, pos: i - b.start })
    }
  })
  return map
}
