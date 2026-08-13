// GPS 坐标转换测试：参考值来自 npm coordtransform 2.1.2 官方实现输出（同源算法逐位核对）。
// 用例：境内转换、境外直通、outOfChina 边界判据。
import { describe, expect, it } from 'vitest'
import { outOfChina, wgs84ToBd09 } from './geo'

describe('wgs84ToBd09', () => {
  it('境内坐标按标准算法转换（北京参考点，与 coordtransform 官方输出逐位一致）', () => {
    // coordtransform.wgs84togcj02(116.404, 39.915) → [116.41024449916938, 39.91640428150164]
    // coordtransform.gcj02tobd09(...) → [116.41662724378733, 39.922699552216216]
    expect(wgs84ToBd09(39.915, 116.404)).toEqual({
      lat: 39.922699552216216,
      lng: 116.41662724378733,
    })
  })

  it('境内坐标偏移量级合理（北京点 BD-09 相对 WGS-84 偏移约 0.01° 级）', () => {
    const bd = wgs84ToBd09(39.915, 116.404)
    expect(Math.abs(bd.lat - 39.915)).toBeGreaterThan(0.001)
    expect(Math.abs(bd.lng - 116.404)).toBeGreaterThan(0.001)
  })

  it('境外坐标直通不转换（纽约）', () => {
    expect(wgs84ToBd09(40.7128, -74.006)).toEqual({ lat: 40.7128, lng: -74.006 })
  })
})

describe('outOfChina', () => {
  it('境内（北京）返回 false', () => {
    expect(outOfChina(39.915, 116.404)).toBe(false)
  })

  it('境外（纽约/欧洲）返回 true', () => {
    expect(outOfChina(40.7128, -74.006)).toBe(true)
    expect(outOfChina(48.8566, 2.3522)).toBe(true)
  })

  it('边界值：恰好落在边界算境内，略出边界算境外', () => {
    // 判据为严格 <：lat=0.8293 / lng=72.004 本身不算境外
    expect(outOfChina(0.8293, 116.404)).toBe(false)
    expect(outOfChina(39.915, 72.004)).toBe(false)
    // 略出边界（南纬 0.0001° / 西经 0.0001°）即境外
    expect(outOfChina(0.8293 - 0.0001, 116.404)).toBe(true)
    expect(outOfChina(39.915, 72.004 - 0.0001)).toBe(true)
  })
})
