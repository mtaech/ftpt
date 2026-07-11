export interface CaptureMeta {
  index: number
  baseName: string
  primaryPath: string
  primaryFormat: string
  stackCount: number
  fileSize: number | null
  dateTaken: string | null
  hasXmp: boolean
  extensions: string[]
}

export interface TreeNode {
  path: string
  name: string
  isFavorite: boolean
  hasChildren: boolean
  children: TreeNode[]
}

export interface OpenDirectoryResult {
  captures: CaptureMeta[]
  tree: TreeNode[]
  totalCount: number
}

export interface ExifMetadata {
  camera: { make: string | null; model: string | null; lens: string | null }
  shooting: {
    exposureTime: string | null
    fNumber: string | null
    iso: number | null
    focalLength: string | null
    exposureCompensation: string | null
    whiteBalance: string | null
  }
  gps: {
    latitude: [number, number, number] | null
    longitude: [number, number, number] | null
    altitude: number | null
  }
  dateTimeOriginal: string | null
  imageWidth: number | null
  imageHeight: number | null
  fileSize: number | null
  colorSpace: string | null
  orientation: number | null
}

export interface XmpMetadata {
  rating: number
  colorLabel: string
  flag: string
}

export interface AppConfig {
  sidecarExtensions: string[]
  thumbnailSize: number
  favoriteDirs: string[]
  lastDirectory: string | null
  theme: string
  defaultDeleteMode: string
  importBehavior: string
  importDateFormat: string
  overwriteStrategy: string
  windowWidth: number
  windowHeight: number
  leftPanelWidth: number
  rightPanelVisible: boolean
  thumbnailCacheDir: string | null
  maxCacheSizeMb: number
}

export type SortBy = 'FileName' | 'DateTaken' | 'FileSize'
export type SortDirection = 'Ascending' | 'Descending'
export type DeleteMode = 'Trash' | 'Permanent'
