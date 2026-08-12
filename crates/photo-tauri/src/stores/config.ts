// 应用配置 store：持有后端 AppConfig 快照，改动即保存（对齐 GPUI save_config「设置保存即时刷新」）。
// DOM 副作用（主题 html.dark class / 字体 --font-family-app 变量）集中在本 store，
// App.vue 启动恢复与设置弹窗共用同一份逻辑，避免两处各写一遍。
import { defineStore } from 'pinia'
import { getAppConfig, setAppConfig } from '@/lib/ipc'
import type { AppConfig } from '@/lib/bindings'

/** 默认配置（对齐 photo-config 默认：thumbnail 220 / Light / Segoe UI / 线程 2；index.html 不写死 dark，默认即亮色） */
const DEFAULT_CONFIG: AppConfig = {
  thumbnailSize: 220,
  favoriteDirs: [],
  lastDirectory: null,
  recentDirectories: [],
  theme: 'Light',
  leftPanelWidth: 180,
  rightPanelVisible: true,
  rightPanelWidth: 200,
  fontFamily: 'Segoe UI',
  recognitionThreadCount: 2,
  includeSubdirectories: false,
  stackMode: 'ByTime',
}

export const useConfigStore = defineStore('config', {
  state: () => ({
    /** 后端配置快照（load() 前为默认值；字段永不为空，各组件可直接读取） */
    config: { ...DEFAULT_CONFIG } as AppConfig,
    /** 是否已从后端成功加载（失败保持默认，不阻塞启动） */
    loaded: false,
  }),
  getters: {
    /** 主题（后端字段可缺省时回退 Light，对齐 index.html 亮色默认） */
    theme: (s) => s.config.theme ?? 'Light',
    /** 界面字体（回退 Segoe UI，与 style.css body 回退一致） */
    fontFamily: (s) => s.config.fontFamily ?? 'Segoe UI',
    /** 识别线程数（回退 2） */
    recognitionThreadCount: (s) => s.config.recognitionThreadCount ?? 2,
    /** 缩略图尺寸 px（回退 220，对齐 photo-config 默认） */
    thumbnailSize: (s) => s.config.thumbnailSize ?? 220,
    /** 扫描包含子目录开关（回退 false = 单层扫描，对齐 photo-config 默认） */
    includeSubdirectories: (s) => s.config.includeSubdirectories ?? false,
    /** 网格堆叠模式（回退 ByTime = 同组照片堆叠，对齐 photo-config 默认） */
    stackMode: (s) => s.config.stackMode ?? 'ByTime',
    /** 网格 cell 高度 = thumbnailSize + 56（对齐 GPUI grid.rs cell_size 公式） */
    rowHeight: (s) => (s.config.thumbnailSize ?? 220) + 56,
  },
  actions: {
    /** 把主题/字体即时应用到 DOM（启动恢复与设置改动共用） */
    applyDom() {
      document.documentElement.classList.toggle('dark', this.theme === 'Dark')
      document.documentElement.style.setProperty('--font-family-app', this.fontFamily)
    },
    /** 从后端拉取配置并应用到 DOM（启动 / 打开设置弹窗时调用；mock 或后端未就绪时保持默认） */
    async load() {
      try {
        this.config = await getAppConfig()
        this.loaded = true
      } catch {
        this.loaded = false
      }
      this.applyDom()
    },
    /** 局部更新：合并 → 即时应用 DOM → 后端保存（对齐 GPUI save_config 即时刷新语义） */
    async update(patch: Partial<AppConfig>) {
      this.config = { ...this.config, ...patch }
      this.applyDom()
      try {
        await setAppConfig(this.config)
        this.loaded = true
      } catch {
        // mock/后端未就绪：本地态仍生效，下次 load 会回读真实值
      }
    },
    /** 缩略图尺寸即时生效（仅本地，不持久化）：滑块拖动中调用，持久化由调用方去抖 */
    setThumbnailSize(size: number) {
      this.config = { ...this.config, thumbnailSize: size }
    },
  },
})
