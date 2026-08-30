// 识别状态：单张/批量识别共用（后端统一 emit recognize:progress / recognize:done，
// 见 recognize_captures 注释「取消也照常 emit done」）。running 哨兵 + 进度 + 完成摘要；
// done 后触发 captures.reload() 回填识别结果（后端已 upsert 识别表并 enrich 内存）。
// StatusBar 消费进度/摘要，InfoPanel 识别卡与右键菜单消费 recognize(paths) 入口。
import { defineStore } from 'pinia'
import type { RecognizeDonePayload, RecognizeProgressPayload } from '@/lib/ipc'
import { cancelRecognition, correctRecognition, onRecognizeDone, onRecognizeProgress, recognizeCaptures } from '@/lib/ipc'
import { useCapturesStore } from './captures'

/** 空提示自动消失时长（ms） */
const NOTICE_MS = 3500

export const useRecognitionStore = defineStore('recognition', {
  state: () => ({
    /** 识别任务进行中（单张/批量共用，对齐 GPUI recognizing_single + batch_recognizing 守卫） */
    running: false,
    /** 最近一次识别进度（done/total/当前文件名） */
    progress: null as RecognizeProgressPayload | null,
    /** 最近一次识别完成摘要（确认/待复核/未检测/失败计数） */
    summary: null as RecognizeDonePayload | null,
    /** 空提示（无未识别照片等轻量提示；NOTICE_MS 后自动清空） */
    notice: null as string | null,
    /** 事件是否已接线（防重复 listen） */
    listening: false,
    // ── 人工纠错（SpeciesCorrectDialog）──
    /** 纠正对话框显隐（InfoPanel「纠正…」按钮 / 网格右键「纠正鸟种…」打开） */
    correctionOpen: false,
    /** 待纠正路径集合（打开时由入口传入；空 = 无目标） */
    correctionPaths: [] as string[],
    /** 纠正任务进行中（对话框提交防重复） */
    correcting: false,
    /** 纠正成功计数（InfoPanel 等以版本号驱动重拉完整结果，防旧结果串显——
     *  状态可能不变（Confirmed→Confirmed），status watcher 不会触发） */
    correctionVersion: 0,
  }),
  actions: {
    /** 事件接线：store 创建后调用一次 */
    init() {
      if (this.listening) return
      this.listening = true
      void onRecognizeProgress((p) => {
        this.progress = p
      })
      void onRecognizeDone((s) => {
        // done 即结束：复位哨兵 + 保存摘要 + 重拉全量回填（网格/InfoPanel 识别摘要）
        this.running = false
        this.progress = null
        this.summary = s
        void useCapturesStore().reload()
      })
    },

    /** 设置空提示（自动消失；重复设置重置计时） */
    setNotice(msg: string) {
      this.notice = msg
      clearTimeout(noticeTimer)
      noticeTimer = setTimeout(() => {
        noticeTimer = undefined
        this.notice = null
      }, NOTICE_MS)
    },

    /**
     * 批量识别指定路径（单张/多选/全部共用入口）。
     * 进行中拒绝并发（对齐 GPUI recognize_single 守卫 + 后端并发守卫）。
     */
    async recognize(paths: string[]) {
      if (this.running || paths.length === 0) return
      this.running = true
      this.progress = { done: 0, total: paths.length, currentPath: paths[0] }
      this.summary = null
      try {
        await recognizeCaptures(paths)
      } catch (e) {
        // 命令调用失败（非事件流）：复位哨兵，等效 GPUI worker 异常兜底
        this.running = false
        this.progress = null
        console.error('识别启动失败', e)
      }
    },

    /** 批量识别全部未识别照片（recognitionStatus == null 即从未识别） */
    recognizeUnrecognized() {
      const paths = useCapturesStore().items
        .filter((c) => c.recognitionStatus === null)
        .map((c) => c.primaryPath)
      if (paths.length === 0) {
        this.setNotice('当前目录没有未识别的照片')
        return
      }
      void this.recognize(paths)
    },

    /** 重新识别全部（Ctrl+Shift+B） */
    recognizeAll() {
      const paths = useCapturesStore().items.map((c) => c.primaryPath)
      if (paths.length === 0) return
      void this.recognize(paths)
    },

    /** 取消进行中识别（Esc / ✕ 按钮）：先本地复位，再通知后端置取消令牌 */
    async cancel() {
      if (!this.running) return
      this.running = false
      this.progress = null
      try {
        await cancelRecognition()
      } catch (e) {
        console.error('取消识别失败', e)
      }
    },

    /** 清空完成摘要/进度/空提示（StatusBar 摘要超时隐藏后调用） */
    reset() {
      this.summary = null
      this.progress = null
      this.notice = null
      clearTimeout(noticeTimer)
      noticeTimer = undefined
    },

    // ── 人工纠错（SpeciesCorrectDialog）──

    /** 打开纠正对话框（入口：InfoPanel「纠正…」按钮 / 网格右键「纠正鸟种…」；空集合 no-op） */
    openCorrection(paths: string[]) {
      if (paths.length === 0) return
      this.correctionPaths = [...paths]
      this.correctionOpen = true
    },

    /** 关闭纠正对话框并清空目标 */
    closeCorrection() {
      this.correctionOpen = false
      this.correctionPaths = []
    },

    /**
     * 批量人工纠正鸟种：调 correct_recognition 写 folder_db recognition +
     * global_db 修正日志；成功后本地同步 captures store 的识别摘要字段
     * （birdName / birdConfidence=100 / recognitionStatus=Confirmed，保留原 bbox 与
     * 眼锐度；对齐后端 enrich_with_recognition 语义），避免整库重拉。失败 rethrow，
     * 由对话框展示错误；`correcting` 哨兵在 finally 复位（防重复提交）。
     */
    async correct(paths: string[], spId: number, cnName: string, sciName: string) {
      if (this.correcting) return
      this.correcting = true
      try {
        await correctRecognition(paths, spId, cnName, sciName)
        const captures = useCapturesStore()
        for (const c of captures.items) {
          if (paths.includes(c.primaryPath)) {
            c.birdName = cnName
            c.birdConfidence = 100
            c.recognitionStatus = 'Confirmed'
          }
        }
        this.correctionVersion++
      } finally {
        this.correcting = false
      }
    },
  },
})

/** 空提示自动消失计时器（模块级：与 store 实例生命周期一致） */
let noticeTimer: number | undefined
