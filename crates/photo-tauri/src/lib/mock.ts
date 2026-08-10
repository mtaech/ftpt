// 浏览器纯 vite dev（无 __TAURI__）下的 mock 层：200 条假 CaptureMeta + SVG 占位图 +
// 模拟事件流，保证网格/预览/快捷键在无 Rust 后端时可开发。
import type {
  AdjustParams,
  AppConfig,
  BatchOpOptions,
  BatchOpPreview,
  BatchOpResult,
  BatchOpType,
  CaptureMeta,
  ColorLabel,
  Flag,
  ImageFormat,
} from './bindings'

export const MOCK_DIR = 'E:/Mock/Birds'

type Handler = (payload: unknown) => void
const listeners = new Map<string, Set<Handler>>()

/** mock 事件总线（形态对齐 @tauri-apps/api/event 的 listen） */
export function mockListen(event: string, handler: Handler): () => void {
  let set = listeners.get(event)
  if (!set) {
    set = new Set()
    listeners.set(event, set)
  }
  set.add(handler)
  return () => set.delete(handler)
}

function mockEmit(event: string, payload: unknown) {
  listeners.get(event)?.forEach((h) => h(payload))
}

const FORMATS = ['Jpeg', 'Raw', 'Jpeg', 'Jpeg', 'Raw', 'Png'] as const
const RAWS = ['NEF', 'CR3', 'ARW']

/** 生成 200 条确定性的假数据（评分/色标/旗标部分预置，便于验证渲染） */
function makeCaptures(): CaptureMeta[] {
  const out: CaptureMeta[] = []
  for (let i = 0; i < 200; i++) {
    const isRaw = FORMATS[i % FORMATS.length] === 'Raw'
    const ext = isRaw ? RAWS[i % RAWS.length] : FORMATS[i % FORMATS.length].toUpperCase()
    out.push({
      index: i,
      baseName: `DSC_${String(1000 + i)}`,
      primaryPath: `${MOCK_DIR}/DSC_${String(1000 + i)}.${ext.toLowerCase()}`,
      primaryFormat: isRaw ? 'raw' : ext.toLowerCase(),
      fileSize: 4_000_000 + ((i * 7919) % 20_000_000),
      dateTaken: `2026-08-${String((i % 9) + 1).padStart(2, '0')}T10:${String(i % 60).padStart(2, '0')}:00`,
      extensions: isRaw ? [ext, 'JPG'] : [ext],
      cameraMake: 'NIKON',
      cameraModel: 'Z 9',
      lens: 'NIKKOR Z 600mm f/4 TC VR S',
      exposureTime: '1/2000',
      fNumber: 'f/5.6',
      iso: 400 + (i % 8) * 200,
      focalLength: '840mm',
      imageWidth: 8256,
      imageHeight: 5504,
      rating: i % 7 === 0 ? 'Three' : 'None',
      colorLabel: i % 11 === 0 ? 'Red' : 'None',
      flag: i % 13 === 0 ? 'Pick' : null,
      birdName: null,
      birdConfidence: null,
      recognitionStatus: null,
      birdBbox: null,
    })
  }
  return out
}

let captures: CaptureMeta[] = []

/**
 * SVG 占位图（离线可用）：按路径哈希取色相，网格纹理 + 文件名，
 * kind 决定尺寸（thumb 400×300，master/full 2400×1600）。
 */
export function placeholderImage(kind: string, path: string): string {
  let hash = 0
  for (let i = 0; i < path.length; i++) hash = (hash * 31 + path.charCodeAt(i)) | 0
  const hue = ((hash % 360) + 360) % 360
  const big = kind !== 'thumb'
  const w = big ? 2400 : 400
  const h = big ? 1600 : 300
  const name = path.split('/').pop() ?? path
  const step = big ? 150 : 50
  const svg =
    `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}">` +
    `<rect width="${w}" height="${h}" fill="hsl(${hue},30%,18%)"/>` +
    `<pattern id="g" width="${step}" height="${step}" patternUnits="userSpaceOnUse">` +
    `<path d="M ${step} 0 L 0 0 0 ${step}" fill="none" stroke="hsl(${hue},25%,30%)" stroke-width="1"/>` +
    `</pattern><rect width="${w}" height="${h}" fill="url(#g)"/>` +
    `<text x="${w / 2}" y="${h / 2}" fill="hsl(${hue},60%,75%)" font-family="monospace" ` +
    `font-size="${big ? 72 : 20}" text-anchor="middle" dominant-baseline="middle">${name}</text>` +
    `</svg>`
  return `data:image/svg+xml,${encodeURIComponent(svg)}`
}

/** mock 版 6 个 command：内存态增删改 + 模拟扫描进度/缩略图事件 */
export const mockCommands = {
  async pickDirectory(): Promise<string | null> {
    return MOCK_DIR
  },

  async scanDirectory(_path: string): Promise<number> {
    captures = makeCaptures()
    const total = captures.length
    // 模拟扫描 → EXIF 回填 → 缩略图三段进度
    const stages = ['scan', 'exif', 'thumb'] as const
    let delay = 0
    for (const stage of stages) {
      for (let done = 1; done <= 4; done++) {
        delay += 60
        setTimeout(
          () => mockEmit('scan:progress', { stage, done: Math.round((total * done) / 4), total }),
          delay,
        )
      }
    }
    setTimeout(() => mockEmit('scan:done', { total, directory: MOCK_DIR }), delay + 60)
    // 逐张缩略图就绪事件（抽 24 张模拟，验证 thumb:ready 刷新链路）
    for (let i = 0; i < 24; i++) {
      setTimeout(() => mockEmit('thumb:ready', { path: captures[i * 8].primaryPath }), delay + 150 + i * 80)
    }
    return total
  },

  async getCaptures(): Promise<CaptureMeta[]> {
    return captures
  },

  async setRating(paths: string[], rating: number): Promise<void> {
    const names = ['None', 'One', 'Two', 'Three', 'Four', 'Five'] as const
    for (const c of captures) if (paths.includes(c.primaryPath)) c.rating = names[rating] ?? 'None'
  },

  async setFlag(paths: string[], flag: Flag | null): Promise<void> {
    for (const c of captures) if (paths.includes(c.primaryPath)) c.flag = flag
  },

  async setColorLabel(paths: string[], label: ColorLabel | null): Promise<void> {
    for (const c of captures) if (paths.includes(c.primaryPath)) c.colorLabel = label ?? 'None'
  },

  // ── Phase 2/3 mock：内存态 + 模拟事件流 ──────────────

  async listFavorites(): Promise<string[]> {
    return [...favorites]
  },
  async addFavorite(path: string): Promise<void> {
    if (!favorites.includes(path)) favorites.push(path)
  },
  async removeFavorite(path: string): Promise<void> {
    favorites = favorites.filter((f) => f !== path)
  },
  async listRecent(): Promise<string[]> {
    return [...recent]
  },
  async listBirdSpecies(): Promise<string[]> {
    return ['北红尾鸲', '斑嘴鸭', '白鹭', '苍鹭', '翠鸟', '红嘴蓝鹊', '麻雀', '山斑鸠']
  },
  async recognizeCaptures(paths: string[]): Promise<void> {
    const total = paths.length
    let done = 0
    for (const path of paths) {
      await sleep(80)
      done += 1
      mockEmit('recognize:progress', { done, total, currentPath: path })
      const c = captures.find((x) => x.primaryPath === path)
      if (c) {
        c.recognitionStatus = 'Confirmed'
        c.birdName = '白鹭'
        c.birdConfidence = 0.87 + (done % 10) / 100
      }
    }
    mockEmit('recognize:done', { total, confirmed: total, needsReview: 0, unrecognized: 0, failed: 0 })
  },
  async cancelRecognition(): Promise<void> {},
  async batchOpPreview(op: BatchOpType, options: BatchOpOptions): Promise<BatchOpPreview> {
    const items = captures
      .filter((c) => options.formats.length === 0 || options.formats.includes(c.primaryFormat as ImageFormat))
      .slice(0, 3)
      .map((c) => ({
        path: c.primaryPath,
        targetPath: options.targetDir ? `${options.targetDir}/${c.baseName}` : null,
      }))
    return { op, count: items.length, items, siblingCount: 0 }
  },
  async batchOpExecute(_op: BatchOpType, options: BatchOpOptions): Promise<BatchOpResult> {
    const { count } = await this.batchOpPreview(_op, options)
    for (let done = 1; done <= count; done++) {
      await sleep(50)
      mockEmit('batch:progress', { done, total: count, currentPath: `${options.targetDir ?? ''}/item-${done}` })
    }
    mockEmit('batch:done', { success: count, failed: 0 })
    return { success: count, failed: 0, failures: [] }
  },
  async getAdjustments(path: string): Promise<AdjustParams> {
    return adjustments.get(path) ?? { exposure: 0, contrast: 0, saturation: 0 }
  },
  async setAdjustments(path: string, params: AdjustParams): Promise<void> {
    adjustments.set(path, params)
  },
  async getAppConfig(): Promise<AppConfig> {
    return {
      thumbnailSize: 220,
      favoriteDirs: [...favorites],
      lastDirectory: MOCK_DIR,
      recentDirectories: [...recent],
      theme: 'Light',
      leftPanelWidth: 180,
      rightPanelVisible: true,
      rightPanelWidth: 200,
      fontFamily: 'Segoe UI',
      recognitionThreadCount: 2,
    }
  },
}

/** mock 内部状态（收藏/最近/调整参数） */
let favorites: string[] = []
let recent: string[] = [MOCK_DIR]
const adjustments = new Map<string, AdjustParams>()

function sleep(ms: number): Promise<void> {
  const { promise, resolve } = Promise.withResolvers<void>()
  setTimeout(resolve, ms)
  return promise
}
