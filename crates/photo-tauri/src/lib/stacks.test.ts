// stacks.ts 纯逻辑测试：分组正确性、主格式选择、聚合顺序、边界。
// 覆盖三种堆叠模式：groupSingles（不堆叠）/ groupStacks（同文件名）/
// groupByTime（同组照片——连拍按拍摄时间聚类）。
import { describe, expect, it } from 'vitest'
import type { CaptureMeta } from './bindings'
import { groupByTime, groupSingles, groupStacks, STACK_TIME_GAP_MS } from './stacks'

/** 构造一条 CaptureMeta（缺省 = 中性值），overrides 覆盖测试所需字段 */
function mk(overrides: Partial<CaptureMeta>): CaptureMeta {
  return {
    index: 0,
    baseName: 'DSC_1000',
    primaryPath: 'E:/Mock/Birds/DSC_1000.jpg',
    primaryFormat: 'JPEG',
    fileSize: 1000,
    dateTaken: '2026-08-01T10:00:00',
    extensions: [],
    cameraMake: null,
    cameraModel: null,
    lens: null,
    exposureTime: null,
    fNumber: null,
    iso: null,
    focalLength: null,
    imageWidth: null,
    imageHeight: null,
    rating: 'None',
    colorLabel: 'None',
    flag: null,
    keywords: [],
    gpsLat: null,
    gpsLon: null,
    focusPoint: null,
    birdName: null,
    birdConfidence: null,
    recognitionStatus: null,
    birdBbox: null,
    eyeSharpness: null,
    ...overrides,
  }
}

describe('groupSingles', () => {
  it('每成员独立成组，key 唯一，保持输入序', () => {
    expect(groupSingles([2, 0, 1])).toEqual([
      { key: 'i-2', members: [2], active: 2 },
      { key: 'i-0', members: [0], active: 0 },
      { key: 'i-1', members: [1], active: 1 },
    ])
  })

  it('空输入返回空数组', () => {
    expect(groupSingles([])).toEqual([])
  })
})

describe('groupStacks（同文件名）', () => {
  it('空输入返回空数组', () => {
    expect(groupStacks([], [])).toEqual([])
  })

  it('无同 stem 项时逐项独立成组（单成员组）', () => {
    const items = [mk({ baseName: 'A' }), mk({ baseName: 'B' }), mk({ baseName: 'C' })]
    const groups = groupStacks([0, 1, 2], items)
    expect(groups).toEqual([
      { key: 'A', members: [0], active: 0 },
      { key: 'B', members: [1], active: 1 },
      { key: 'C', members: [2], active: 2 },
    ])
  })

  it('同 stem 项聚合为一组，成员保持显示序出现顺序', () => {
    const items = [
      mk({ baseName: 'IMG_1', primaryPath: 'E:/P/IMG_1.jpg' }),
      mk({ baseName: 'IMG_2', primaryPath: 'E:/P/IMG_2.jpg' }),
      mk({ baseName: 'IMG_1', primaryPath: 'E:/P/IMG_1.nef' }),
    ]
    // 显示序被打散（如按评分排序）：[IMG_2, IMG_1.jpg, IMG_1.nef]
    const groups = groupStacks([1, 0, 2], items)
    expect(groups).toHaveLength(2)
    // 组位置 = 组内首个成员位置：IMG_1 组在第 2 位（下标 0 的成员先出现）
    expect(groups[0]).toEqual({ key: 'IMG_2', members: [1], active: 1 })
    expect(groups[1]).toEqual({ key: 'IMG_1', members: [0, 2], active: 0 })
  })

  it('主格式 = JPEG 优先（真实扩展名判定，不依赖 primaryFormat 规范化名）', () => {
    const items = [
      mk({ baseName: 'X', primaryPath: 'E:/P/X.nef', primaryFormat: 'NEF' }),
      mk({ baseName: 'X', primaryPath: 'E:/P/X.jpeg', primaryFormat: 'jpeg' }), // 假后缀场景
      mk({ baseName: 'X', primaryPath: 'E:/P/X.jpg', primaryFormat: 'JPEG' }),
    ]
    const [g] = groupStacks([0, 1, 2], items)
    expect(g.members).toEqual([0, 1, 2])
    expect(g.active).toBe(2) // .jpg 在 .jpeg 前
  })

  it('纯 RAW 堆叠回退组内首个成员', () => {
    const items = [
      mk({ baseName: 'X', primaryPath: 'E:/P/X.nef' }),
      mk({ baseName: 'X', primaryPath: 'E:/P/X.cr3' }),
    ]
    const [g] = groupStacks([0, 1], items)
    expect(g.active).toBe(0)
  })

  it('无 baseName 的项跳过（防御，正常扫描不产生）', () => {
    const items = [mk({ baseName: '' }), mk({ baseName: 'A' })]
    const groups = groupStacks([0, 1], items)
    expect(groups).toEqual([{ key: 'A', members: [1], active: 1 }])
  })

  it('多组互不干扰：组序 = 各组首个成员出现序', () => {
    const items = [
      mk({ baseName: 'B', primaryPath: 'E:/P/B.jpg' }),
      mk({ baseName: 'A', primaryPath: 'E:/P/A.jpg' }),
      mk({ baseName: 'A', primaryPath: 'E:/P/A.nef' }),
      mk({ baseName: 'B', primaryPath: 'E:/P/B.nef' }),
    ]
    const groups = groupStacks([0, 1, 2, 3], items)
    expect(groups).toHaveLength(2)
    expect(groups[0].key).toBe('B')
    expect(groups[1].key).toBe('A')
  })
})

describe('groupByTime（同组照片）', () => {
  const t = (h: number, m: number, s: number) => `2026-08-01T${String(h).padStart(2, '0')}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`
  const ts = (h: number, m: number, s: number) => Date.UTC(2026, 7, 1, h, m, s)

  it('相邻时间差 ≤2s 的成员合并为一组（连拍），组 id = 组内最早时间戳', () => {
    const items = [
      mk({ baseName: 'A', dateTaken: t(10, 0, 0) }),
      mk({ baseName: 'B', dateTaken: t(10, 0, 1) }),
      mk({ baseName: 'C', dateTaken: t(10, 0, 5) }), // 与 B 差 4s > 2s → 新组
      mk({ baseName: 'D', dateTaken: t(10, 0, 6) }), // 与 C 差 1s → 同组
    ]
    const groups = groupByTime([0, 1, 2, 3], items)
    expect(groups).toHaveLength(2)
    expect(groups[0]).toEqual({ key: `t-${ts(10, 0, 0)}`, members: [0, 1], active: 0 })
    expect(groups[1]).toEqual({ key: `t-${ts(10, 0, 5)}`, members: [2, 3], active: 2 })
  })

  it('与显示序无关：打散显示序仍按时间聚类，组位置 = 组内最小显示位', () => {
    const items = [
      mk({ baseName: 'A', dateTaken: t(10, 0, 0) }),
      mk({ baseName: 'B', dateTaken: t(10, 0, 5) }),
      mk({ baseName: 'C', dateTaken: t(10, 0, 1) }),
    ]
    // 显示序 [A, B, C]：A 与 C 差 1s 同组（时间序 [A, C]），B 独立
    const groups = groupByTime([0, 1, 2], items)
    expect(groups).toHaveLength(2)
    expect(groups[0]).toEqual({ key: `t-${ts(10, 0, 0)}`, members: [0, 2], active: 0 })
    expect(groups[1]).toEqual({ key: `t-${ts(10, 0, 5)}`, members: [1], active: 1 })
  })

  it('无 dateTaken/解析失败的成员 = 独立单成员组，不并入任何时间组', () => {
    const items = [
      mk({ baseName: 'A', dateTaken: t(10, 0, 0) }),
      mk({ baseName: 'B', dateTaken: null }),
      mk({ baseName: 'C', dateTaken: t(10, 0, 1) }),
    ]
    const groups = groupByTime([0, 1, 2], items)
    expect(groups).toHaveLength(2)
    expect(groups[0]).toEqual({ key: `t-${ts(10, 0, 0)}`, members: [0, 2], active: 0 })
    expect(groups[1]).toEqual({ key: 'x-1', members: [1], active: 1 })
  })

  it('同组内主格式仍生效（JPEG 优先）', () => {
    const items = [
      mk({ baseName: 'A', dateTaken: t(10, 0, 0), primaryPath: 'E:/P/A.jpg' }),
      mk({ baseName: 'B', dateTaken: t(10, 0, 0), primaryPath: 'E:/P/B.nef', primaryFormat: 'NEF' }),
      mk({ baseName: 'C', dateTaken: t(10, 0, 1), primaryPath: 'E:/P/C.cr3', primaryFormat: 'CR3' }),
    ]
    const [g] = groupByTime([0, 1, 2], items)
    expect(g.members).toEqual([0, 1, 2]) // 同秒 + 1s → 同组
    expect(g.active).toBe(0) // JPG 优先
  })

  it('时间戳差恰为 gapMs 边界归同组（≤ 语义，与 burst.ts 一致）', () => {
    const items = [
      mk({ baseName: 'A', dateTaken: t(10, 0, 0) }),
      mk({ baseName: 'B', dateTaken: t(10, 0, 2) }), // 差 2000ms = STACK_TIME_GAP_MS
    ]
    const [g] = groupByTime([0, 1], items)
    expect(g.members).toEqual([0, 1])
    expect(STACK_TIME_GAP_MS).toBe(2000)
  })

  it('空输入返回空数组', () => {
    expect(groupByTime([], [])).toEqual([])
  })
})
