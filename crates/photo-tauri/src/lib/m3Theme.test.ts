import { describe, expect, it } from 'vitest'
import { DEFAULT_ACCENT, normalizeAccentHex } from './m3Theme'

describe('normalizeAccentHex', () => {
  it('保留规范小写 #rrggbb', () => {
    expect(normalizeAccentHex('#d99a3c')).toBe('#d99a3c')
  })

  it('大写与无 # 前缀归一为小写 #rrggbb', () => {
    expect(normalizeAccentHex('D99A3C')).toBe('#d99a3c')
    expect(normalizeAccentHex('d99a3c')).toBe('#d99a3c')
  })

  it('非法值回退默认琥珀', () => {
    expect(normalizeAccentHex('#abc')).toBe(DEFAULT_ACCENT)
    expect(normalizeAccentHex('red')).toBe(DEFAULT_ACCENT)
    expect(normalizeAccentHex('')).toBe(DEFAULT_ACCENT)
  })

  it('null/undefined 回退默认琥珀', () => {
    expect(normalizeAccentHex(null)).toBe(DEFAULT_ACCENT)
    expect(normalizeAccentHex(undefined)).toBe(DEFAULT_ACCENT)
  })
})
