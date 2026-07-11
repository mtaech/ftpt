import { invoke } from '@tauri-apps/api/core'
import type {
  CaptureMeta, TreeNode, OpenDirectoryResult,
  ExifMetadata, XmpMetadata, AppConfig,
} from './index'

export const openDirectory = (path: string, sidecarExtensions: string[]) =>
  invoke<OpenDirectoryResult>('open_directory', { path, sidecarExtensions })

export const getDirectoryTree = () =>
  invoke<TreeNode[]>('get_directory_tree')

export const expandDirectory = (path: string) =>
  invoke<TreeNode[]>('expand_directory', { path })

export const getThumbnail = (path: string, size: number) =>
  invoke<number[]>('get_thumbnail', { path, size })

export const clearCache = () =>
  invoke<void>('clear_cache')

export const getCacheStats = () =>
  invoke<[number, number]>('get_cache_stats')

export const getExif = (path: string) =>
  invoke<ExifMetadata>('get_exif', { path })

export const readCaptureXmp = (primaryPath: string) =>
  invoke<XmpMetadata>('read_capture_xmp', { primaryPath })

export const writeCaptureXmp = (primaryPath: string, metadata: XmpMetadata) =>
  invoke<void>('write_capture_xmp', { primaryPath, metadata })

export const deleteCaptures = (capturePaths: string[][], mode: string) =>
  invoke<void>('delete_captures', { capturePaths, mode })

export const moveCaptures = (capturePaths: string[][], dest: string) =>
  invoke<void>('move_captures', { capturePaths, dest })

export const copyCaptures = (capturePaths: string[][], dest: string) =>
  invoke<void>('copy_captures', { capturePaths, dest })

export const renameCaptures = (items: Array<[string, string]>) =>
  invoke<void>('rename_captures', { items })

export const loadConfig = () =>
  invoke<AppConfig>('load_config')

export const saveConfig = (config: AppConfig) =>
  invoke<void>('save_config', { config })

export const convertImages = (paths: string[], options: {
  outputDir: string; outputFormat: string; jpegQuality: number; maxDimension: number
}) => invoke<string[]>('convert_images', { paths, options })

import { open, ask } from '@tauri-apps/plugin-dialog'

export const openFolderDialog = (title: string) =>
  open({ directory: true, multiple: false, title }) as Promise<string | null>

export const confirmDialog = (message: string, title: string) =>
  ask(message, { title, kind: 'warning' })

export const detectDrives = () =>
  invoke<string[]>('detect_drives')

export const importCaptures = (capturePaths: string[][], options: {
  destRoot: string; behavior: string; dateFormat: string; overwriteStrategy: string
}) => invoke<void>('import_captures', { paths: capturePaths, options })
