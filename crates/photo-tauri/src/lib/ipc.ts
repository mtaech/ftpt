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
} from './bindings'
import { mockCommands, mockListen, placeholderImage } from './mock'

/** 是否运行在 Tauri webview 内（false = 浏览器 mock 模式） */
export const isTauri =
  typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

const api = isTauri ? commands : mockCommands

// ── commands（契约 6 个） ─────────────────────────────

export const pickDirectory: () => Promise<string | null> = api.pickDirectory
export const scanDirectory: (path: string) => Promise<number> = api.scanDirectory
export const getCaptures: () => Promise<CaptureMeta[]> = api.getCaptures
export const setRating: (paths: string[], rating: number) => Promise<void> = api.setRating
export const setFlag: (paths: string[], flag: Flag | null) => Promise<void> = api.setFlag
export const setColorLabel: (paths: string[], label: ColorLabel | null) => Promise<void> =
  api.setColorLabel

// ── Phase 2/3 commands ──────────────────────────────

export const listFavorites: () => Promise<string[]> = api.listFavorites
export const addFavorite: (path: string) => Promise<void> = api.addFavorite
export const removeFavorite: (path: string) => Promise<void> = api.removeFavorite
export const listRecent: () => Promise<string[]> = api.listRecent
export const listBirdSpecies: () => Promise<string[]> = api.listBirdSpecies
export const recognizeCaptures: (paths: string[]) => Promise<void> = api.recognizeCaptures
export const cancelRecognition: () => Promise<void> = api.cancelRecognition
export const batchOpPreview: (
  op: BatchOpType,
  options: BatchOpOptions,
) => Promise<BatchOpPreview> = api.batchOpPreview
export const batchOpExecute: (
  op: BatchOpType,
  options: BatchOpOptions,
) => Promise<BatchOpResult> = api.batchOpExecute
export const getAdjustments: (path: string) => Promise<AdjustParams> = api.getAdjustments
export const setAdjustments: (path: string, params: AdjustParams) => Promise<void> =
  api.setAdjustments
export const getAppConfig: () => Promise<AppConfig> = api.getAppConfig

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
