// 命名模板渲染的 TS 镜像单测（对照 Rust template.rs 测试边界移植）
import { describe, expect, it } from 'vitest'
import { normalizeDate, renderNameTemplate, sanitizeFilename, type NameTemplateContext } from './nameTemplate'

function ctx(name: string, extra: Partial<NameTemplateContext> = {}): NameTemplateContext {
  return { name, seq: 0, ...extra }
}

describe('renderNameTemplate', () => {
  it('渲染全部占位符（seq 补零 3 位）', () => {
    const c = ctx('DSC_0001', {
      species: '北红尾鸲',
      date: '2024:05:12 10:30:00',
      camera: 'NIKON Z9',
      seq: 7,
    })
    expect(renderNameTemplate('{name}_{species}_{date}_{seq}_{camera}', c)).toBe(
      'DSC_0001_北红尾鸲_20240512_007_NIKON Z9',
    )
  })

  it('缺失字段渲染为空串', () => {
    expect(renderNameTemplate('{species}-{date}-{camera}-{seq}', ctx('IMG_001'))).toBe('---000')
  })

  it('未知占位符与未闭合花括号原样保留', () => {
    expect(renderNameTemplate('a{foo}b', ctx('IMG_001'))).toBe('a{foo}b')
    expect(renderNameTemplate('a{b', ctx('IMG_001'))).toBe('a{b')
  })

  it('非法字符清洗 + 连续空白折叠', () => {
    expect(renderNameTemplate('a/b\\c:d*e?f"g<h>i|j', ctx('IMG_001'))).toBe('abcdefghij')
    expect(renderNameTemplate('  a   b\t\tc\n ', ctx('IMG_001'))).toBe('a b c')
  })

  it('尾部句点去除', () => {
    expect(renderNameTemplate('{name}.', ctx('IMG_001'))).toBe('IMG_001')
  })

  it('渲染为空 → fallback 原名', () => {
    expect(renderNameTemplate(':::   ', ctx('IMG_001'))).toBe('IMG_001')
    expect(renderNameTemplate('', ctx('IMG_001'))).toBe('IMG_001')
    // 模板只含缺失的 {species} → 空 → fallback
    expect(renderNameTemplate('{species}', ctx('IMG'))).toBe('IMG')
  })

  it('日期归一支持 EXIF/ISO/斜杠格式，乱码为空', () => {
    expect(normalizeDate('2023-11-08T09:15:00')).toBe('20231108')
    expect(normalizeDate('2023/11/08 09:15')).toBe('20231108')
    expect(normalizeDate('昨天拍的')).toBe('')
    expect(normalizeDate(null)).toBe('')
    expect(sanitizeFilename('  x  ')).toBe('x')
  })
})
