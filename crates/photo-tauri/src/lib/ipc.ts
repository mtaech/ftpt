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
  type Flag,
  type Recognition,
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

// ── Phase 2/3 commands ──────────────────────────────

export const listFavorites: () => Promise<string[]> = () => api.listFavorites()
export const addFavorite: (path: string) => Promise<void> = (path) => api.addFavorite(path)
export const removeFavorite: (path: string) => Promise<void> = (path) => api.removeFavorite(path)
export const listRecent: () => Promise<string[]> = () => api.listRecent()
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
export const getAdjustments: (path: string) => Promise<AdjustParams> = (path) =>
  api.getAdjustments(path)
export const setAdjustments: (path: string, params: AdjustParams) => Promise<void> = (
  path,
  params,
) => unwrapVoid(api.setAdjustments(path, params))
export const getAppConfig: () => Promise<AppConfig> = () => api.getAppConfig()

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

export const onRecognizeProgress = (cb: (p: RecognizeProgressPayload) => void) =>
  listenEvent<RecognizeProgressPayload>('recognize:progress', cb)
export const onRecognizeDone = (cb: (p: RecognizeDonePayload) => void) =>
  listenEvent<RecognizeDonePayload>('recognize:done', cb)
export const onBatchProgress = (cb: (p: BatchProgressPayload) => void) =>
  listenEvent<BatchProgressPayload>('batch:progress', cb)
export const onBatchDone = (cb: (p: BatchDonePayload) => void) =>
  listenEvent<BatchDonePayload>('batch:done', cb)
