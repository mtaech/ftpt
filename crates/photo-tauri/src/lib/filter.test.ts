// filter.ts 纯逻辑测试：覆盖无筛选、各条件单独生效、组合、排序、边界。
// 判定边界对齐 GPUI 版 state/filter.rs（含「dateTaken 解析失败保留」的 Rust 行为）。
import { describe, expect, it } from 'vitest'
import type { CaptureMeta, FilterCriteria, SortBy, SortDirection } from './bindings'
import {
  applyFilterAndSort,
  defaultFilterCriteria,
  filterCaptures,
  formatToString,
  hasActiveFilters,
} from './filter'

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

const opts = (sortBy: SortBy, sortDirection: SortDirection) => ({
  criteria: defaultFilterCriteria(),
  sortBy,
  sortDirection,
})

describe('filterCaptures', () => {
  it('无筛选条件返回全部（保持原序）', () => {
    const items = [mk({ baseName: 'B' }), mk({ baseName: 'A' }), mk({ baseName: 'C' })]
    expect(filterCaptures(items, defaultFilterCriteria())).toEqual([0, 1, 2])
  })

  it('格式筛选：JPEG / PNG / RAW（大小写不敏感，mock 数据为小写 primaryFormat）', () => {
    const items = [
      mk({ baseName: 'a', primaryFormat: 'jpeg' }),
      mk({ baseName: 'b', primaryFormat: 'png' }),
      mk({ baseName: 'c', primaryFormat: 'raw' }),
      mk({ baseName: 'd', primaryFormat: 'NEF' }), // RAW 扩展名（真实数据为 Display 大写）
    ]
    const c = defaultFilterCriteria()
    c.formatFilter = 'Jpeg'
    expect(filterCaptures(items, c)).toEqual([0])
    c.formatFilter = { Raw: 'RAW' }
    expect(filterCaptures(items, c)).toEqual([2])
    c.formatFilter = { Raw: 'NEF' }
    expect(filterCaptures(items, c)).toEqual([3])
  })

  it('鸟种多选：命中任一选中项即保留', () => {
    const items = [
      mk({ baseName: 'a', birdName: '白鹭' }),
      mk({ baseName: 'b', birdName: '翠鸟' }),
      mk({ baseName: 'c', birdName: null }),
    ]
    const c = defaultFilterCriteria()
    c.birdNames = ['白鹭', '翠鸟']
    expect(filterCaptures(items, c)).toEqual([0, 1])
    c.birdNames = ['麻雀']
    expect(filterCaptures(items, c)).toEqual([])
  })

  it('日期范围：from/to 各自生效；无拍摄时间排除、解析失败保留（Rust 边界）', () => {
    const items = [
      mk({ baseName: 'a', dateTaken: '2026-08-01T10:00:00' }),
      mk({ baseName: 'b', dateTaken: '2026-08-05 09:30:00' }), // 空格分隔格式也可解析
      mk({ baseName: 'c', dateTaken: '2026-08-10T08:00:00' }),
      mk({ baseName: 'd', dateTaken: null }),
      mk({ baseName: 'e', dateTaken: 'not-a-date' }),
    ]
    const c = defaultFilterCriteria()
    c.dateFrom = '2026-08-03'
    c.dateTo = '2026-08-08'
    // b 在范围内；a/c 出界；d 无时间排除；e 解析失败保留（if let Ok 无 else 分支）
    expect(filterCaptures(items, c)).toEqual([1, 4])
    c.dateFrom = null
    c.dateTo = '2026-08-02'
    expect(filterCaptures(items, c)).toEqual([0, 4])
  })

  it('评分 ≥ N：无评分（None=0）不满足 ≥1，边界精确', () => {
    const items = [
      mk({ baseName: 'a', rating: 'None' }),
      mk({ baseName: 'b', rating: 'One' }),
      mk({ baseName: 'c', rating: 'Two' }),
      mk({ baseName: 'd', rating: 'Five' }),
    ]
    const c = defaultFilterCriteria()
    c.minRating = 'One'
    expect(filterCaptures(items, c)).toEqual([1, 2, 3])
    c.minRating = 'Two'
    expect(filterCaptures(items, c)).toEqual([2, 3])
    c.minRating = 'Five'
    expect(filterCaptures(items, c)).toEqual([3])
  })

  it('色标精确匹配', () => {
    const items = [
      mk({ baseName: 'a', colorLabel: 'Red' }),
      mk({ baseName: 'b', colorLabel: 'None' }),
      mk({ baseName: 'c', colorLabel: 'Red' }),
    ]
    const c = defaultFilterCriteria()
    c.colorLabel = 'Red'
    expect(filterCaptures(items, c)).toEqual([0, 2])
  })

  it('旗标筛选：Pick/Reject 精确匹配，未标记只留无旗标', () => {
    const items = [
      mk({ baseName: 'a', flag: 'Pick' }),
      mk({ baseName: 'b', flag: 'Reject' }),
      mk({ baseName: 'c', flag: null }),
    ]
    const c = defaultFilterCriteria()
    c.flagFilter = 'Pick'
    expect(filterCaptures(items, c)).toEqual([0])
    c.flagFilter = 'Reject'
    expect(filterCaptures(items, c)).toEqual([1])
    c.flagFilter = null
    c.unflaggedFilter = true
    expect(filterCaptures(items, c)).toEqual([2])
  })

  it('识别状态：四种分别生效（NotRecognized = 无识别记录）', () => {
    const items = [
      mk({ baseName: 'a', recognitionStatus: 'Confirmed' }),
      mk({ baseName: 'b', recognitionStatus: 'NeedsReview' }),
      mk({ baseName: 'c', recognitionStatus: 'Unrecognized' }),
      mk({ baseName: 'd', recognitionStatus: null }),
    ]
    const c = defaultFilterCriteria()
    c.recognitionFilter = 'Confirmed'
    expect(filterCaptures(items, c)).toEqual([0])
    c.recognitionFilter = 'NeedsReview'
    expect(filterCaptures(items, c)).toEqual([1])
    c.recognitionFilter = 'Unrecognized'
    expect(filterCaptures(items, c)).toEqual([2])
    c.recognitionFilter = 'NotRecognized'
    expect(filterCaptures(items, c)).toEqual([3])
  })

  it('ISO 区间：闭区间边界精确，无 ISO 数据排除', () => {
    const items = [
      mk({ iso: 100 }),
      mk({ iso: 200 }),
      mk({ iso: 400 }),
      mk({ iso: null }),
    ]
    const c = defaultFilterCriteria()
    c.isoMin = 200
    c.isoMax = 400
    expect(filterCaptures(items, c)).toEqual([1, 2])
    // 单侧限制
    const c2 = defaultFilterCriteria()
    c2.isoMin = 200
    expect(filterCaptures(items, c2)).toEqual([1, 2])
    const c3 = defaultFilterCriteria()
    c3.isoMax = 200
    expect(filterCaptures(items, c3)).toEqual([0, 1])
  })

  it('焦距区间：解析 "600mm"/"840mm" 数值，解析失败/无焦距排除', () => {
    const items = [
      mk({ focalLength: '600mm' }),
      mk({ focalLength: '840mm' }),
      mk({ focalLength: '50mm' }),
      mk({ focalLength: '无法解析' }),
      mk({ focalLength: null }),
    ]
    const c = defaultFilterCriteria()
    c.focalMin = 600
    expect(filterCaptures(items, c)).toEqual([0, 1])
    const c2 = defaultFilterCriteria()
    c2.focalMin = 500
    c2.focalMax = 700
    expect(filterCaptures(items, c2)).toEqual([0])
    // 设了区间但焦距缺失/解析失败 → 排除
    const c3 = defaultFilterCriteria()
    c3.focalMax = 100
    expect(filterCaptures(items, c3)).toEqual([2])
  })

  it('镜头多选：精确匹配 EXIF lens 串，任一命中保留，无镜头排除', () => {
    const items = [
      mk({ lens: 'NIKKOR Z 600mm f/4 TC VR S' }),
      mk({ lens: 'NIKKOR Z 24-70mm f/2.8' }),
      mk({ lens: 'NIKKOR Z 600mm f/4 TC VR S' }),
      mk({ lens: null }),
    ]
    const c = defaultFilterCriteria()
    c.lensFilter = ['NIKKOR Z 600mm f/4 TC VR S']
    expect(filterCaptures(items, c)).toEqual([0, 2])
    const c2 = defaultFilterCriteria()
    c2.lensFilter = ['NIKKOR Z 600mm f/4 TC VR S', 'NIKKOR Z 24-70mm f/2.8']
    expect(filterCaptures(items, c2)).toEqual([0, 1, 2])
  })

  it('关键词筛选：包含任一选中关键词即中，空/无命中排除', () => {
    const items = [
      mk({ keywords: ['精选', '测试'] }),
      mk({ keywords: ['天空'] }),
      mk({ keywords: [] }),
    ]
    const c = defaultFilterCriteria()
    c.keywordFilter = ['精选']
    expect(filterCaptures(items, c)).toEqual([0])
    // 任一命中：任一选中词出现在该图关键词中即保留（item0 无 天空/不存在，排除）
    const c2 = defaultFilterCriteria()
    c2.keywordFilter = ['天空', '不存在']
    expect(filterCaptures(items, c2)).toEqual([1])
  })

  it('EXIF/关键词组合：ISO+焦距+镜头+关键词按「与」同时生效', () => {
    const items = [
      mk({ iso: 400, focalLength: '600mm', lens: 'L1', keywords: ['鸟'] }),
      mk({ iso: 400, focalLength: '600mm', lens: 'L1', keywords: ['花'] }),
      mk({ iso: 800, focalLength: '600mm', lens: 'L1', keywords: ['鸟'] }),
      mk({ iso: 400, focalLength: '600mm', lens: 'L2', keywords: ['鸟'] }),
    ]
    const c = defaultFilterCriteria()
    c.isoMin = 400
    c.isoMax = 400
    c.focalMin = 500
    c.lensFilter = ['L1']
    c.keywordFilter = ['鸟']
    expect(filterCaptures(items, c)).toEqual([0])
  })

  it('组合筛选：多条件按「与」同时生效', () => {
    const items = [
      mk({ baseName: 'a', primaryFormat: 'jpeg', rating: 'Three', flag: 'Pick', dateTaken: '2026-08-02T08:00:00' }),
      mk({ baseName: 'b', primaryFormat: 'jpeg', rating: 'Three', flag: 'Pick', dateTaken: '2026-08-03T08:00:00' }),
      mk({ baseName: 'c', primaryFormat: 'jpeg', rating: 'One', flag: 'Pick', dateTaken: '2026-08-03T08:00:00' }),
      mk({ baseName: 'd', primaryFormat: 'png', rating: 'Three', flag: 'Pick', dateTaken: '2026-08-03T08:00:00' }),
    ]
    const c: FilterCriteria = {
      formatFilter: 'Jpeg',
      birdNames: [],
      dateFrom: '2026-08-03',
      dateTo: null,
      minRating: 'Two',
      colorLabel: null,
      flagFilter: 'Pick',
      unflaggedFilter: false,
      recognitionFilter: 'All',
      isoMin: null,
      isoMax: null,
      focalMin: null,
      focalMax: null,
      lensFilter: [],
      keywordFilter: [],
    }
    expect(filterCaptures(items, c)).toEqual([1])
  })
})

describe('applyFilterAndSort', () => {
  const items = [
    mk({ baseName: 'b', dateTaken: null, fileSize: 300, rating: 'One' }),
    mk({ baseName: 'C', dateTaken: '2026-08-01T00:00:00', fileSize: 100, rating: 'Three' }),
    mk({ baseName: 'a', dateTaken: '2026-08-02T00:00:00', fileSize: 200, rating: 'Two' }),
  ]

  it('文件名排序：小写归一后码点序', () => {
    expect(applyFilterAndSort(items, opts('FileName', 'Ascending'))).toEqual([2, 0, 1])
  })

  it('拍摄日期排序：null 按空串垫底', () => {
    expect(applyFilterAndSort(items, opts('DateTaken', 'Ascending'))).toEqual([0, 1, 2])
  })

  it('文件大小排序', () => {
    expect(applyFilterAndSort(items, opts('FileSize', 'Ascending'))).toEqual([1, 2, 0])
  })

  it('评分排序（None=0）', () => {
    expect(applyFilterAndSort(items, opts('Rating', 'Ascending'))).toEqual([0, 2, 1])
  })

  it('修改时间排序：以 dateTaken 为代理，无时间（null）排最前', () => {
    expect(applyFilterAndSort(items, opts('Modified', 'Ascending'))).toEqual([0, 1, 2])
  })

  it('眼锐度排序：全 None 保持原序（稳定）', () => {
    const all = [
      mk({ baseName: 'a', eyeSharpness: null }),
      mk({ baseName: 'b', eyeSharpness: null }),
      mk({ baseName: 'c', eyeSharpness: null }),
    ]
    expect(applyFilterAndSort(all, opts('EyeSharpness', 'Ascending'))).toEqual([0, 1, 2])
    expect(applyFilterAndSort(all, opts('EyeSharpness', 'Descending'))).toEqual([0, 1, 2])
  })

  it('眼锐度排序：部分 None 时 None 排最前，有值按数值升序', () => {
    const mixed = [
      mk({ baseName: 'a', eyeSharpness: null }),
      mk({ baseName: 'b', eyeSharpness: 2.5 }),
      mk({ baseName: 'c', eyeSharpness: 1.0 }),
    ]
    // None(0) → 1.0(2) → 2.5(1)
    expect(applyFilterAndSort(mixed, opts('EyeSharpness', 'Ascending'))).toEqual([0, 2, 1])
    // 降序 = 升序反转（None 垫底，由外层 sortDirection 统一处理）
    expect(applyFilterAndSort(mixed, opts('EyeSharpness', 'Descending'))).toEqual([1, 2, 0])
  })

  it('眼锐度排序：纯数值升降序', () => {
    const nums = [
      mk({ baseName: 'a', eyeSharpness: 1.0 }),
      mk({ baseName: 'b', eyeSharpness: 3.0 }),
      mk({ baseName: 'c', eyeSharpness: 2.0 }),
    ]
    expect(applyFilterAndSort(nums, opts('EyeSharpness', 'Ascending'))).toEqual([0, 2, 1])
    expect(applyFilterAndSort(nums, opts('EyeSharpness', 'Descending'))).toEqual([1, 2, 0])
  })

  it('眼锐度排序与评分排序互不影响', () => {
    const mixed = [
      mk({ baseName: 'a', rating: 'One', eyeSharpness: 1.0 }),
      mk({ baseName: 'b', rating: 'Three', eyeSharpness: null }),
      mk({ baseName: 'c', rating: 'Two', eyeSharpness: 2.0 }),
    ]
    // 眼锐度：None(1) → 1.0(0) → 2.0(2)；评分：One(0) → Two(2) → Three(1)
    expect(applyFilterAndSort(mixed, opts('EyeSharpness', 'Ascending'))).toEqual([1, 0, 2])
    expect(applyFilterAndSort(mixed, opts('Rating', 'Ascending'))).toEqual([0, 2, 1])
  })

  it('技术分排序：按分数升序，None（未评分）排最后', () => {
    const q = [
      mk({ baseName: 'a', primaryPath: 'E:/Mock/Birds/a.jpg' }),
      mk({ baseName: 'b', primaryPath: 'E:/Mock/Birds/b.jpg' }),
      mk({ baseName: 'c', primaryPath: 'E:/Mock/Birds/c.jpg' }),
    ]
    const qs = {
      'E:/Mock/Birds/a.jpg': 0.9,
      'E:/Mock/Birds/b.jpg': 0.3,
      // c 未评分
    }
    const qOpts = (dir: SortDirection) => ({
      criteria: defaultFilterCriteria(),
      sortBy: 'Quality' as SortBy,
      sortDirection: dir,
      qualityScores: qs,
    })
    // 升序：b(0.3) → a(0.9) → c(None 垫底)
    expect(applyFilterAndSort(q, qOpts('Ascending'))).toEqual([1, 0, 2])
    // 降序 = 升序反转（None 排最前，与其他排序键的降序反转语义一致）
    expect(applyFilterAndSort(q, qOpts('Descending'))).toEqual([2, 0, 1])
  })

  it('技术分排序：分数相等保持原序（稳定）', () => {
    const q = [
      mk({ baseName: 'a', primaryPath: 'E:/Mock/Birds/a.jpg' }),
      mk({ baseName: 'b', primaryPath: 'E:/Mock/Birds/b.jpg' }),
      mk({ baseName: 'c', primaryPath: 'E:/Mock/Birds/c.jpg' }),
    ]
    const qs = { 'E:/Mock/Birds/a.jpg': 0.5, 'E:/Mock/Birds/b.jpg': 0.5 }
    const qOpts = (dir: SortDirection) => ({
      criteria: defaultFilterCriteria(),
      sortBy: 'Quality' as SortBy,
      sortDirection: dir,
      qualityScores: qs,
    })
    // a/b 同分保持原序，c(None) 垫底
    expect(applyFilterAndSort(q, qOpts('Ascending'))).toEqual([0, 1, 2])
    // 缺省 qualityScores = 全 None（未评分）：任何方向均保持原序
    expect(applyFilterAndSort(q, opts('Quality', 'Ascending'))).toEqual([0, 1, 2])
    expect(applyFilterAndSort(q, opts('Quality', 'Descending'))).toEqual([0, 1, 2])
  })

  it('降序为升序反转', () => {
    expect(applyFilterAndSort(items, opts('Rating', 'Descending'))).toEqual([1, 2, 0])
    expect(applyFilterAndSort(items, opts('FileName', 'Descending'))).toEqual([1, 0, 2])
  })

  it('排序稳定：比较键相等时保持原序', () => {
    const dup = [
      mk({ baseName: 'a', fileSize: 5 }),
      mk({ baseName: 'a', fileSize: 9 }),
      mk({ baseName: 'a', fileSize: 7 }),
    ]
    expect(applyFilterAndSort(dup, opts('FileName', 'Ascending'))).toEqual([0, 1, 2])
  })

  it('先过滤后排序：排序作用于筛选结果', () => {
    const c = defaultFilterCriteria()
    c.minRating = 'Two'
    const r = applyFilterAndSort(items, { criteria: c, sortBy: 'Rating', sortDirection: 'Ascending' })
    // 仅保留 C(Three) 与 a(Two)，按评分升序 → a(2) 在前
    expect(r).toEqual([2, 1])
  })
})

describe('边界', () => {
  it('空列表', () => {
    expect(filterCaptures([], defaultFilterCriteria())).toEqual([])
    expect(applyFilterAndSort([], opts('FileName', 'Ascending'))).toEqual([])
  })

  it('全不匹配返回空', () => {
    const items = [mk({ baseName: 'a', rating: 'None' }), mk({ baseName: 'b', rating: 'One' })]
    const c = defaultFilterCriteria()
    c.minRating = 'Five'
    expect(filterCaptures(items, c)).toEqual([])
  })

  it('单条列表：任何排序下仍返回自身', () => {
    const items = [mk({ baseName: 'only' })]
    for (const sortBy of ['FileName', 'DateTaken', 'FileSize', 'Rating', 'Modified', 'EyeSharpness', 'Quality'] as SortBy[]) {
      expect(applyFilterAndSort(items, opts(sortBy, 'Ascending'))).toEqual([0])
      expect(applyFilterAndSort(items, opts(sortBy, 'Descending'))).toEqual([0])
    }
  })
})

describe('辅助函数', () => {
  it('hasActiveFilters：任一条件激活即 true（含 colorLabel），默认全 false', () => {
    expect(hasActiveFilters(defaultFilterCriteria())).toBe(false)
    const c = defaultFilterCriteria()
    c.colorLabel = 'Red'
    expect(hasActiveFilters(c)).toBe(true)
    c.colorLabel = null
    c.recognitionFilter = 'Confirmed'
    expect(hasActiveFilters(c)).toBe(true)
    c.recognitionFilter = 'All'
    c.isoMin = 100
    expect(hasActiveFilters(c)).toBe(true)
    c.isoMin = null
    c.isoMax = 3200
    expect(hasActiveFilters(c)).toBe(true)
    c.isoMax = null
    c.focalMin = 400
    expect(hasActiveFilters(c)).toBe(true)
    c.focalMin = null
    c.focalMax = 800
    expect(hasActiveFilters(c)).toBe(true)
    c.focalMax = null
    c.lensFilter = ['L1']
    expect(hasActiveFilters(c)).toBe(true)
    c.lensFilter = []
    c.keywordFilter = ['鸟']
    expect(hasActiveFilters(c)).toBe(true)
  })

  it('formatToString：对齐 domain.rs Display（含 Raw 载荷）', () => {
    expect(formatToString('Jpeg')).toBe('JPEG')
    expect(formatToString('Png')).toBe('PNG')
    expect(formatToString('WebP')).toBe('WebP')
    expect(formatToString({ Raw: 'NEF' })).toBe('NEF')
    expect(formatToString('Other')).toBe('OTHER')
  })
})
