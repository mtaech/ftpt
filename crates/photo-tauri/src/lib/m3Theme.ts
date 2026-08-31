import { argbFromHex, hexFromArgb, DynamicScheme, Hct, Variant } from '@material/material-color-utilities'

/**
 * Material You 主题模块（观片灯箱世界）：seed 色 → HCT 生成 accent 色族，
 * 运行时写入 `document.documentElement` 的 CSS 变量（内联样式，优先于 style.css
 * 的静态兜底）。style.css 静态定义完整表面/中性/容器层，本模块**只重染 accent 一族**
 * （--primary/--primary-foreground/--ring/--accent-dim/--primary-container/
 * --on-primary-container），表面与中性色不随 seed 变——这是“灯箱固定中性底 + 仅
 * accent 由 seed 驱动”世界的核心。
 *
 * 亮/暗 accent 自适应：灯箱（亮色主形象）的主按钮白字必须 ≥5:1，故亮色下若 seed
 * 为亮色（如琥珀金 #d99a3c），自动压深一档（--primary 用深琥珀 #a8661d，
 * --primary-foreground 白）；暗色用原始亮 seed（作为暗底上的安全灯）。
 */

/** 默认 seed 色：琥珀金（灯箱标记 accent；暗色直接用，亮色自动压深） */
export const DEFAULT_ACCENT = '#d99a3c'

/** 亮色下主按钮白字需 ≥5:1，深琥珀 #a8661d 满足；当生成值与 seed 亮度冲突时使用 */
const LIGHT_PRIMARY_FALLBACK = '#a8661d'

/** 归一化 seed 色：trim + 去可选 `#`，匹配 6 位十六进制才返回小写 `#rrggbb`，否则默认琥珀 */
export function normalizeAccentHex(v: string | null | undefined): string {
  const t = (v ?? '').trim().replace(/^#/, '')
  return /^[0-9a-fA-F]{6}$/.test(t) ? `#${t.toLowerCase()}` : DEFAULT_ACCENT
}

/** CSS 变量名 → 取值函数（明暗共用，仅 accent 族；返回 hex 字符串） */
type AccentRole = 'primary' | 'primaryForeground' | 'ring' | 'accentDim' | 'primaryContainer' | 'onPrimaryContainer'

/** 由 seed 生成 M3 tonal spot scheme（明/暗），取 accent 族角色 */
function accentRolesFor(seedHex: string, dark: boolean): Record<AccentRole, string> {
  const scheme = new DynamicScheme({
    sourceColorHct: Hct.fromInt(argbFromHex(normalizeAccentHex(seedHex))),
    variant: Variant.TONAL_SPOT,
    contrastLevel: 0,
    isDark: dark,
  })
  const primary = hexFromArgb(scheme.primary as number)
  const ring = hexFromArgb(scheme.secondary as number)
  const primaryContainer = hexFromArgb(scheme.primaryContainer as number)
  const onPrimaryContainer = hexFromArgb(scheme.onPrimaryContainer as number)
  let p = primary
  let pFg = hexFromArgb(scheme.onPrimary as number)
  // 亮色主形象：主按钮需白字 ≥5:1，亮度偏亮的 seed 自动压深一档
  if (!dark && lightnessOf(primary) > 0.55) {
    p = LIGHT_PRIMARY_FALLBACK
    pFg = '#ffffff'
  }
  return {
    primary: p,
    primaryForeground: pFg,
    ring,
    accentDim: `color-mix(in srgb, ${primaryContainer} 45%, transparent)`,
    primaryContainer,
    onPrimaryContainer,
  }
}

/** 解析 #rrggbb 亮度贡献比例（0–1；仅用于亮色压深判定） */
function lightnessOf(hex: string): number {
  const m = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(hex)
  if (!m) return 0
  const r = parseInt(m[1], 16) / 255
  const g = parseInt(m[2], 16) / 255
  const b = parseInt(m[3], 16) / 255
  return 0.2126 * r + 0.7152 * g + 0.0722 * b
}

/** 只写 accent 族的 CSS 变量映射（表面/中性/容器层在 style.css 静态定义） */
const ACCENT_VAR_MAP: ReadonlyArray<readonly [string, AccentRole]> = [
  ['--primary', 'primary'],
  ['--primary-foreground', 'primaryForeground'],
  ['--ring', 'ring'],
  ['--accent-dim', 'accentDim'],
  ['--primary-container', 'primaryContainer'],
  ['--on-primary-container', 'onPrimaryContainer'],
]

/**
 * 应用观片灯箱主题：由 seed 色生成对应明/暗 accent 族，把 accent CSS 变量
 * 逐项写入 `document.documentElement` 内联样式。表面/中性色由 style.css 静态控制。
 */
export function applyMaterialTheme(seedHex: string, dark: boolean): void {
  const roles = accentRolesFor(seedHex, dark)
  const style = document.documentElement.style
  for (const [varName, role] of ACCENT_VAR_MAP) {
    style.setProperty(varName, roles[role])
  }
}
