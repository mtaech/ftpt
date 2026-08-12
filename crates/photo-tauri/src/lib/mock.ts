// 浏览器纯 vite dev（无 __TAURI__）下的 mock 层：200 条假 CaptureMeta + SVG 占位图 +
// 模拟事件流，保证网格/预览/快捷键在无 Rust 后端时可开发。
import type {
  AdjustParams,
  AppConfig,
  BatchOpFailure,
  BatchOpItem,
  BatchOpOptions,
  BatchOpPreview,
  BatchOpResult,
  BatchOpType,
  CaptureMeta,
  ColorLabel,
  CorrectionStat,
  Flag,
  HistogramPayload,
  ImportCandidate,
  ImportDrive,
  ImportMode,
  ImportPlan,
  ImportResult,
  Recognition,
  SpeciesOverview,
  SpeciesPhoto,
  SubdirInfo,
  UndoBatchResult,
} from './bindings'
import { renderNameTemplate } from './nameTemplate'
import { formatToString } from './filter'

export const MOCK_DIR = 'E:/Mock/Birds'

/** 与 tauri-specta Result 模式命令同形状的返回（生成物 resolve 联合而不是 reject） */
type MockResult<T> = { status: 'ok'; data: T } | { status: 'error'; error: string }

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

/**
 * 堆叠样本：i=7 与 i=8 同 stem（DSC_1007 CR3 + DSC_1007 JPG）、i=9 与 i=10 同 stem
 * （DSC_1009 JPG + DSC_1009 CR3）——验证网格堆叠徽标（×2）/格式切换/预览切换按钮。
 * 其余 stem 唯一（每文件一个 Capture，JPG/RAW 不配对，见 CONTEXT.md）。
 */
const STACK_ALT_STEM: Record<number, string> = { 8: 'DSC_1007', 10: 'DSC_1009' }

/** 生成 200 条确定性的假数据（评分/色标/旗标部分预置，便于验证渲染） */
function makeCaptures(): CaptureMeta[] {
  const out: CaptureMeta[] = []
  for (let i = 0; i < 200; i++) {
    const isRaw = FORMATS[i % FORMATS.length] === 'Raw'
    const ext = isRaw ? RAWS[i % RAWS.length] : FORMATS[i % FORMATS.length].toUpperCase()
    const stem = STACK_ALT_STEM[i] ?? `DSC_${String(1000 + i)}`
    out.push({
      index: i,
      baseName: stem,
      primaryPath: `${MOCK_DIR}/${stem}.${ext.toLowerCase()}`,
      primaryFormat: isRaw ? 'raw' : ext.toLowerCase(),
      fileSize: 4_000_000 + ((i * 7919) % 20_000_000),
      // 前 6 张（1s 间隔）与 100–103（同秒）构成连拍组，供网格徽标/对比模式手动验证；
      // 其余按分钟递增：日期随 i/60 变化（非 i%9），避免 (日期, 分钟) 组合在 200 条内
      // 周期性碰撞（原 (i%9) 周期 180：i=6 与 i=186 时间戳相同，同组堆叠误并组）
      dateTaken:
        i < 6
          ? `2026-08-01T10:15:${String(28 + i).padStart(2, '0')}`
          : i >= 100 && i < 104
            ? '2026-08-02T10:40:00'
            : `2026-08-${String((Math.floor(i / 60) % 9) + 1).padStart(2, '0')}T10:${String(i % 60).padStart(2, '0')}:00`,
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
      // GPS：每 3 张有坐标（含南纬/西经负值样本），其余无——验证 GPS 行两态
      gpsLat: i % 3 === 0 ? 39.9042 + (i % 12) * 0.05 : null,
      gpsLon: i % 3 === 0 ? (i % 5 === 0 ? -(116.4074 + (i % 7) * 0.03) : 116.4074 + (i % 7) * 0.03) : null,
      rating: i % 7 === 0 ? 'Three' : 'None',
      colorLabel: i % 11 === 0 ? 'Red' : 'None',
      flag: i % 13 === 0 ? 'Pick' : null,
      // 预置少量关键词（确定性），便于手动验证 InfoPanel 关键词卡与筛选 chips
      keywords: i % 5 === 0 ? ['精选'] : i % 3 === 0 ? ['测试'] : [],
      birdName: null,
      birdConfidence: null,
      recognitionStatus: null,
      birdBbox: null,
      eyeSharpness: null,
      // 对焦点：mock 给部分样本一个确定性焦点（x 随 i 偏移），验证叠加层
      focusPoint: i % 4 === 0 ? { x: 0.3 + (i % 4) * 0.15, y: 0.4, width: 0, height: 0, shape: 'Point' } : null,
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

  async scanDirectory(path: string): Promise<number> {
    captures = makeCaptures()
    mockCurrentDir = path
    const total = captures.length
    // 与真实后端一致：最近目录列表更新（最新在前、去重）
    recent = [mockCurrentDir, ...recent.filter((r) => r !== mockCurrentDir)].slice(0, 10)
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
    setTimeout(() => mockEmit('scan:done', { total, directory: mockCurrentDir }), delay + 60)
    // 逐张缩略图就绪事件（抽 24 张模拟，验证 thumb:ready 刷新链路）
    for (let i = 0; i < 24; i++) {
      setTimeout(() => mockEmit('thumb:ready', { path: captures[i * 8].primaryPath }), delay + 150 + i * 80)
    }
    return total
  },

  async getCaptures(): Promise<CaptureMeta[]> {
    return captures
  },

  async setRating(paths: string[], rating: number): Promise<MockResult<null>> {
    const names = ['None', 'One', 'Two', 'Three', 'Four', 'Five'] as const
    for (const c of captures) if (paths.includes(c.primaryPath)) c.rating = names[rating] ?? 'None'
    return { status: 'ok', data: null }
  },

  async setFlag(paths: string[], flag: Flag | null): Promise<MockResult<null>> {
    for (const c of captures) if (paths.includes(c.primaryPath)) c.flag = flag
    return { status: 'ok', data: null }
  },

  async setColorLabel(paths: string[], label: ColorLabel | null): Promise<MockResult<null>> {
    for (const c of captures) if (paths.includes(c.primaryPath)) c.colorLabel = label ?? 'None'
    return { status: 'ok', data: null }
  },

  async setKeywords(paths: string[], keywords: string[]): Promise<MockResult<null>> {
    // 与后端 set_keywords 同语义：归一化（去空白/去空串/去重）后全量替换
    const seen = new Set<string>()
    const cleaned = keywords
      .map((k) => k.trim())
      .filter((k) => k !== '' && !seen.has(k) && (seen.add(k), true))
    for (const c of captures) if (paths.includes(c.primaryPath)) c.keywords = [...cleaned]
    return { status: 'ok', data: null }
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
  async listSubdirs(path: string): Promise<MockResult<SubdirInfo[]>> {
    return { status: 'ok', data: MOCK_SUBDIRS[path] ?? [] }
  },
  async listBirdSpecies(): Promise<MockResult<string[]>> {
    return {
      status: 'ok',
      data: ['北红尾鸲', '斑嘴鸭', '白鹭', '苍鹭', '翠鸟', '红嘴蓝鹊', '麻雀', '山斑鸠'],
    }
  },
  async recognizeCaptures(paths: string[]): Promise<MockResult<null>> {
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
    return { status: 'ok', data: null }
  },
  async cancelRecognition(): Promise<void> {},
  async batchOpPreview(op: BatchOpType, options: BatchOpOptions): Promise<MockResult<BatchOpPreview>> {
    const items = mockBatchPreviewItems(op, options)
    const siblingCount = options.syncSiblings ? items.length : 0
    return { status: 'ok', data: { op, count: items.length, items, siblingCount } }
  },
  async batchOpExecute(op: BatchOpType, options: BatchOpOptions): Promise<MockResult<BatchOpResult>> {
    const items = mockBatchPreviewItems(op, options)
    const count = items.length
    for (let done = 1; done <= count; done++) {
      await sleep(50)
      mockEmit('batch:progress', { done, total: count, currentPath: items[done - 1].path })
    }
    if (op === 'Delete') {
      captures = captures.filter((c) => !items.some((it) => it.path === c.primaryPath))
      mockEmit('scan:done', { total: captures.length, directory: mockCurrentDir })
    }
    // 撤销日志（对齐后端 OpJournal）：Move/Copy 记录最近一次批次，Delete 清空（回收站不可撤销）
    mockUndoJournal = op === 'Move' || op === 'Copy' ? { kind: op, count } : null
    mockEmit('batch:done', { success: count, failed: 0 })
    return { status: 'ok', data: { success: count, failed: 0, failures: [] } }
  },
  /**
   * 撤销最近一次批量操作（移动/复制）：mock 无真实文件系统，网格本就完整，
   * 语义上等价于「文件已回来」——返回撤销条数并触发 scan:done 全量重扫。
   */
  async undoBatchOperation(): Promise<MockResult<UndoBatchResult>> {
    const entry = mockUndoJournal
    if (!entry) return { status: 'error', error: '没有可撤销的批量操作' }
    mockUndoJournal = null
    mockEmit('scan:done', { total: captures.length, directory: mockCurrentDir })
    return { status: 'ok', data: { reverted: entry.count, skipped: [] } }
  },
  async getAdjustments(path: string): Promise<AdjustParams> {
    return adjustments.get(path) ?? { exposure: 0, contrast: 0, saturation: 0 }
  },
  async setAdjustments(path: string, params: AdjustParams): Promise<MockResult<null>> {
    adjustments.set(path, params)
    return { status: 'ok', data: null }
  },

  // ── 直方图 / 剪切叠加（T1 批次 HistogramPanel 切片）────────────────

  /** 路径哈希 → 确定性伪随机（mulberry32 风格；同一路径结果稳定） */
  async getHistogram(path: string): Promise<MockResult<HistogramPayload>> {
    let seed = 0
    for (let i = 0; i < path.length; i++) seed = (seed * 31 + path.charCodeAt(i)) | 0
    const rand = () => {
      seed = (seed + 0x6d2b79f5) | 0
      let t = Math.imul(seed ^ (seed >>> 15), 1 | seed)
      t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296
    }
    // 合成直方图：以 (中心, 峰值) 的高斯 + 噪声生成 4 通道，总和 = totalPixels
    const total = 8256 * 5504
    const mkChannel = (center: number, sigma: number): number[] => {
      const bins = new Array<number>(256).fill(0)
      let sum = 0
      for (let i = 0; i < 256; i++) {
        const d = (i - center) / sigma
        const v = Math.round(total * 0.35 * Math.exp(-0.5 * d * d) + total * 0.06 * rand())
        bins[i] = v
        sum += v
      }
      // 归一化使总和恰好等于 total（保持 bin 总数 = 像素数的不变量）
      const k = total / sum
      for (let i = 0; i < 256; i++) bins[i] = Math.round(bins[i] * k)
      return bins
    }
    const luma = mkChannel(110 + (seed % 60), 34)
    const r = mkChannel(95 + (seed % 50), 30)
    const g = mkChannel(120 + (seed % 55), 30)
    const b = mkChannel(70 + (seed % 65), 30)
    const clipHighCount = luma.slice(250).reduce((a, v) => a + v, 0)
    const clipLowCount = luma.slice(0, 6).reduce((a, v) => a + v, 0)
    return {
      status: 'ok',
      data: { luma, r, g, b, clipHighCount, clipLowCount, totalPixels: total },
    }
  },

  /** canvas 合成剪切叠加 PNG（上半红 = 高光、下半蓝 = 死黑，中间透明带），转字节返回 */
  async getClippingMask(_path: string): Promise<MockResult<number[]>> {
    const c = document.createElement('canvas')
    c.width = 400
    c.height = 300
    const ctx = c.getContext('2d')
    if (!ctx) return { status: 'ok', data: [] }
    ctx.clearRect(0, 0, 400, 300)
    ctx.fillStyle = 'rgba(255,0,0,1)'
    ctx.fillRect(0, 0, 400, 120)
    ctx.fillStyle = 'rgba(0,0,255,1)'
    ctx.fillRect(0, 180, 400, 120)
    const blob = await new Promise<Blob | null>((res) => c.toBlob(res, 'image/png'))
    if (!blob) return { status: 'ok', data: [] }
    const buf = new Uint8Array(await blob.arrayBuffer())
    return { status: 'ok', data: Array.from(buf) }
  },

  async getAppConfig(): Promise<AppConfig> {
    return { ...appConfig, favoriteDirs: [...favorites], recentDirectories: [...recent] }
  },

  // ── Phase 3 mock ───────────────────────────────────

  async getRecognition(path: string): Promise<MockResult<Recognition | null>> {
    const c = captures.find((x) => x.primaryPath === path)
    if (!c || !c.recognitionStatus) return { status: 'ok', data: null }
    return {
      status: 'ok',
      data: {
        status: c.recognitionStatus,
        bird: c.birdName ? { birdId: 1, cnName: c.birdName, latinName: 'Mockus birdus' } : null,
        classIndex: 0,
        confidence: c.birdConfidence ?? null,
        bbox: c.birdBbox ?? null,
        eyeSharpness: 0.72,
        eyeBbox: c.birdBbox ?? null,
        candidates: [],
        failureStage: 'None',
        recognizedAt: new Date().toISOString(),
      },
    }
  },
  async correctBird(path: string, birdName: string): Promise<MockResult<null>> {
    const c = captures.find((x) => x.primaryPath === path)
    if (c) {
      c.recognitionStatus = 'Confirmed'
      c.birdName = birdName
      c.birdConfidence = 1
    }
    return { status: 'ok', data: null }
  },
  async deleteCaptures(paths: string[]): Promise<MockResult<null>> {
    captures = captures.filter((c) => !paths.includes(c.primaryPath))
    // 与后端 delete_captures 语义一致：删除后 emit scan:done 驱动前端重扫
    mockEmit('scan:done', { total: captures.length, directory: mockCurrentDir })
    return { status: 'ok', data: null }
  },
  async exportAdjusted(path: string, outputDir: string | null): Promise<MockResult<string>> {
    return { status: 'ok', data: `${outputDir ?? ''}/${path.split(/[\\\\/]/).pop()}_adjusted.jpg` }
  },
  async listSystemFonts(): Promise<MockResult<string[]>> {
    return {
      status: 'ok',
      data: ['Segoe UI', 'Microsoft YaHei UI', 'Cascadia Mono', 'JetBrains Mono', 'Consolas'],
    }
  },
  async setAppConfig(config: AppConfig): Promise<MockResult<null>> {
    appConfig = config
    return { status: 'ok', data: null }
  },

  // ── T1 批次（SpeciesIndex）：全局鸟种统计 mock ──────────

  async getSpeciesStats(): Promise<MockResult<SpeciesOverview>> {
    return {
      status: 'ok',
      data: {
        folderCount: 2,
        stats: SPECIES_MOCK.map((s) => ({
          birdName: s.bird,
          photoCount: s.count,
          firstDate: s.dates[0],
          lastDate: s.dates[s.dates.length - 1],
          avgSharpness: s.sharp,
        })),
      },
    }
  },
  async getSpeciesPhotos(birdName: string): Promise<MockResult<SpeciesPhoto[]>> {
    return { status: 'ok', data: mockSpeciesPhotos(birdName) }
  },

  // ── T 批次 Wave 2（RecoFeedback）：修正审计 + 命中率 + 高频鸟种 mock ──

  async getCorrectionStats(): Promise<MockResult<CorrectionStat[]>> {
    return { status: 'ok', data: MOCK_CORRECTION_STATS }
  },
  async getFrequentSpecies(limit: number): Promise<MockResult<string[]>> {
    return { status: 'ok', data: mockFrequentSpecies(limit) }
  },

  // ── T1 批次（ImportRebuild）：导入 mock（假驱动器 + 假计划） ──

  async listImportDrives(): Promise<ImportDrive[]> {
    return [...MOCK_IMPORT_DRIVES]
  },
  async scanImportSource(path: string): Promise<MockResult<ImportCandidate[]>> {
    // 模拟递归扫描耗时
    await sleep(120)
    return { status: 'ok', data: mockImportCandidates(path) }
  },
  async planImport(candidates: ImportCandidate[], _destRoot: string): Promise<MockResult<ImportPlan>> {
    await sleep(60)
    return { status: 'ok', data: mockPlanImport(candidates) }
  },
  async executeImport(plan: ImportPlan, destRoot: string, _mode: ImportMode): Promise<MockResult<ImportResult>> {
    const files = plan.groups.flatMap((g) => g.files)
    const total = files.length
    let done = 0
    for (const file of files) {
      await sleep(40)
      done += 1
      mockEmit('import:progress', { done, total, current: file })
    }
    const imported = total
    const skipped = plan.skipped.length
    mockEmit('import:done', { imported, skipped, failed: 0 })
    // 目标根目录处于当前扫描目录内 → 模拟后端重扫（新导入照片刷新网格）
    if (destRoot === mockCurrentDir || destRoot.startsWith(`${mockCurrentDir}/`)) {
      mockEmit('scan:done', { total: captures.length, directory: mockCurrentDir })
    }
    return { status: 'ok', data: { imported, skipped, failed: 0 } }
  },

  // ── T1 批次（ExportPresets）：批量重命名（模板）+ 批量导出（预设） mock ──

  async batchRename(paths: string[], template: string, startSeq: number): Promise<MockResult<BatchOpResult>> {
    const failures: BatchOpFailure[] = []
    let success = 0
    const total = paths.length
    paths.forEach((path, i) => {
      const c = captures.find((x) => x.primaryPath === path)
      const base = renderNameTemplate(template, {
        name: c?.baseName ?? path.split(/[\\/]/).pop() ?? 'photo',
        species: c?.birdName ?? null,
        date: c?.dateTaken ?? null,
        camera: c?.cameraModel ?? null,
        seq: startSeq + i,
      })
      if (c) {
        // 模拟改名：更新内存 CaptureMeta（真实后端重扫后网格同样刷新）
        c.baseName = base
        c.primaryPath = c.primaryPath.replace(/[^\\/]+$/, `${base}${extOf(c.primaryPath)}`)
        success += 1
      } else {
        failures.push({ path, error: '找不到该照片' })
      }
      mockEmit('batch:progress', { done: i + 1, total, currentPath: path })
    })
    mockEmit('batch:done', { success, failed: failures.length })
    return { status: 'ok', data: { success, failed: failures.length, failures } }
  },

  async exportCaptures(
    paths: string[],
    _outputDir: string,
    _longEdge: number | null,
    _quality: number,
    template: string,
    startSeq: number,
  ): Promise<MockResult<BatchOpResult>> {
    const failures: BatchOpFailure[] = []
    const total = paths.length
    let success = 0
    for (let i = 0; i < total; i++) {
      const path = paths[i]
      const c = captures.find((x) => x.primaryPath === path)
      if (c) {
        success += 1
        // 模拟导出：渲染目标名放进进度事件（与后端 export_captures 语义一致）
        const rendered = renderNameTemplate(template, {
          name: c.baseName,
          species: c.birdName ?? null,
          date: c.dateTaken ?? null,
          camera: c.cameraModel ?? null,
          seq: startSeq + i,
        })
        mockEmit('export:progress', {
          done: i + 1,
          total,
          currentPath: `${path} → ${rendered}.jpg`,
        })
      } else {
        failures.push({ path, error: '找不到该照片' })
        mockEmit('export:progress', { done: i + 1, total, currentPath: path })
      }
    }
    mockEmit('export:done', { success, failed: failures.length })
    return { status: 'ok', data: { success, failed: failures.length, failures } }
  },
}

/** 取路径扩展名（含点；无扩展名返回空串） */
function extOf(p: string): string {
  const m = /(\.[^./\\]+)$/.exec(p)
  return m ? m[1] : ''
}

/** mock 内部状态（收藏/最近/调整参数/配置） */
let favorites: string[] = []
let recent: string[] = [MOCK_DIR]
const adjustments = new Map<string, AdjustParams>()

/** mock 撤销日志（对齐后端 OpJournal：只记最近一次 Move/Copy 批次，Delete 不记录） */
let mockUndoJournal: { kind: 'Move' | 'Copy'; count: number } | null = null

// ── T1 批次（SpeciesIndex）：全局鸟种统计 mock 数据 ────────
// 确定性合成：5 种鸟（张数降序，含无锐度样本）+ 照片指向 mock captures 路径
// （缩略图走 placeholderImage，无需真实文件）

const SPECIES_MOCK: { bird: string; count: number; sharp: number | null; dates: string[] }[] = [
  { bird: '白鹭', count: 6, sharp: 42.5, dates: ['2026-08-01', '2026-08-03'] },
  { bird: '翠鸟', count: 4, sharp: 61.2, dates: ['2026-08-02', '2026-08-05'] },
  { bird: '苍鹭', count: 3, sharp: null, dates: ['2026-08-01', '2026-08-02'] },
  { bird: '麻雀', count: 2, sharp: 33.0, dates: ['2026-08-04', '2026-08-04'] },
  { bird: '斑嘴鸭', count: 1, sharp: 55.5, dates: ['2026-08-06', '2026-08-06'] },
]

/** 某鸟种 mock 照片定位：与 makeCaptures 同构生成 primaryPath（ext 大小写一致），
 *  保证统计视图双击跳转能在 captures.items 中按主路径命中 */
function mockSpeciesPhotos(bird: string): SpeciesPhoto[] {
  const specIdx = SPECIES_MOCK.findIndex((s) => s.bird === bird)
  if (specIdx < 0) return []
  const spec = SPECIES_MOCK[specIdx]
  const out: SpeciesPhoto[] = []
  for (let i = 0; i < spec.count; i++) {
    const n = 1000 + ((specIdx * 13 + i * 7) % 200)
    const isRaw = FORMATS[n % FORMATS.length] === 'Raw'
    const ext = isRaw ? RAWS[n % RAWS.length] : FORMATS[n % FORMATS.length].toUpperCase()
    out.push({ folder: MOCK_DIR, relPath: `DSC_${String(n)}.${ext.toLowerCase()}` })
  }
  return out
}

// ── T 批次 Wave 2（RecoFeedback）：识别命中率 + 高频鸟种 mock 数据 ────────
// 与 SPECIES_MOCK 同口径：predicted = 该鸟种张数，correctedAway = 被人工改走张数。
// 含边界样本：翠鸟/苍鹭命中率偏低（模型弱项）、麻雀/斑嘴鸭样本 < 3 触发「样本少」。
const MOCK_CORRECTION_STATS: CorrectionStat[] = [
  { birdName: '斑嘴鸭', predictedCount: 1, correctedAwayCount: 0, accuracy: 1 },
  { birdName: '白鹭', predictedCount: 6, correctedAwayCount: 0, accuracy: 1 },
  { birdName: '苍鹭', predictedCount: 3, correctedAwayCount: 2, accuracy: 1 / 3 },
  { birdName: '翠鸟', predictedCount: 4, correctedAwayCount: 1, accuracy: 0.75 },
  { birdName: '麻雀', predictedCount: 2, correctedAwayCount: 2, accuracy: 0 },
]

/** mock 高频鸟种：SPECIES_MOCK 按张数降序（后端同序）；limit 截断 */
function mockFrequentSpecies(limit: number): string[] {
  return [...SPECIES_MOCK]
    .sort((a, b) => b.count - a.count)
    .slice(0, limit)
    .map((s) => s.bird)
}

// ── T1 批次（ImportRebuild）：导入 mock 数据与计划模拟 ────────

/** mock 可移动驱动器（假 SD 卡 / U 盘，列表入口可见） */
const MOCK_IMPORT_DRIVES: ImportDrive[] = [
  { path: 'E:\\', label: 'Canon EOS' },
  { path: 'F:\\', label: 'SANDISK' },
]

/**
 * mock 导入候选：确定性生成 4 个日期 × 每组 3-4 张（DCIM 形态路径）。
 * 每组首张带 `dup_` 前缀，模拟「目标已存在且大小相同」去重命中，
 * 便于验证干跑预览的跳过清单展示。
 */
function mockImportCandidates(source: string): ImportCandidate[] {
  const dates = ['2024-05-01', '2024-06-02', '2024-07-15', '2024-08-03']
  const perGroup = [4, 3, 4, 3]
  const out: ImportCandidate[] = []
  let seq = 1000
  dates.forEach((date, gi) => {
    for (let i = 0; i < perGroup[gi]; i++) {
      seq += 1
      const dup = i === 0 && gi > 0 ? 'dup_' : ''
      out.push({
        path: `${source}/DCIM/100CANON/${dup}IMG_${seq}.jpg`,
        date,
        size: 3_500_000 + seq,
      })
    }
  })
  return out
}

/** mock 计划：按日期分组（保持候选顺序）；dup_ 前缀 = 跳过（模拟目标去重命中） */
function mockPlanImport(candidates: ImportCandidate[]): ImportPlan {
  const groups: ImportPlan['groups'] = []
  const skipped: ImportPlan['skipped'] = []
  for (const c of candidates) {
    const name = c.path.split(/[\\/]/).pop() ?? c.path
    if (name.startsWith('dup_')) {
      skipped.push({ path: c.path, reason: '目标已存在且大小相同' })
      continue
    }
    let g = groups.find((x) => x.dateDir === c.date)
    if (!g) {
      g = { dateDir: c.date, files: [] }
      groups.push(g)
    }
    g.files.push(c.path)
  }
  return { groups, skipped }
}
/** mock 当前扫描目录（scanDirectory 按入参更新，scan:done 携带它供侧栏切换） */
let mockCurrentDir: string = MOCK_DIR
/** mock 目录树数据：MOCK_DIR 一层子目录 + 二级示例（验证侧栏懒加载逐层展开） */
const MOCK_SUBDIRS: Record<string, SubdirInfo[]> = {
  [MOCK_DIR]: [
    { name: '2026-07 春迁记录', path: `${MOCK_DIR}/2026-07 春迁记录`, photoCount: 42 },
    { name: '2026-06 繁殖季', path: `${MOCK_DIR}/2026-06 繁殖季`, photoCount: 18 },
    { name: 'RAW 归档', path: `${MOCK_DIR}/RAW 归档`, photoCount: 0 },
  ],
  [`${MOCK_DIR}/2026-07 春迁记录`]: [
    { name: '湿地', path: `${MOCK_DIR}/2026-07 春迁记录/湿地`, photoCount: 12 },
    { name: '林缘', path: `${MOCK_DIR}/2026-07 春迁记录/林缘`, photoCount: 30 },
  ],
}
let appConfig: AppConfig = {
  thumbnailSize: 220,
  favoriteDirs: [],
  lastDirectory: MOCK_DIR,
  recentDirectories: [MOCK_DIR],
  theme: 'Light',
  leftPanelWidth: 180,
  rightPanelVisible: true,
  rightPanelWidth: 200,
  fontFamily: 'Segoe UI',
  recognitionThreadCount: 2,
  includeSubdirectories: false,
  stackMode: 'ByTime',
  gridColumns: 4,
  uiScale: 100,
  exportPresets: [{ name: '原图', longEdge: null, quality: 95, template: '{name}' }],
}

function sleep(ms: number): Promise<void> {
  const { promise, resolve } = Promise.withResolvers<void>()
  setTimeout(resolve, ms)
  return promise
}

/**
 * mock 批量操作匹配：格式小写归一比较（mock primaryFormat 为小写 'jpeg'/'raw'，
 * ImageFormat 为大写枚举；真实后端 Display 为大写，前端归一后语义等价）。
 * syncSiblings=true 时额外带出同 stem 兄弟文件（模拟 expand_with_siblings）。
 */
function mockBatchPreviewItems(op: BatchOpType, options: BatchOpOptions): BatchOpItem[] {
  const fmtOk = (c: CaptureMeta) =>
    options.formats.length === 0 ||
    options.formats.some((f) => formatToString(f).toLowerCase() === c.primaryFormat.toLowerCase())
  const base = captures.filter((c) => fmtOk(c)).slice(0, 3)
  let items = base.map((c) => ({
    path: c.primaryPath,
    targetPath: op === 'Delete' ? null : options.targetDir ? `${options.targetDir}/${c.baseName}` : null,
  }))
  if (options.syncSiblings && base.length > 0) {
    const stem = base[0].baseName.replace(/\.[^.]+$/, '')
    const siblings = captures
      .filter((c) => c.baseName.startsWith(stem) && !items.some((it) => it.path === c.primaryPath))
      .slice(0, 2)
      .map((c) => ({
        path: c.primaryPath,
        targetPath: op === 'Delete' ? null : options.targetDir ? `${options.targetDir}/${c.baseName}` : null,
      }))
    items = [...items, ...siblings]
  }
  return items
}
