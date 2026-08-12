// 连拍分组纯逻辑测试：同秒成组、间隔边界（恰好 gapMs / 超阈值拆开）、
// null 与解析失败为独立单张并切断组链、组间交错不误并、单张不成组、空数组。
import { describe, expect, it } from 'vitest'
import type { CaptureMeta } from './bindings'
import { computeBurstGroups, parseExifDate } from './burst'

/** 构造一条 CaptureMeta（缺省 = 中性值），overrides 覆盖测试所需字段（对齐 filter.test.ts） */
function mk(dateTaken: string | null, overrides: Partial<CaptureMeta> = {}): CaptureMeta {
  return {
    index: 0,
    baseName: 'DSC_1000',
    primaryPath: 'E:/Mock/Birds/DSC_1000.jpg',
    primaryFormat: 'JPEG',
    fileSize: 1000,
    dateTaken,
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

describe('parseExifDate', () => {
  it('解析 EXIF 冒号格式与 mock/ISO 破折号格式（同一时刻等价）', () => {
    expect(parseExifDate('2026:06:28 10:15:30')).toBe(Date.UTC(2026, 5, 28, 10, 15, 30))
    expect(parseExifDate('2026-08-01T10:15:30')).toBe(Date.UTC(2026, 7, 1, 10, 15, 30))
  })

  it('null / 空串 / 垃圾串 / 字段越界返回 null', () => {
    expect(parseExifDate(null)).toBeNull()
    expect(parseExifDate('')).toBeNull()
    expect(parseExifDate('not a date')).toBeNull()
    expect(parseExifDate('2026:13:01 00:00:00')).toBeNull() // 13 月越界（Date.UTC 会进位，必须拦截）
    expect(parseExifDate('2026:06:32 00:00:00')).toBeNull() // 32 日越界
    expect(parseExifDate('2026:06:28 25:00:00')).toBeNull() // 25 时越界
    expect(parseExifDate('2026-06-28')).toBeNull() // 缺时间分量
  })
})

describe('computeBurstGroups', () => {
  it('同秒连拍成组：size/pos/groupId 一致，逐项登记', () => {
    const items = [
      mk('2026:06:28 10:15:30'),
      mk('2026:06:28 10:15:30'),
      mk('2026:06:28 10:15:31'),
    ]
    const map = computeBurstGroups(items)
    expect(map.size).toBe(3)
    const entries = [0, 1, 2].map((i) => map.get(i))
    expect(entries[0]).toEqual({ groupId: 'burst-0', size: 3, pos: 0 })
    expect(entries[1]).toEqual({ groupId: 'burst-0', size: 3, pos: 1 })
    expect(entries[2]).toEqual({ groupId: 'burst-0', size: 3, pos: 2 })
  })

  it('间隔边界：恰好 gapMs 归同组，超过阈值拆开', () => {
    // 恰好 2000ms（10:15:30 → 10:15:32）→ 同组
    const atBoundary = computeBurstGroups([
      mk('2026:06:28 10:15:30'),
      mk('2026:06:28 10:15:32'),
    ])
    expect(atBoundary.size).toBe(2)
    expect(atBoundary.get(0)?.groupId).toBe(atBoundary.get(1)?.groupId)

    // 超过阈值（秒级精度下最近的下一个刻度 = 3000ms）→ 拆开，各自单张不登记
    const overBoundary = computeBurstGroups([
      mk('2026:06:28 10:15:30'),
      mk('2026:06:28 10:15:33'),
    ])
    expect(overBoundary.size).toBe(0)

    // 自定义 gapMs 验证精确阈值语义：差 2000ms → gap 1500 时拆开
    const gap1500 = computeBurstGroups([mk('2026:06:28 10:15:30'), mk('2026:06:28 10:15:32')], 1500)
    expect(gap1500.size).toBe(0)
  })

  it('null dateTaken：独立单张不登记，且切断组链（前后两段不合并）', () => {
    const map = computeBurstGroups([
      mk('2026:06:28 10:15:30'),
      mk(null),
      mk('2026:06:28 10:15:31'),
    ])
    // 第 0 项与第 2 项仅差 1s，但中间隔了 null → 各自单张，不误并
    expect(map.size).toBe(0)
  })

  it('解析失败等同 null：单张不登记并切断组链', () => {
    const map = computeBurstGroups([
      mk('2026:06:28 10:15:30'),
      mk('EXIF 缺失'),
      mk('2026:06:28 10:15:30'),
      mk('2026:06:28 10:15:31'),
    ])
    // 第 2、3 项紧邻同秒 → 成组；第 0 项被解析失败项隔离
    expect(map.size).toBe(2)
    expect(map.get(0)).toBeUndefined()
    expect(map.get(2)?.groupId).toBe(map.get(3)?.groupId)
    expect(map.get(2)?.pos).toBe(0)
    expect(map.get(3)?.pos).toBe(1)
  })

  it('组间交错不误并：两组时间接近但被间隔断开，各自独立成组', () => {
    const map = computeBurstGroups([
      mk('2026:06:28 10:15:30'),
      mk('2026:06:28 10:15:31'),
      mk('2026:06:28 10:15:35'), // 与第 1 项差 4s > 2s，断链
      mk('2026:06:28 10:15:36'),
    ])
    expect(map.size).toBe(4)
    const g0 = map.get(0)?.groupId
    const g1 = map.get(1)?.groupId
    const g2 = map.get(2)?.groupId
    const g3 = map.get(3)?.groupId
    expect(g0).toBe(g1)
    expect(g2).toBe(g3)
    expect(g0).not.toBe(g2) // 两组 groupId 不同
    expect(map.get(0)?.size).toBe(2)
    expect(map.get(2)?.size).toBe(2)
  })

  it('单张不成组', () => {
    expect(computeBurstGroups([mk('2026:06:28 10:15:30')]).size).toBe(0)
    expect(computeBurstGroups([mk(null)]).size).toBe(0)
  })

  it('空数组返回空 Map', () => {
    expect(computeBurstGroups([]).size).toBe(0)
  })

  it('显示序即分组序：非拍摄时间排序下仍按传入顺序连续分组', () => {
    // 模拟文件名排序下时间乱序：0/1 紧邻成组，2 时间更早但与 1 差 60s 不并入
    const map = computeBurstGroups([
      mk('2026:06:28 10:15:30'),
      mk('2026:06:28 10:15:31'),
      mk('2026:06:28 10:14:31'),
    ])
    expect(map.size).toBe(2)
    expect(map.get(0)?.groupId).toBe(map.get(1)?.groupId)
    expect(map.get(2)).toBeUndefined()
  })
})
