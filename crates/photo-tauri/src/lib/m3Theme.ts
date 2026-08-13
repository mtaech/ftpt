import { argbFromHex, hexFromArgb, DynamicScheme, Hct, Variant } from '@material/material-color-utilities'

/**
 * Material You 主题模块：seed 色 → HCT 动态生成明暗整套色调板，运行时写入
 * document.documentElement 的 CSS 变量（内联样式，优先于 style.css 的
 * `:root`/`.dark` 静态兜底色块）。
 *
 * 色彩角色映射表见 applyMaterialTheme 内 COLOR_MAP；style.css 的静态色块
 * 保留为 pre-JS 兜底（JS 未运行时不至于无色）。
 *
 * 说明：0.4.0 的 `themeFromSourceColor()` 返回旧版 Scheme（仅 surface/
 * surfaceVariant，无 surface-container 层级角色），无法满足映射表的容器
 * 色调层级；改用同包现代 API `DynamicScheme` + `Variant.TONAL_SPOT`
 * （M3 默认变体，palette 由 sourceColorHct 自动派生），角色齐全、零新增依赖。
 */

/** 默认 seed 色：沿用现有蓝色 accent 身份 */
export const DEFAULT_ACCENT = '#3b82f6'

/** 归一化 seed 色：trim + 去可选 `#`，匹配 6 位十六进制才返回小写 `#rrggbb`，否则默认蓝 */
export function normalizeAccentHex(v: string | null | undefined): string {
  const t = (v ?? '').trim().replace(/^#/, '')
  return /^[0-9a-fA-F]{6}$/.test(t) ? `#${t.toLowerCase()}` : DEFAULT_ACCENT
}

/** CSS 变量名 → M3 scheme 字段（明暗共用同一张表，仅 scheme 来源不同） */
const COLOR_MAP: ReadonlyArray<readonly [string, keyof DynamicScheme]> = [
  ['--background', 'surfaceContainer'],
  ['--foreground', 'onSurface'],
  ['--card', 'surfaceContainerLowest'],
  ['--card-foreground', 'onSurface'],
  ['--popover', 'surfaceContainerHigh'],
  ['--popover-foreground', 'onSurface'],
  ['--primary', 'primary'],
  ['--primary-foreground', 'onPrimary'],
  ['--secondary', 'secondaryContainer'],
  ['--secondary-foreground', 'onSecondaryContainer'],
  ['--muted', 'surfaceContainer'],
  ['--muted-foreground', 'onSurfaceVariant'],
  ['--accent', 'surfaceContainerHigh'],
  ['--accent-foreground', 'onSurface'],
  ['--destructive', 'error'],
  ['--destructive-foreground', 'onError'],
  ['--border', 'outlineVariant'],
  ['--input', 'outline'],
  ['--ring', 'primary'],
  ['--sidebar', 'surfaceContainerLowest'],
  ['--sidebar-foreground', 'onSurface'],
  ['--sidebar-primary', 'primary'],
  ['--sidebar-primary-foreground', 'onPrimary'],
  ['--sidebar-accent', 'secondaryContainer'],
  ['--sidebar-accent-foreground', 'onSecondaryContainer'],
  ['--sidebar-border', 'outlineVariant'],
  ['--sidebar-ring', 'primary'],
  ['--element-background', 'surfaceContainerLow'],
  ['--element-hover', 'surfaceContainerHigh'],
  ['--element-active', 'surfaceContainerHighest'],
  ['--element-selected', 'secondaryContainer'],
  ['--chart-1', 'primary'],
  ['--chart-2', 'secondary'],
  ['--chart-3', 'tertiary'],
  ['--chart-4', 'error'],
  ['--chart-5', 'onSurfaceVariant'],
  ['--primary-container', 'primaryContainer'],
  ['--on-primary-container', 'onPrimaryContainer'],
  ['--secondary-container', 'secondaryContainer'],
  ['--on-secondary-container', 'onSecondaryContainer'],
  ['--tertiary', 'tertiary'],
  ['--on-tertiary', 'onTertiary'],
  ['--tertiary-container', 'tertiaryContainer'],
  ['--on-tertiary-container', 'onTertiaryContainer'],
  ['--error-container', 'errorContainer'],
  ['--on-error-container', 'onErrorContainer'],
  ['--surface-dim', 'surfaceDim'],
  ['--surface-bright', 'surfaceBright'],
  ['--surface-container-lowest', 'surfaceContainerLowest'],
  ['--surface-container-low', 'surfaceContainerLow'],
  ['--surface-container', 'surfaceContainer'],
  ['--surface-container-high', 'surfaceContainerHigh'],
  ['--surface-container-highest', 'surfaceContainerHighest'],
  ['--on-surface', 'onSurface'],
  ['--on-surface-variant', 'onSurfaceVariant'],
  ['--outline', 'outline'],
  ['--outline-variant', 'outlineVariant'],
  ['--inverse-surface', 'inverseSurface'],
  ['--inverse-on-surface', 'inverseOnSurface'],
  ['--inverse-primary', 'inversePrimary'],
  ['--surface-tint', 'surfaceTint'],
  ['--scrim', 'scrim'],
  ['--shadow', 'shadow'],
]

/** 由 seed 生成 M3 tonal spot scheme（明/暗） */
function schemeFor(seedHex: string, dark: boolean): DynamicScheme {
  return new DynamicScheme({
    sourceColorHct: Hct.fromInt(argbFromHex(normalizeAccentHex(seedHex))),
    variant: Variant.TONAL_SPOT,
    contrastLevel: 0,
    isDark: dark,
  })
}

/**
 * 应用 Material You 主题：由 seed 色生成对应明/暗 scheme，把色彩映射表
 * 逐项写入 `document.documentElement` 内联 CSS 变量。
 */
export function applyMaterialTheme(seedHex: string, dark: boolean): void {
  const scheme = schemeFor(seedHex, dark)
  const style = document.documentElement.style
  for (const [varName, field] of COLOR_MAP) {
    // COLOR_MAP 均为颜色角色 getter（number）；DynamicScheme 上还有 isDark 等
    // 非色字段，keyof 无法窄化，此处按表断言为 number
    style.setProperty(varName, hexFromArgb(scheme[field] as number))
  }
}
