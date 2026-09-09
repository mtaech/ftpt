// 前端日志：console / 未捕获异常 → 后端统一日志管道（文件日志，与 Rust tracing 同管道）。
// 浏览器 mock 模式（无 Tauri 环境）下全部 no-op，不影响 mock 数据流。
// 安装：App.vue onMounted 调 installFrontendLogging()；业务代码可随时 logToFile()。
import { isTauri, logToBackend } from './ipc'

export type LogLevel = 'debug' | 'info' | 'warn' | 'error'

/** 写入一条前端日志（非 Tauri 环境 no-op；链路自身故障静默，不影响主流程） */
export function logToFile(level: LogLevel, message: string, context?: string) {
  if (!isTauri) return
  try {
    // 必须 catch：后端未就绪/退出中时 invoke 会 reject，未处理的拒绝会触发
    // window 'unhandledrejection' → 再记一条 → 再 reject（日志风暴）
    void logToBackend(level, message, context ?? null).catch(() => {})
  } catch {
    // 日志机制自身失败不抛出
  }
}

let installed = false

/** 安全序列化单个参数（Error → stack，对象 → JSON，其余 → String） */
function stringifyArg(arg: unknown): string {
  if (arg instanceof Error) return arg.stack ?? String(arg)
  if (typeof arg === 'string') return arg
  if (arg === null) return 'null'
  if (arg === undefined) return 'undefined'
  try {
    return typeof arg === 'object' ? (JSON.stringify(arg) ?? String(arg)) : String(arg)
  } catch {
    return String(arg)
  }
}

/**
 * 全局安装前端日志（幂等）：
 * - 拦截 console.debug/info/warn/error，转发到后端（保留原有控制台输出）
 * - window error / unhandledrejection → 后端 error 日志（带位置信息）
 * 在 App.vue onMounted 中调用；mock 模式（非 Tauri）不安装。
 */
export function installFrontendLogging() {
  if (installed) return
  installed = true
  if (!isTauri) return

  const wrap =
    (level: LogLevel) =>
    (original: (...args: any[]) => void) =>
    (...args: any[]) => {
      original(...args)
      logToFile(level, args.map(stringifyArg).join(' '), 'console')
    }
  console.debug = wrap('debug')(console.debug)
  console.info = wrap('info')(console.info)
  console.warn = wrap('warn')(console.warn)
  console.error = wrap('error')(console.error)

  window.addEventListener('error', (e) => {
    logToFile('error', `${e.message} @ ${e.filename ?? ''}:${e.lineno ?? 0}:${e.colno ?? 0}`, 'window:error')
  })
  window.addEventListener('unhandledrejection', (e) => {
    logToFile('error', `unhandledrejection: ${stringifyArg(e.reason)}`, 'window:unhandledrejection')
  })
}
