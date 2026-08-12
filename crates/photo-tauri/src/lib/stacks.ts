// 显示堆叠纯逻辑：将筛选+排序后的成员下标（filteredIndices）分组为网格显示项。
// 扫描模型不变（每文件一个 Capture，见 CONTEXT.md「画面」），堆叠是前端显示层分组。
// 两种堆叠模式（photo-config StackMode，配置于设置弹窗）：
// - ByFileName：同 stem（baseName）文件合并——JPG/NEF 同画面，key = baseName
// - ByTime：同组照片堆叠——拍摄时间差 ≤2s 的连拍合并（与 burst.ts 连拍阈值一致），
//   key = `t-<组内最早时间戳>`（比组序号稳定）
// - None：不堆叠，每成员独立成组（网格渲染路径统一，单成员组无堆叠 UI）
// 组语义：组位置 = 组内成员在显示序中的最小位置；激活成员默认 = 主格式（JPEG 优先，
// 出图格式优先于 RAW/其他）；用户手动切换后由 filter store 的 stackActive 覆盖。
import type { CaptureMeta } from './bindings'
import { parseExifDate } from './burst'

/** 堆叠组：网格中的一个显示项，成员 = 同组的 items 下标 */
export interface StackGroup {
  /** 分组键：ByFileName = baseName；ByTime = `t-<最早时间戳>`；None = `i-<下标>` */
  key: string
  /** 成员下标（captures.items 下标，按分组算法排序；size≥1） */
  members: number[]
  /** 激活成员下标（默认主格式；用户切换后由 filter store 的 stackActive 覆盖） */
  active: number
}

/** 主格式优先级：JPEG 出图优先；未命中列表（RAW/其他）rank=Infinity 排最后 */
const PRIMARY_ORDER = [
  'jpg',
  'jpeg',
  'png',
  'tif',
  'tiff',
  'gif',
  'webp',
  'bmp',
  'heic',
  'heif',
  'avif',
]

/** 同组照片堆叠的时间窗（ms）：连拍判定阈值，与 burst.ts 连拍分组一致（≤2s） */
export const STACK_TIME_GAP_MS = 2000

function extOf(path: string): string {
  const m = /\.([^.\\/]+)$/.exec(path)
  return m ? m[1].toLowerCase() : ''
}

/**
 * 选主成员：按主路径**真实扩展名**（非 primaryFormat 规范化名，避免 jpeg→.jpg
 * 假后缀，对齐 CaptureMeta::display_name 的取法）在 PRIMARY_ORDER 中取最优；
 * 全部未命中（如纯 RAW 堆叠）回退组内首个成员。
 */
function pickPrimary(members: number[], items: CaptureMeta[]): number {
  let best = members[0]
  let bestRank = Number.POSITIVE_INFINITY
  for (const i of members) {
    const rank = PRIMARY_ORDER.indexOf(extOf(items[i]?.primaryPath ?? ''))
    if (rank >= 0 && rank < bestRank) {
      bestRank = rank
      best = i
    }
  }
  return best
}

/** 组装 StackGroup 并记录组位置（组内成员在显示序中的最小位置，用于排序输出） */
function buildGroup(
  key: string,
  members: number[],
  items: CaptureMeta[],
  posOf: Map<number, number>,
): { g: StackGroup; pos: number } {
  let pos = Number.POSITIVE_INFINITY
  for (const i of members) {
    const p = posOf.get(i)
    if (p !== undefined && p < pos) pos = p
  }
  return { g: { key, members, active: pickPrimary(members, items) }, pos }
}

/**
 * 不堆叠：每成员独立成组（网格渲染路径与堆叠模式统一，单成员组无堆叠 UI）。
 * 空输入返回空数组。
 */
export function groupSingles(indices: number[]): StackGroup[] {
  return indices.map((i) => ({ key: `i-${i}`, members: [i], active: i }))
}

/**
 * 同文件名堆叠：显示序下标按 baseName 分组（组位置 = 组内首个成员位置，
 * 同 stem 因筛选/排序被打散时聚合到最先出现的位置）。
 * 单成员组也返回；无 baseName 的项跳过（防御，正常扫描不产生）。
 */
export function groupStacks(indices: number[], items: CaptureMeta[]): StackGroup[] {
  const byStem = new Map<string, number[]>()
  const stems: string[] = []
  for (const i of indices) {
    const stem = items[i]?.baseName ?? ''
    if (stem === '') continue
    const list = byStem.get(stem)
    if (list) {
      list.push(i)
    } else {
      byStem.set(stem, [i])
      stems.push(stem)
    }
  }
  const posOf = new Map<number, number>()
  indices.forEach((i, p) => posOf.set(i, p))
  const groups: { g: StackGroup; pos: number }[] = []
  for (const stem of stems) {
    groups.push(buildGroup(stem, byStem.get(stem)!, items, posOf))
  }
  groups.sort((a, b) => a.pos - b.pos)
  return groups.map((x) => x.g)
}

/**
 * 同组照片堆叠：按拍摄时间聚类（与显示序无关），相邻时间戳差 ≤ gapMs 归为同组。
 * - 组内成员按时间序（同时间戳保持显示序）；组 id = `t-<组内最早时间戳>`，
 *   比组序号稳定：筛选变化时最早帧仍在组内则 id 不变，覆盖激活不失效
 * - 组位置 = 组内成员在显示序中的最小位置（组按此排序输出）
 * - 无 dateTaken/解析失败的成员 = 独立单成员组（`x-<下标>`，不并入任何时间组）
 * 空输入返回空数组。
 */
export function groupByTime(
  indices: number[],
  items: CaptureMeta[],
  gapMs = STACK_TIME_GAP_MS,
): StackGroup[] {
  const withTime: { idx: number; t: number }[] = []
  const noTime: number[] = []
  for (const i of indices) {
    const t = parseExifDate(items[i]?.dateTaken ?? null)
    if (t === null) noTime.push(i)
    else withTime.push({ idx: i, t })
  }
  // 稳定按时间排序（同时间戳保持显示序；Array.sort 稳定）
  withTime.sort((a, b) => a.t - b.t)
  // 聚类：相邻差 ≤ gapMs 同组（与 burst.ts 连拍判定一致）
  const clusters: { members: number[]; t0: number }[] = []
  let cur: number[] = []
  let t0 = 0
  let prevT: number | null = null
  for (const e of withTime) {
    if (cur.length > 0 && prevT !== null && e.t - prevT > gapMs) {
      clusters.push({ members: cur, t0 })
      cur = []
    }
    if (cur.length === 0) t0 = e.t
    cur.push(e.idx)
    prevT = e.t
  }
  if (cur.length > 0) clusters.push({ members: cur, t0 })

  const posOf = new Map<number, number>()
  indices.forEach((i, p) => posOf.set(i, p))
  const groups: { g: StackGroup; pos: number }[] = []
  for (const c of clusters) {
    groups.push(buildGroup(`t-${c.t0}`, c.members, items, posOf))
  }
  for (const i of noTime) {
    groups.push(buildGroup(`x-${i}`, [i], items, posOf))
  }
  groups.sort((a, b) => a.pos - b.pos)
  return groups.map((x) => x.g)
}
