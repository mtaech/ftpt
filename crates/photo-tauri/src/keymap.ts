// 正式 keybinding 层：键位全集对齐 GPUI crates/photo-tool-app/src/ui/layout.rs 的
// on_key_down 匹配（(key, ctrl) 元组 + b 键的 Shift 分支），并补浏览器端需要的
// 焦点上下文隔离（表单控件聚焦时不触发）与修饰键精确匹配（Ctrl 组合不被吞）。
//
// 职责边界：本模块只做「按键 → action 名」的解析与分发，不碰任何 store；
// action 的真实实现由调用方（App.vue）通过 handlers 表注入。

/** 全部可分发动作名（对齐 GPUI Action 枚举；含 Phase 3 占位项） */
export type KeymapAction =
  // 评分（1–5 评分，0 清除）
  | 'rate1'
  | 'rate2'
  | 'rate3'
  | 'rate4'
  | 'rate5'
  | 'rate0'
  // 色标（6红 7黄 8绿 9蓝）
  | 'labelRed'
  | 'labelYellow'
  | 'labelGreen'
  | 'labelBlue'
  // 旗标（P/X 标记，U 清除）
  | 'flagPick'
  | 'flagReject'
  | 'flagNone'
  // 识别（B 单张 / Ctrl+B 批量未识别 / Ctrl+Shift+B 重新识别全部）
  | 'recognize'
  | 'recognizeUnrecognized'
  | 'recognizeAll'
  // 预览 / 视图
  | 'toggleBbox'
  | 'toggleGridPreview'
  // 导航（方向键 / Home / End）
  | 'prev'
  | 'next'
  | 'first'
  | 'last'
  // 文件操作
  | 'delete'
  // 选择（Ctrl+A 全选 / Ctrl+D 取消选择）
  | 'selectAll'
  | 'deselectAll'
  // 其他（Esc 退出预览 / F5 重扫 / Ctrl+[ Ctrl+] 面板开关）
  | 'closePreview'
  | 'refresh'
  | 'toggleLeftPanel'
  | 'toggleRightPanel'

/** 动作分发表：调用方把 action 名接到真实 store 调用上（Phase 3 项可先 no-op） */
export type KeymapHandlers = Partial<Record<KeymapAction, () => void>>

interface KeyBinding {
  /** e.key 规范化后的键名（见 normalizeKey） */
  key: string
  /** true=必须按 Ctrl；false=必须不按 Ctrl；缺省=不要求（对齐 GPUI 的 _ 通配） */
  ctrl?: boolean
  /** true=必须按 Shift；false=必须不按 Shift；缺省=不要求 */
  shift?: boolean
  action: KeymapAction
}

/**
 * 键位表：逐键移植 GPUI layout.rs 的 match (key, ctrl) 分支。
 * 注意 GPUI 的方向键为扁平 ±1 移动（display_order 下标），4 列网格下
 * 跨行是自然发生的，不做行内钳制——见 prev/next 的 App.vue 实现。
 */
const BINDINGS: readonly KeyBinding[] = [
  // 评分：1–5 评分，0 清除
  { key: '1', ctrl: false, action: 'rate1' },
  { key: '2', ctrl: false, action: 'rate2' },
  { key: '3', ctrl: false, action: 'rate3' },
  { key: '4', ctrl: false, action: 'rate4' },
  { key: '5', ctrl: false, action: 'rate5' },
  { key: '0', ctrl: false, action: 'rate0' },
  // 色标：6红 7黄 8绿 9蓝
  { key: '6', ctrl: false, action: 'labelRed' },
  { key: '7', ctrl: false, action: 'labelYellow' },
  { key: '8', ctrl: false, action: 'labelGreen' },
  { key: '9', ctrl: false, action: 'labelBlue' },
  // 旗标：P/X 标记，U 清除
  { key: 'p', ctrl: false, action: 'flagPick' },
  { key: 'x', ctrl: false, action: 'flagReject' },
  { key: 'u', ctrl: false, action: 'flagNone' },
  // 识别：B 单张 / Ctrl+B 批量未识别 / Ctrl+Shift+B 重新识别全部
  // （b 键的 ctrl/shift 精确匹配，保证 Ctrl 按下时不落到单张识别，不双重触发）
  { key: 'b', ctrl: false, action: 'recognize' },
  { key: 'b', ctrl: true, shift: false, action: 'recognizeUnrecognized' },
  { key: 'b', ctrl: true, shift: true, action: 'recognizeAll' },
  // 预览：V 检测框开关
  { key: 'v', ctrl: false, action: 'toggleBbox' },
  // 视图：G 网格/预览切换
  { key: 'g', ctrl: false, action: 'toggleGridPreview' },
  // 导航：左右方向键 / Home / End
  { key: 'left', ctrl: false, action: 'prev' },
  { key: 'right', ctrl: false, action: 'next' },
  { key: 'home', ctrl: false, action: 'first' },
  { key: 'end', ctrl: false, action: 'last' },
  // 删除：Delete（GPUI 不区分修饰键，(key, _) 通配）
  { key: 'delete', action: 'delete' },
  // 选择：Ctrl+A 全选 / Ctrl+D 取消选择
  { key: 'a', ctrl: true, action: 'selectAll' },
  { key: 'd', ctrl: true, action: 'deselectAll' },
  // 其他：Esc 退出预览（pendingBox 优先，见 App.vue）/ F5 重扫 / Ctrl+[ Ctrl+] 面板开关
  // （对齐 GPUI Action::ToggleLeftPanel / ToggleRightPanel，不再是目录后退前进）
  { key: 'escape', ctrl: false, action: 'closePreview' },
  { key: 'f5', ctrl: false, action: 'refresh' },
  { key: '[', ctrl: true, action: 'toggleLeftPanel' },
  { key: ']', ctrl: true, action: 'toggleRightPanel' },
]

/** 把浏览器 e.key 规范化为绑定表键名（对齐 GPUI keystroke 命名：left/right/home/end/escape/f5） */
function normalizeKey(key: string): string {
  switch (key) {
    case 'ArrowLeft':
      return 'left'
    case 'ArrowRight':
      return 'right'
    case 'ArrowUp':
      return 'up'
    case 'ArrowDown':
      return 'down'
    default:
      // 字母/数字统一小写（Shift+P 的 e.key 为 'P'，与普通 p 同键位）
      return key.toLowerCase()
  }
}

/** 修饰键判定：binding 未声明的修饰键不检查（对齐 GPUI 只读 control/shift；Alt/Meta 忽略） */
function modsOk(e: KeyboardEvent, b: KeyBinding): boolean {
  if (b.ctrl !== undefined && e.ctrlKey !== b.ctrl) return false
  if (b.shift !== undefined && e.shiftKey !== b.shift) return false
  return true
}

/** 按键 → 动作名（首个匹配生效；无匹配返回 null） */
function matchBinding(e: KeyboardEvent): KeymapAction | null {
  const key = normalizeKey(e.key)
  for (const b of BINDINGS) {
    if (b.key === key && modsOk(e, b)) return b.action
  }
  return null
}

/** 焦点上下文隔离：表单控件/可编辑区聚焦时不触发全局键位（用户输入优先） */
function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false
  const tag = target.tagName
  return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || target.isContentEditable
}

/**
 * 安装全局 keydown 监听并返回卸载函数。
 * 命中键位后先 preventDefault（阻止浏览器滚动/全选/刷新等默认行为）再分发；
 * 未命中或未注册 handler 时不拦截。
 */
export function installKeymap(handlers: KeymapHandlers): () => void {
  const onKeyDown = (e: KeyboardEvent) => {
    if (isEditableTarget(e.target)) return
    const action = matchBinding(e)
    if (!action) return
    const handler = handlers[action]
    if (!handler) return
    e.preventDefault()
    handler()
  }
  window.addEventListener('keydown', onKeyDown)
  return () => window.removeEventListener('keydown', onKeyDown)
}
