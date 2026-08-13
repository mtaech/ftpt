import { describe, expect, it } from 'vitest'
import { DEFAULT_ACCENT, normalizeAccentHex } from './m3Theme'

describe('normalizeAccentHex', () => {
  it('保留规范小写 #rrggbb', () => {
    expect(normalizeAccentHex('#3b82f6')).toBe('#3b82f6')
  })

  it('大写与无 # 前缀归一为小写 #rrggbb', () => {
    expect(normalizeAccentHex('3B82F6')).toBe('#3b82f6')
  })

  it('非法值回退默认蓝', () => {
    expect(normalizeAccentHex('#abc')).toBe(DEFAULT_ACCENT)
    expect(normalizeAccentHex('red')).toBe(DEFAULT_ACCENT)
    expect(normalizeAccentHex('')).toBe(DEFAULT_ACCENT)
  })

  it('null/undefined 回退默认蓝', () => {
    expect(normalizeAccentHex(null)).toBe(DEFAULT_ACCENT)
    expect(normalizeAccentHex(undefined)).toBe(DEFAULT_ACCENT)
  })
})
