// 导出对话框状态（T1 批次：导出预设 + 命名模板）：
// - 预设管理：CRUD 持久化到 AppConfig.exportPresets（photo-config TOML/SQLite）
// - 批量导出：exportCaptures command（长边/质量/模板），export:progress/done 事件驱动进度
// - 实时预览：renderNameTemplate（TS 镜像 template.rs）渲染第一张待导出照片
import { defineStore } from 'pinia'
import type { BatchOpResult, ExportPreset } from '@/lib/bindings'
import {
  exportCaptures,
  getAppConfig,
  onExportDone,
  onExportProgress,
  pickDirectory,
  setAppConfig,
  type ExportDonePayload,
  type ExportProgressPayload,
} from '@/lib/ipc'
import { renderNameTemplate } from '@/lib/nameTemplate'
import { useCapturesStore } from './captures'

export const useExportStore = defineStore('export', {
  state: () => ({
    /** 对话框显隐 */
    open: false,
    /** 待导出路径集合（打开时由入口传入，顺序即序号顺序） */
    paths: [] as string[],
    /** 预设列表（来自 getAppConfig().exportPresets） */
    presets: [] as ExportPreset[],
    /** 当前选中预设下标（-1 = 新建/自定义，未关联预设） */
    presetIndex: -1,
    /** 预设名输入框（新建预设时填写；选中预设时为只读展示名） */
    presetName: '',
    /** 长边像素输入（空 = 原尺寸 None；解析失败/≤0 同样视为原尺寸） */
    longEdge: '',
    /** JPEG 质量（1-100 滑杆） */
    quality: 95,
    /** 命名模板（占位符 {name}/{species}/{date}/{seq}/{camera}） */
    template: '{name}',
    /** 目标目录（导出必填） */
    targetDir: null as string | null,
    /** 执行中（进度弹窗显示依据） */
    running: false,
    /** 导出进度（export:progress 事件） */
    progress: null as ExportProgressPayload | null,
    /** 最近一次执行结果 */
    result: null as BatchOpResult | null,
    /** 面板 toast（组件负责 4s 消失） */
    toast: null as string | null,
    /** 事件是否已接线 */
    listening: false,
  }),
  getters: {
    /** 长边 → 后端参数（空/非法 → None） */
    longEdgeParam(state): number | null {
      const n = Number(state.longEdge)
      return Number.isFinite(n) && n > 0 ? Math.round(n) : null
    },
    /** 钳制后的质量（1-100） */
    qualityParam(state): number {
      return Math.min(100, Math.max(1, Math.round(state.quality)))
    },
    /** 实时预览：第一张待导出照片的渲染基名（无路径/空模板时为空） */
    previewName(state): string {
      if (!state.open || state.paths.length === 0 || !state.template.trim()) return ''
      const meta = useCapturesStore().items.find((m) => m.primaryPath === state.paths[0])
      return renderNameTemplate(state.template, {
        name: meta?.baseName ?? 'photo',
        species: meta?.birdName ?? null,
        date: meta?.dateTaken ?? null,
        camera: meta?.cameraModel ?? null,
        seq: 1,
      })
    },
    /** 进度弹窗文案（n/m · 当前文件） */
    progressText(state): string {
      const p = state.progress
      if (!p) return ''
      return `${p.done}/${p.total} · ${p.currentPath.split(/[\\/]/).pop() ?? p.currentPath}`
    },
  },
  actions: {
    /** 事件接线：导出进度（防重复 listen，对齐 batch store init 模式） */
    init() {
      if (this.listening) return
      this.listening = true
      void onExportProgress((p) => {
        this.progress = p
      })
      void onExportDone((p: ExportDonePayload) => {
        // 完成汇总（事件与 invoke 返回双通道，这里只更新状态供进度弹窗收尾）
        this.progress = { done: p.success + p.failed, total: p.success + p.failed, currentPath: '' }
      })
    },

    /** 打开导出对话框：传入待导出主路径集合，加载预设并套用首个 */
    async openDialog(paths: string[]) {
      this.paths = [...paths]
      this.result = null
      this.progress = null
      this.running = false
      try {
        const cfg = await getAppConfig()
        this.presets = cfg.exportPresets ?? []
      } catch {
        this.presets = []
      }
      this.presetIndex = this.presets.length > 0 ? 0 : -1
      this.applyPreset(this.presetIndex)
      this.open = true
    },

    closeDialog() {
      this.open = false
    },

    /** 选中预设：套用其字段（未选中时保持当前表单） */
    applyPreset(i: number) {
      this.presetIndex = i
      const p = this.presets[i]
      if (!p) return
      this.presetName = p.name ?? ''
      this.longEdge = p.longEdge ? String(p.longEdge) : ''
      this.quality = p.quality ?? 95
      this.template = p.template ?? '{name}'
    },

    /** 新建预设：脱离当前选中，清空表单（保留常用默认） */
    newPreset() {
      this.presetIndex = -1
      this.presetName = ''
      this.longEdge = ''
      this.quality = 95
      this.template = '{name}_{seq}'
    },

    /** 保存当前表单到预设（同名覆盖，否则新建）；持久化到 AppConfig */
    async savePreset() {
      const name = this.presetName.trim()
      if (!name) {
        this.toast = '请输入预设名称'
        return
      }
      const preset: ExportPreset = {
        name,
        longEdge: this.longEdgeParam,
        quality: this.qualityParam,
        template: this.template,
      }
      const idx = this.presets.findIndex((p) => p.name === name)
      if (idx >= 0) this.presets[idx] = preset
      else this.presets.push(preset)
      this.presetIndex = this.presets.findIndex((p) => p.name === name)
      await this.persistPresets()
      this.toast = `预设「${name}」已保存`
    },

    /** 删除当前选中预设（至少保留一个时不置空选中；持久化） */
    async deletePreset() {
      if (this.presetIndex < 0) return
      const [removed] = this.presets.splice(this.presetIndex, 1)
      this.presetIndex = this.presets.length > 0 ? 0 : -1
      this.applyPreset(this.presetIndex)
      await this.persistPresets()
      if (removed) this.toast = `预设「${removed.name ?? ''}」已删除`
    },

    /** 预设列表落盘（保留其余 AppConfig 字段） */
    async persistPresets() {
      try {
        const cfg = await getAppConfig()
        await setAppConfig({ ...cfg, exportPresets: this.presets })
      } catch (e) {
        this.toast = `预设保存失败：${String(e)}`
      }
    },

    /** 一步式选择目标目录 */
    async chooseTarget() {
      const dir = await pickDirectory()
      if (dir) this.targetDir = dir
    },

    /** 执行批量导出：exportCaptures（spawn_blocking 后端逐张渲染） */
    async runExport() {
      if (this.running) return
      if (this.paths.length === 0) {
        this.toast = '没有可导出的照片'
        return
      }
      if (!this.targetDir) {
        this.toast = '请先选择目标目录'
        return
      }
      this.running = true
      this.result = null
      this.progress = { done: 0, total: this.paths.length, currentPath: '' }
      try {
        const result = await exportCaptures(
          this.paths,
          this.targetDir,
          this.longEdgeParam,
          this.qualityParam,
          this.template,
          1,
        )
        this.result = result
        this.toast = `导出完成：成功 ${result.success} / 失败 ${result.failed}`
      } catch (e) {
        this.toast = `导出失败：${String(e)}`
      } finally {
        this.running = false
      }
    },
  },
})
