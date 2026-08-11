// 筛选/排序纯逻辑移植（GPUI 版 crates/photo-tool-app/src/state/filter.rs 的
// apply_filter_and_sort 过滤段 + domain.rs FilterCriteria::has_active_filter）。
// 前端持有全量 CaptureMeta，筛选/排序零 IPC（迁移计划 Q5）。
// 判定边界与 Rust 版逐一对应，测试见 filter.test.ts。
import type {
  CaptureMeta,
  FilterCriteria,
  ImageFormat,
  Rating,
  SortBy,
  SortDirection,
} from './bindings'

/** Rating → 数字（0–5），对齐 domain.rs 的 `as u8`（None=0 … Five=5） */
const RATING_ORDER: Rating[] = ['None', 'One', 'Two', 'Three', 'Four', 'Five']

function ratingValue(r: Rating): number {
  return RATING_ORDER.indexOf(r)
}

/** ImageFormat → 显示串，对齐 domain.rs 的 Display impl（primaryFormat 即其输出） */
export function formatToString(fmt: ImageFormat): string {
  switch (fmt) {
    case 'Jpeg':
      return 'JPEG'
    case 'Png':
      return 'PNG'
    case 'Tiff':
      return 'TIFF'
    case 'Heif':
      return 'HEIF'
    case 'WebP':
      return 'WebP'
    case 'Bmp':
      return 'BMP'
    case 'Gif':
      return 'GIF'
    case 'Other':
      return 'OTHER'
    default:
      return fmt.Raw // { Raw: string }：RAW 扩展名原样输出
  }
}

/**
 * 格式身份匹配：primaryFormat 与筛选格式小写归一后比较。
 * Rust 是精确字符串比较（meta.primary_format != fmt.to_string()），但两套
 * 数据大小写不一致——真实数据为 Display 大写（"JPEG"），mock 为小写
 * （"jpeg"）——小写比较保持「格式身份相等」这一判定边界，且不同格式的
 * 字符串不会因归一而互相碰撞。
 */
function matchesFormat(primaryFormat: string, fmt: ImageFormat): boolean {
  return primaryFormat.toLowerCase() === formatToString(fmt).toLowerCase()
}

/** 拍摄时间 → 日期串 YYYY-MM-DD；仅接受 filter.rs 的两种 NaiveDateTime 格式 */
const DATE_TAKEN_RE = /^(\d{4}-\d{2}-\d{2})[T ]\d{2}:\d{2}:\d{2}$/

function parseDateTaken(dateStr: string): string | null {
  const m = DATE_TAKEN_RE.exec(dateStr)
  return m ? m[1] : null
}

/**
 * 焦距字符串 → 毫米数值（如 "600mm" → 600、"840mm" → 840）。
 * 变焦镜头取首个数值（广角端，如 "70-200mm" → 70）——对"焦距≥X"的鸟类拍摄
 * 场景判界保守且可预期。解析失败返回 null（设了焦距区间时按"不匹配"排除）。
 */
export function parseFocalLengthMm(s: string): number | null {
  const m = /(\d+(?:\.\d+)?)/.exec(s)
  return m ? parseFloat(m[1]) : null
}

/**
 * 过滤：返回通过全部条件的下标数组（升序）。边界与 filter.rs 逐一对应：
 * - dateTaken 为 null 且设了日期范围 → 排除；dateTaken 存在但解析失败 → 保留
 *   （Rust 的 `if let Ok` 无 else 分支，解析失败直接落到下一条件）；
 * - minRating：无评分（None=0）不满足 ≥1；
 * - NotRecognized = 无识别记录（recognitionStatus 为 null）。
 */
export function filterCaptures(items: CaptureMeta[], criteria: FilterCriteria): number[] {
  const out: number[] = []
  for (let i = 0; i < items.length; i++) {
    const meta = items[i]
    // format_filter：格式精确匹配（大小写归一）
    if (criteria.formatFilter !== null && !matchesFormat(meta.primaryFormat, criteria.formatFilter)) {
      continue
    }
    // bird_names：鸟种多选，命中任一选中项即保留
    if (criteria.birdNames.length > 0) {
      if (meta.birdName === null || !criteria.birdNames.includes(meta.birdName)) continue
    }
    // date_from / date_to
    if (criteria.dateFrom !== null || criteria.dateTo !== null) {
      if (meta.dateTaken !== null) {
        const d = parseDateTaken(meta.dateTaken)
        if (d !== null) {
          if (criteria.dateFrom !== null && d < criteria.dateFrom) continue
          if (criteria.dateTo !== null && d > criteria.dateTo) continue
        }
        // 解析失败：Rust 行为 = 保留（无 else 分支）
      } else {
        continue // 无拍摄时间且设了日期范围 → 排除
      }
    }
    // unflagged_filter：只显示没有旗标的照片
    if (criteria.unflaggedFilter && meta.flag !== null) continue
    // min_rating：评分 ≥ N
    if (criteria.minRating !== null && ratingValue(meta.rating) < ratingValue(criteria.minRating)) {
      continue
    }
    // color_label：颜色标签精确匹配
    if (criteria.colorLabel !== null && meta.colorLabel !== criteria.colorLabel) continue
    // flag_filter：旗标精确匹配（Pick/Reject）
    if (criteria.flagFilter !== null && meta.flag !== criteria.flagFilter) continue
    // iso 区间（闭区间 [isoMin, isoMax]；无 ISO 数据且设了区间 → 排除）
    if (criteria.isoMin !== null || criteria.isoMax !== null) {
      if (meta.iso === null) continue
      if (criteria.isoMin !== null && meta.iso < criteria.isoMin) continue
      if (criteria.isoMax !== null && meta.iso > criteria.isoMax) continue
    }
    // 焦距区间（闭区间 [focalMin, focalMax]，mm 数值；无焦距或解析失败 → 排除）
    if (criteria.focalMin !== null || criteria.focalMax !== null) {
      if (meta.focalLength === null) continue
      const mm = parseFocalLengthMm(meta.focalLength)
      if (mm === null) continue // 解析失败 = 不匹配（有筛选时排除）
      if (criteria.focalMin !== null && mm < criteria.focalMin) continue
      if (criteria.focalMax !== null && mm > criteria.focalMax) continue
    }
    // 镜头多选（精确匹配 EXIF lens 串；空 = 不限）
    if (criteria.lensFilter.length > 0) {
      if (meta.lens === null || !criteria.lensFilter.includes(meta.lens)) continue
    }
    // 关键词筛选：包含任一选中关键词即中（空 = 不限）
    if (criteria.keywordFilter.length > 0) {
      if (!meta.keywords.some((k) => criteria.keywordFilter.includes(k))) continue
    }
    // recognition_filter
    switch (criteria.recognitionFilter) {
      case 'Confirmed':
        if (meta.recognitionStatus !== 'Confirmed') continue
        break
      case 'NeedsReview':
        if (meta.recognitionStatus !== 'NeedsReview') continue
        break
      case 'Unrecognized':
        if (meta.recognitionStatus !== 'Unrecognized') continue
        break
      case 'NotRecognized':
        if (meta.recognitionStatus !== null) continue
        break
      case 'All':
        break
    }
    out.push(i)
  }
  return out
}

/** 字符串比较（对齐 Rust String::cmp：码点序，非 locale） */
function cmpStr(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0
}

/**
 * 两拍摄的排序比较（对齐 filter.rs 的 sort_by 比较器）。
 * Modified 以 dateTaken 为 mtime 代理：浏览器端无 fs::metadata，拍摄时间为空
 * = None 排最前（模拟 Rust Option<SystemTime> 的 None < Some 语义）。
 * EyeSharpness 为 T0 新增（Rust 侧无比较器，纯前端实现）：None 语义对齐 Modified
 * （None 排最前），有值按数值升序；降序由 applyFilterAndSort 外层反转统一处理。
 */
export function compareCaptures(sortBy: SortBy, a: CaptureMeta, b: CaptureMeta): number {
  switch (sortBy) {
    case 'FileName':
      return cmpStr(a.baseName.toLowerCase(), b.baseName.toLowerCase())
    case 'DateTaken':
      return cmpStr(a.dateTaken ?? '', b.dateTaken ?? '') // 对齐 unwrap_or("")
    case 'FileSize':
      return (a.fileSize ?? 0) - (b.fileSize ?? 0) // 对齐 unwrap_or(0)
    case 'Rating':
      return ratingValue(a.rating) - ratingValue(b.rating)
    case 'EyeSharpness': {
      const sa = a.eyeSharpness
      const sb = b.eyeSharpness
      if (sa === null && sb === null) return 0
      if (sa === null) return -1 // None 排最前（对齐 Modified 分支的 Option 语义）
      if (sb === null) return 1
      return sa - sb // 数值升序；降序由外层 sortDirection 处理
    }
    case 'Modified': {
      const ta = a.dateTaken
      const tb = b.dateTaken
      if (ta === null && tb === null) return 0
      if (ta === null) return -1
      if (tb === null) return 1
      return cmpStr(ta, tb)
    }
  }
}

/**
 * 过滤 + 排序 → 下标数组（display_order，即 captures.items 下标）。
 * Array.prototype.sort 在现代引擎为稳定排序，与 Rust 稳定 sort_by 一致。
 */
export function applyFilterAndSort(
  items: CaptureMeta[],
  options: { criteria: FilterCriteria; sortBy: SortBy; sortDirection: SortDirection },
): number[] {
  const { criteria, sortBy, sortDirection } = options
  const indices = filterCaptures(items, criteria)
  indices.sort((a, b) => {
    const cmp = compareCaptures(sortBy, items[a], items[b])
    return sortDirection === 'Ascending' ? cmp : -cmp // 对齐 Descending => cmp.reverse()
  })
  return indices
}

/** 默认筛选条件（对齐 FilterCriteria::default()） */
export function defaultFilterCriteria(): FilterCriteria {
  return {
    formatFilter: null,
    birdNames: [],
    dateFrom: null,
    dateTo: null,
    minRating: null,
    colorLabel: null,
    flagFilter: null,
    unflaggedFilter: false,
    recognitionFilter: 'All',
    isoMin: null,
    isoMax: null,
    focalMin: null,
    focalMax: null,
    lensFilter: [],
    keywordFilter: [],
  }
}

/**
 * 是否有任一筛选条件生效（无筛选时操作集 = 全部文件，批量文件操作应拒绝执行）。
 * 采用 domain.rs FilterCriteria::has_active_filter（含 colorLabel）——
 * GPUI 的 RootView::has_active_filters 漏了 color_label 字段，这里按权威实现。
 */
export function hasActiveFilters(criteria: FilterCriteria): boolean {
  return (
    criteria.formatFilter !== null ||
    criteria.birdNames.length > 0 ||
    criteria.dateFrom !== null ||
    criteria.dateTo !== null ||
    criteria.minRating !== null ||
    criteria.colorLabel !== null ||
    criteria.flagFilter !== null ||
    criteria.unflaggedFilter ||
    criteria.recognitionFilter !== 'All' ||
    criteria.isoMin !== null ||
    criteria.isoMax !== null ||
    criteria.focalMin !== null ||
    criteria.focalMax !== null ||
    criteria.lensFilter.length > 0 ||
    criteria.keywordFilter.length > 0
  )
}
