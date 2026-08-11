// IPC 薄封装：真实环境走 bindings.commands（tauri-specta typed invoke），
// 浏览器纯 vite dev 走 mock 层，组件/store 不感知差异。
import { listen } from '@tauri-apps/api/event'
import {
  commands,
  type AdjustParams,
  type AppConfig,
  type BatchOpOptions,
  type BatchOpPreview,
  type BatchOpResult,
  type BatchOpType,
  type CaptureMeta,
  type ColorLabel,
  type CorrectionStat,
  type Flag,
  type HistogramPayload,
  type ImportCandidate,
  type ImportDrive,
  type ImportMode,
  type ImportPlan,
  type ImportResult,
  type Recognition,
  type SpeciesOverview,
  type SpeciesPhoto,
  type SubdirInfo,
  type UndoBatchResult,
} from './bindings'
import { mockCommands, mockListen, placeholderImage } from './mock'

/** 是否运行在 Tauri webview 内（false = 浏览器 mock 模式） */
export const isTauri =
  typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

const api = isTauri ? commands : mockCommands

// ── 解包 tauri-specta Result 模式命令 ──────────────────────
// tauri-specta 默认 ErrorHandlingMode::Result：Result 命令的生成物 resolve
// 为 { status:'ok', data } | { status:'error', error }，而不是 reject。
// 这里解包回 Promise<T> + reject 形态，stores 的 try/catch 回滚语义无需改动。

/** 解包 Result<T, E> 命令：error 抛错（保留调用方 reject 契约），ok 返回 data */
async function unwrap<T>(
  p: Promise<{ status: 'ok'; data: T } | { status: 'error'; error: unknown }>,
): Promise<T> {
  const r = await p
  if (r.status === 'error') {
    throw new Error(typeof r.error === 'string' ? r.error : JSON.stringify(r.error))
  }
  return r.data
}

/** 解包 Result<(), E> 命令（Ok 侧生成物为 null）：统一映射为 void */
async function unwrapVoid<E>(
  p: Promise<{ status: 'ok'; data: null } | { status: 'error'; error: E }>,
): Promise<void> {
  await unwrap(p)
}

// ── commands（契约 6 个） ─────────────────────────────

export const pickDirectory: () => Promise<string | null> = () => api.pickDirectory()
export const scanDirectory: (path: string) => Promise<number> = (path) => api.scanDirectory(path)
export const getCaptures: () => Promise<CaptureMeta[]> = () => api.getCaptures()
export const setRating: (paths: string[], rating: number) => Promise<void> = (paths, rating) =>
  unwrapVoid(api.setRating(paths, rating))
export const setFlag: (paths: string[], flag: Flag | null) => Promise<void> = (paths, flag) =>
  unwrapVoid(api.setFlag(paths, flag))
export const setColorLabel: (paths: string[], label: ColorLabel | null) => Promise<void> = (
  paths,
  label,
) => unwrapVoid(api.setColorLabel(paths, label))
export const setKeywords: (paths: string[], keywords: string[]) => Promise<void> = (
  paths,
  keywords,
) => unwrapVoid(api.setKeywords(paths, keywords))

// ── Phase 2/3 commands ──────────────────────────────

export const listFavorites: () => Promise<string[]> = () => api.listFavorites()
export const addFavorite: (path: string) => Promise<void> = (path) => api.addFavorite(path)
export const removeFavorite: (path: string) => Promise<void> = (path) => api.removeFavorite(path)
export const listRecent: () => Promise<string[]> = () => api.listRecent()
export const listSubdirs: (path: string) => Promise<SubdirInfo[]> = (path) =>
  unwrap(api.listSubdirs(path))
export const listBirdSpecies: () => Promise<string[]> = () => unwrap(api.listBirdSpecies())
export const recognizeCaptures: (paths: string[]) => Promise<void> = (paths) =>
  unwrapVoid(api.recognizeCaptures(paths))
export const cancelRecognition: () => Promise<void> = () => api.cancelRecognition()
export const batchOpPreview: (
  op: BatchOpType,
  options: BatchOpOptions,
) => Promise<BatchOpPreview> = (op, options) => unwrap(api.batchOpPreview(op, options))
export const batchOpExecute: (
  op: BatchOpType,
  options: BatchOpOptions,
) => Promise<BatchOpResult> = (op, options) => unwrap(api.batchOpExecute(op, options))
/** 撤销最近一次批量操作（移动/复制；删除走回收站不在范围）。返回撤销/跳过统计 */
export const undoBatchOperation: () => Promise<UndoBatchResult> = () =>
  unwrap(api.undoBatchOperation())
export const getAdjustments: (path: string) => Promise<AdjustParams> = (path) =>
  api.getAdjustments(path)
export const setAdjustments: (path: string, params: AdjustParams) => Promise<void> = (
  path,
  params,
) => unwrapVoid(api.setAdjustments(path, params))
export const getAppConfig: () => Promise<AppConfig> = () => api.getAppConfig()

// ── 直方图 / 剪切叠加（T1 批次 HistogramPanel 切片）─────────────────

/** 计算直方图（预览尺寸解码；失败 reject，如 RAW 解码异常） */
export const getHistogram: (path: string) => Promise<HistogramPayload> = (path) =>
  unwrap(api.getHistogram(path))
/** 剪切叠加图 PNG 字节（红 = 高光溢出、蓝 = 死黑；前端转 Blob URL 叠放） */
export const getClippingMask: (path: string) => Promise<number[]> = (path) =>
  unwrap(api.getClippingMask(path))
export const batchRename: (
  paths: string[],
  template: string,
  startSeq: number,
) => Promise<BatchOpResult> = (paths, template, startSeq) =>
  unwrap(api.batchRename(paths, template, startSeq))
export const exportCaptures: (
  paths: string[],
  outputDir: string,
  longEdge: number | null,
  quality: number,
  template: string,
  startSeq: number,
) => Promise<BatchOpResult> = (paths, outputDir, longEdge, quality, template, startSeq) =>
  unwrap(api.exportCaptures(paths, outputDir, longEdge, quality, template, startSeq))

// ── Phase 3 commands ────────────────────────────────

export const getRecognition: (path: string) => Promise<Recognition | null> = (path) =>
  unwrap(api.getRecognition(path))
export const correctBird: (path: string, birdName: string) => Promise<void> = (path, birdName) =>
  unwrapVoid(api.correctBird(path, birdName))
export const deleteCaptures: (paths: string[]) => Promise<void> = (paths) =>
  unwrapVoid(api.deleteCaptures(paths))
export const exportAdjusted: (path: string, outputDir: string | null) => Promise<string> = (
  path,
  outputDir,
) => unwrap(api.exportAdjusted(path, outputDir))
export const listSystemFonts: () => Promise<string[]> = () => unwrap(api.listSystemFonts())
export const setAppConfig: (config: AppConfig) => Promise<void> = (config) =>
  unwrapVoid(api.setAppConfig(config))

// ── T1 批次（SpeciesIndex）：全局鸟种统计 ─────────────────

export const getSpeciesStats: () => Promise<SpeciesOverview> = () =>
  unwrap(api.getSpeciesStats())
export const getSpeciesPhotos: (birdName: string) => Promise<SpeciesPhoto[]> = (birdName) =>
  unwrap(api.getSpeciesPhotos(birdName))

// ── T 批次 Wave 2（RecoFeedback）：修正审计 + 命中率 + 高频鸟种 ──

/** 全库识别命中率（按鸟种聚合 predicted/correctedAway/accuracy） */
export const getCorrectionStats: () => Promise<CorrectionStat[]> = () =>
  unwrap(api.getCorrectionStats())
/** 高频鸟种（修正鸟种下拉「常用」分组；张数降序，本机使用频次即区域相关性代理） */
export const getFrequentSpecies: (limit: number) => Promise<string[]> = (limit) =>
  unwrap(api.getFrequentSpecies(limit))

// ── ptimg:// 自定义协议 URL ───────────────────────────

export type PtimgKind = 'thumb' | 'master' | 'full'

const isWindows = typeof navigator !== 'undefined' && navigator.userAgent.includes('Windows')

/**
 * 拼 ptimg 协议 URL。
 * Windows webview 实际请求形态为 http://ptimg.localhost/<kind>/<urlencoded路径>?v=<n>；
 * 其他平台为 ptimg://<kind>/...。v 为缓存破坏参数（thumb:ready 后递增强制刷新）。
 */
export function ptimgUrl(kind: PtimgKind, path: string, v?: number): string {
  if (!isTauri) return placeholderImage(kind, path)
  const encoded = encodeURIComponent(path)
  const query = v !== undefined ? `?v=${v}` : ''
  return isWindows
    ? `http://ptimg.localhost/${kind}/${encoded}${query}`
    : `ptimg://${kind}/${encoded}${query}`
}

// ── events（契约 4 个，app.emit 广播） ────────────────

export type ScanProgressPayload = { stage: 'scan' | 'exif' | 'thumb'; done: number; total: number }
export type ScanDonePayload = { total: number; directory: string }
export type CaptureEnrichedPayload = { indices: number[] }
export type ThumbReadyPayload = { path: string }

export type Unlisten = () => void

/** 统一事件订阅：mock 与真实 listen 同形态，返回退订函数 */
function listenEvent<T>(event: string, cb: (payload: T) => void): Promise<Unlisten> {
  if (!isTauri) return Promise.resolve(mockListen(event, (p) => cb(p as T)))
  return listen<T>(event, (e) => cb(e.payload))
}

export const onScanProgress = (cb: (p: ScanProgressPayload) => void) =>
  listenEvent<ScanProgressPayload>('scan:progress', cb)
export const onScanDone = (cb: (p: ScanDonePayload) => void) =>
  listenEvent<ScanDonePayload>('scan:done', cb)
export const onCaptureEnriched = (cb: (p: CaptureEnrichedPayload) => void) =>
  listenEvent<CaptureEnrichedPayload>('capture:enriched', cb)
export const onThumbReady = (cb: (p: ThumbReadyPayload) => void) =>
  listenEvent<ThumbReadyPayload>('thumb:ready', cb)

// ── Phase 2/3 events ────────────────────────────────

export type RecognizeProgressPayload = { done: number; total: number; currentPath: string }
export type RecognizeDonePayload = {
  total: number
  confirmed: number
  needsReview: number
  unrecognized: number
  failed: number
}
export type BatchProgressPayload = { done: number; total: number; currentPath: string }
export type BatchDonePayload = { success: number; failed: number }
export type ExportProgressPayload = { done: number; total: number; currentPath: string }
export type ExportDonePayload = { success: number; failed: number }

export const onRecognizeProgress = (cb: (p: RecognizeProgressPayload) => void) =>
  listenEvent<RecognizeProgressPayload>('recognize:progress', cb)
export const onRecognizeDone = (cb: (p: RecognizeDonePayload) => void) =>
  listenEvent<RecognizeDonePayload>('recognize:done', cb)
export const onBatchProgress = (cb: (p: BatchProgressPayload) => void) =>
  listenEvent<BatchProgressPayload>('batch:progress', cb)
export const onBatchDone = (cb: (p: BatchDonePayload) => void) =>
  listenEvent<BatchDonePayload>('batch:done', cb)
export const onExportProgress = (cb: (p: ExportProgressPayload) => void) =>
  listenEvent<ExportProgressPayload>('export:progress', cb)
export const onExportDone = (cb: (p: ExportDonePayload) => void) =>
  listenEvent<ExportDonePayload>('export:done', cb)

// ── Phase 4 commands：导入（ImportRebuild） ─────────────

export const listImportDrives: () => Promise<ImportDrive[]> = () => api.listImportDrives()
export const scanImportSource: (path: string) => Promise<ImportCandidate[]> = (path) =>
  unwrap(api.scanImportSource(path))
export const planImport: (
  candidates: ImportCandidate[],
  destRoot: string,
) => Promise<ImportPlan> = (candidates, destRoot) =>
  unwrap(api.planImport(candidates, destRoot))
export const executeImport: (
  plan: ImportPlan,
  destRoot: string,
  mode: ImportMode,
) => Promise<ImportResult> = (plan, destRoot, mode) =>
  unwrap(api.executeImport(plan, destRoot, mode))

// ── Phase 4 events：导入进度/完成 ──────────────────────

export type ImportProgressPayload = { done: number; total: number; current: string }
export type ImportDonePayload = { imported: number; skipped: number; failed: number }

export const onImportProgress = (cb: (p: ImportProgressPayload) => void) =>
  listenEvent<ImportProgressPayload>('import:progress', cb)
export const onImportDone = (cb: (p: ImportDonePayload) => void) =>
  listenEvent<ImportDonePayload>('import:done', cb)
