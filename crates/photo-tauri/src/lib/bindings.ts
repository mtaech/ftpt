// ⚠️ 手写 stub：仅供并行开发期使用。集成时由 A 侧 specta 导出测试生成物整体覆盖，
//    届时以真实生成文件为准修正类型引用。
// 类型形状对齐 crates/photo-domain/src/domain.rs 的 serde 输出（#[serde(rename_all = "camelCase")]，
// 枚举为外部标签：单元变体 = 变体名字符串，Raw(String) = { Raw: string }）。
import { invoke } from '@tauri-apps/api/core'

/** 图片格式（domain.rs ImageFormat） */
export type ImageFormat =
  | 'Jpeg'
  | 'Png'
  | 'Tiff'
  | 'Heif'
  | 'WebP'
  | 'Bmp'
  | 'Gif'
  | { Raw: string }
  | 'Other'

/** 评分（domain.rs Rating，serde 输出变体名） */
export type Rating = 'None' | 'One' | 'Two' | 'Three' | 'Four' | 'Five'

/** 颜色标签（domain.rs ColorLabel） */
export type ColorLabel = 'None' | 'Red' | 'Yellow' | 'Green' | 'Blue' | 'Purple'

/** Pick/Reject 旗标（domain.rs Flag） */
export type Flag = 'Pick' | 'Reject'

/** 识别状态（domain.rs RecognitionStatus；无记录 = 未识别，字段为 null） */
export type RecognitionStatus = 'Confirmed' | 'NeedsReview' | 'Unrecognized'

/** 检测框：归一化坐标 0–1（domain.rs BBox） */
export type BBox = { x1: number; y1: number; x2: number; y2: number }

/** 拍摄摘要（domain.rs CaptureMeta，camelCase） */
export type CaptureMeta = {
  index: number
  baseName: string
  primaryPath: string
  primaryFormat: string
  fileSize: number | null
  dateTaken: string | null
  extensions: string[]
  // EXIF 摘要（可延迟填充）
  cameraMake: string | null
  cameraModel: string | null
  lens: string | null
  exposureTime: string | null
  fNumber: string | null
  iso: number | null
  focalLength: string | null
  imageWidth: number | null
  imageHeight: number | null
  // 评分/色标/旗标
  rating: Rating
  colorLabel: ColorLabel
  flag: Flag | null
  // 识别摘要
  birdName: string | null
  birdConfidence: number | null
  recognitionStatus: RecognitionStatus | null
  birdBbox: BBox | null
}

// ── Phase 2/3 契约扩展（手写 stub，与 domain.rs serde 输出一致） ─────────────

/** 排序方式（domain.rs SortBy） */
export type SortBy = 'FileName' | 'DateTaken' | 'FileSize' | 'Rating' | 'Modified'

/** 排序方向（domain.rs SortDirection） */
export type SortDirection = 'Ascending' | 'Descending'

/** 识别状态筛选（domain.rs RecognitionFilter；NotRecognized = 从未识别） */
export type RecognitionFilter = 'All' | 'Confirmed' | 'NeedsReview' | 'Unrecognized' | 'NotRecognized'

/** 筛选条件（domain.rs FilterCriteria，camelCase；sort 单独成字段见 filter store） */
export type FilterCriteria = {
  formatFilter: ImageFormat | null
  birdNames: string[]
  dateFrom: string | null
  dateTo: string | null
  minRating: Rating | null
  colorLabel: ColorLabel | null
  flagFilter: Flag | null
  unflaggedFilter: boolean
  recognitionFilter: RecognitionFilter
}

/** 批量文件操作类型（domain.rs BatchOpType，ADR 0006：作用于当前筛选结果） */
export type BatchOpType = 'Copy' | 'Delete' | 'Move'

/** 批量操作选项：目标目录（Move/Copy 必填，Delete 忽略）+ 同步同名 + 格式集合 */
export type BatchOpOptions = {
  targetDir: string | null
  syncSiblings: boolean
  formats: ImageFormat[]
}

/** 干跑预览条目：path = 源文件，targetPath = 目标位置（Delete 为 null） */
export type BatchOpItem = { path: string; targetPath: string | null }

export type BatchOpPreview = {
  op: BatchOpType
  count: number
  items: BatchOpItem[]
  siblingCount: number
}

export type BatchOpFailure = { path: string; error: string }

export type BatchOpResult = { success: number; failed: number; failures: BatchOpFailure[] }

/** 调整参数（domain.rs AdjustParams，ADR 0007；全零 = 无调整，短路渲染路径） */
export type AdjustParams = { exposure: number; contrast: number; saturation: number }

/** 主题（photo-config Theme） */
export type Theme = 'Light' | 'Dark'

/** 应用配置（photo-config AppConfig，camelCase；GPUI 版默认 theme = Light） */
export type AppConfig = {
  thumbnailSize: number
  favoriteDirs: string[]
  lastDirectory: string | null
  recentDirectories: string[]
  theme: Theme
  leftPanelWidth: number
  rightPanelVisible: boolean
  rightPanelWidth: number
  fontFamily: string
  recognitionThreadCount: number
}

/** 契约 commands 的 typed invoke（tauri-specta 生成形态；snake_case 名经 tauri-specta 转 camelCase） */
export const commands = {
  /** 目录选择对话框，取消返回 null */
  async pickDirectory(): Promise<string | null> {
    return await invoke<string | null>('pick_directory')
  },
  /** 扫描目录：同步返回总数，EXIF/缩略图经事件异步推进 */
  async scanDirectory(path: string): Promise<number> {
    return await invoke<number>('scan_directory', { path })
  },
  /** 当前扫描结果全量下推 */
  async getCaptures(): Promise<CaptureMeta[]> {
    return await invoke<CaptureMeta[]>('get_captures')
  },
  /** 评分 0–5（0 = 清除） */
  async setRating(paths: string[], rating: number): Promise<void> {
    return await invoke<void>('set_rating', { paths, rating })
  },
  /** 旗标；null = 清除（U 键 FlagNone） */
  async setFlag(paths: string[], flag: Flag | null): Promise<void> {
    return await invoke<void>('set_flag', { paths, flag })
  },
  /** 色标；null = 清除 */
  async setColorLabel(paths: string[], label: ColorLabel | null): Promise<void> {
    return await invoke<void>('set_color_label', { paths, label })
  },
  /** 收藏目录列表（AppConfig.favoriteDirs） */
  async listFavorites(): Promise<string[]> {
    return await invoke<string[]>('list_favorites')
  },
  /** 添加收藏目录 */
  async addFavorite(path: string): Promise<void> {
    return await invoke<void>('add_favorite', { path })
  },
  /** 移除收藏目录 */
  async removeFavorite(path: string): Promise<void> {
    return await invoke<void>('remove_favorite', { path })
  },
  /** 最近打开目录（AppConfig.recentDirectories，扫描时自动记录） */
  async listRecent(): Promise<string[]> {
    return await invoke<string[]>('list_recent')
  },
  /** 名录全量鸟种（拼音排序，筛选下拉用） */
  async listBirdSpecies(): Promise<string[]> {
    return await invoke<string[]>('list_bird_species')
  },
  /** 批量识别（后台执行，事件 recognize:progress / recognize:done） */
  async recognizeCaptures(paths: string[]): Promise<void> {
    return await invoke<void>('recognize_captures', { paths })
  },
  /** 取消进行中的识别 */
  async cancelRecognition(): Promise<void> {
    return await invoke<void>('cancel_recognition')
  },
  /** 批量操作干跑（只算不动文件）；无筛选条件时应由前端禁用 */
  async batchOpPreview(op: BatchOpType, options: BatchOpOptions): Promise<BatchOpPreview> {
    return await invoke<BatchOpPreview>('batch_op_preview', { op, options })
  },
  /** 批量操作执行（事件 batch:progress / batch:done；Move/Delete 后前端全量重扫） */
  async batchOpExecute(op: BatchOpType, options: BatchOpOptions): Promise<BatchOpResult> {
    return await invoke<BatchOpResult>('batch_op_execute', { op, options })
  },
  /** 读取调整参数（无记录返回全零） */
  async getAdjustments(path: string): Promise<AdjustParams> {
    return await invoke<AdjustParams>('get_adjustments', { path })
  },
  /** 写入调整参数并触发预览刷新（后端经 ptimg master ?v= 变更，前端重拉 img） */
  async setAdjustments(path: string, params: AdjustParams): Promise<void> {
    return await invoke<void>('set_adjustments', { path, params })
  },
  /** 读取完整配置（启动时应用主题等） */
  async getAppConfig(): Promise<AppConfig> {
    return await invoke<AppConfig>('get_app_config')
  },
}
