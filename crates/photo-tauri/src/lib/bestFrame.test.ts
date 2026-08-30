// 连拍组自动选优纯逻辑测试：眼锐度降序、None 垫底、并列取文件更大、再并列取路径
// 字典序小、空组/单成员组返回 null、nonBestPaths 剔除最优帧、结果与输入顺序无关（确定性）。
import { describe, expect, it } from 'vitest'
import type { CaptureMeta } from './bindings'
import { nonBestPaths, pickBestFrame } from './bestFrame'

/** 构造一条 CaptureMeta（缺省 = 中性值），overrides 覆盖测试所需字段（对齐 burst.test.ts） */
function mk(path: string, overrides: Partial<CaptureMeta> = {}): CaptureMeta {
  return {
    index: 0,
    baseName: 'DSC_1000',
    primaryPath: path,
    primaryFormat: 'JPEG',
    fileSize: 1000,
    dateTaken: '2026:08:13 10:00:00',
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

describe('pickBestFrame', () => {
  it('按 eye_sharpness 降序选最优（最高分胜出）', () => {
    const group = [
      mk('E:/Mock/A.jpg', { eyeSharpness: 30 }),
      mk('E:/Mock/B.jpg', { eyeSharpness: 90 }),
      mk('E:/Mock/C.jpg', { eyeSharpness: 60 }),
    ]
    expect(pickBestFrame(group)).toBe('E:/Mock/B.jpg')
  })

  it('None 视为最低：有锐度帧胜过 None 帧', () => {
    const group = [
      mk('E:/Mock/A.jpg', { eyeSharpness: null }),
      mk('E:/Mock/B.jpg', { eyeSharpness: 40 }),
      mk('E:/Mock/C.jpg', { eyeSharpness: null }),
    ]
    expect(pickBestFrame(group)).toBe('E:/Mock/B.jpg')
  })

  it('全 None 回退 tie-break：同锐度并列取文件更大者', () => {
    const group = [
      mk('E:/Mock/C.jpg', { eyeSharpness: null, fileSize: 10 }),
      mk('E:/Mock/D.jpg', { eyeSharpness: null, fileSize: 20 }),
      mk('E:/Mock/E.jpg', { eyeSharpness: null, fileSize: 15 }),
    ]
    expect(pickBestFrame(group)).toBe('E:/Mock/D.jpg')
  })

  it('锐度并列时取文件更大者', () => {
    const group = [
      mk('E:/Mock/A.jpg', { eyeSharpness: 80, fileSize: 5000 }),
      mk('E:/Mock/B.jpg', { eyeSharpness: 80, fileSize: 8000 }),
      mk('E:/Mock/C.jpg', { eyeSharpness: 80, fileSize: 3000 }),
    ]
    expect(pickBestFrame(group)).toBe('E:/Mock/B.jpg')
  })

  it('锐度与大小都并列时取路径字典序小者', () => {
    const group = [
      mk('E:/Mock/B.jpg', { eyeSharpness: 80, fileSize: 5000 }),
      mk('E:/Mock/A.jpg', { eyeSharpness: 80, fileSize: 5000 }),
      mk('E:/Mock/C.jpg', { eyeSharpness: 80, fileSize: 5000 }),
    ]
    expect(pickBestFrame(group)).toBe('E:/Mock/A.jpg')
  })

  it('空组返回 null', () => {
    expect(pickBestFrame([])).toBeNull()
  })

  it('单成员组返回 null', () => {
    expect(pickBestFrame([mk('E:/Mock/A.jpg')])).toBeNull()
  })

  it('确定性：相同输入重复调用结果一致（与输入顺序无关）', () => {
    const group = [
      mk('E:/Mock/B.jpg', { eyeSharpness: 70, fileSize: 2000 }),
      mk('E:/Mock/A.jpg', { eyeSharpness: 90, fileSize: 1000 }),
      mk('E:/Mock/C.jpg', { eyeSharpness: 90, fileSize: 1000 }),
    ]
    const first = pickBestFrame(group)
    const shuffled = [group[2], group[0], group[1]]
    const second = pickBestFrame(shuffled)
    expect(first).toBe(second)
    expect(first).toBe('E:/Mock/A.jpg')
  })
})

describe('nonBestPaths', () => {
  it('返回组内除最优帧外的全部路径（保持原组序）', () => {
    const group = [
      mk('E:/Mock/B.jpg', { eyeSharpness: 30 }),
      mk('E:/Mock/A.jpg', { eyeSharpness: 90 }),
      mk('E:/Mock/C.jpg', { eyeSharpness: 60 }),
    ]
    expect(nonBestPaths(group)).toEqual(['E:/Mock/B.jpg', 'E:/Mock/C.jpg'])
  })

  it('空组/单成员组返回空数组', () => {
    expect(nonBestPaths([])).toEqual([])
    expect(nonBestPaths([mk('E:/Mock/A.jpg')])).toEqual([])
  })
})
