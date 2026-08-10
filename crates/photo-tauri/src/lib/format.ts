// 展示辅助：与 domain.rs 的显示约定对齐。
import type { CaptureMeta, Rating } from './bindings'

/** 评分枚举 → 数字（0–5），键位/徽标共用 */
const RATING_ORDER: Rating[] = ['None', 'One', 'Two', 'Three', 'Four', 'Five']

export function ratingToNumber(r: Rating): number {
  return RATING_ORDER.indexOf(r)
}

export function numberToRating(n: number): Rating {
  return RATING_ORDER[Math.min(Math.max(n, 0), 5)]
}

/**
 * 显示用文件名：baseName + 主路径真实扩展名（对齐 Rust CaptureMeta::display_name，
 * 不用 primaryFormat 拼后缀，避免 .jpg 显示成 .jpeg）。
 */
export function displayName(c: CaptureMeta): string {
  const m = /\.([^.\\/]+)$/.exec(c.primaryPath)
  return m ? `${c.baseName}.${m[1]}` : c.baseName
}

/** 文件大小人性化（状态栏/网格共用，等宽字体展示） */
export function formatBytes(size: number | null): string {
  if (size === null) return ''
  if (size < 1024) return `${size} B`
  const units = ['KB', 'MB', 'GB']
  let v = size / 1024
  let u = 0
  while (v >= 1024 && u < units.length - 1) {
    v /= 1024
    u++
  }
  return `${v >= 100 ? Math.round(v) : v.toFixed(1)} ${units[u]}`
}

/** 常见 RAW 扩展名（对齐 domain.rs is_raw_extension 白名单；静态查找表） */
const RAW_EXTS: Record<string, true> = {
  nef: true,
  cr2: true,
  cr3: true,
  arw: true,
  dng: true,
  orf: true,
  rw2: true,
  raf: true,
  pef: true,
  srw: true,
}

function isRawExt(ext: string): boolean {
  return RAW_EXTS[ext.toLowerCase()] === true
}

/**
 * 格式徽标短标签：JPG/PNG/RAW(NEF)/TIFF/HEIF/WEBP/BMP/GIF/OTHER（网格 cell 左上角）。
 * primaryFormat 为规范化格式名（jpeg/png/…）；RAW 直接存扩展名（NEF/CR3/…）；
 * OTHER 固定大写。mock 形态 primaryFormat='raw' 时回退到 extensions 找真实 RAW 扩展名。
 */
export function formatBadgeLabel(c: CaptureMeta): string {
  const fmt = c.primaryFormat
  switch (fmt.toLowerCase()) {
    case 'jpeg':
      return 'JPG'
    case 'png':
      return 'PNG'
    case 'tiff':
      return 'TIFF'
    case 'heif':
      return 'HEIF'
    case 'webp':
      return 'WEBP'
    case 'bmp':
      return 'BMP'
    case 'gif':
      return 'GIF'
    case 'other':
      return 'OTHER'
    case 'raw': {
      // mock 形态：primaryFormat='raw'，真实扩展名在 extensions 里
      const raw = c.extensions.find(isRawExt)
      return raw ? `RAW(${raw.toUpperCase()})` : 'RAW'
    }
    default:
      // 真实后端形态：primaryFormat 即 RAW 扩展名（如 NEF）
      if (isRawExt(fmt)) return `RAW(${fmt.toUpperCase()})`
      return fmt.toUpperCase()
  }
}

/** 是否非图片格式（视频等）：网格不渲染缩略图，只居中显示格式徽标 */
export function isOtherFormat(c: CaptureMeta): boolean {
  return c.primaryFormat.toUpperCase() === 'OTHER'
}
