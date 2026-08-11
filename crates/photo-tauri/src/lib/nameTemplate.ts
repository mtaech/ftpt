// 命名模板渲染（TS 侧镜像，供导出对话框/重命名区实时预览）。
// 与 crates/photo-engine/src/template.rs 的 render_name_template 语义对齐：
// 占位符 {name}/{species}/{date}/{seq}/{camera}；未知占位符原样保留；
// 结果文件名清洗（去除 / \ : * ? " < > | 与控制字符，连续空白折叠）；
// 渲染后为空 → fallback 原名。

export interface NameTemplateContext {
  /** 原名（无扩展名） */
  name: string
  /** 鸟种名（无则空串） */
  species?: string | null
  /** 拍摄日期（EXIF date_time_original 原文，渲染时归一为 YYYYMMDD） */
  date?: string | null
  /** 相机型号 */
  camera?: string | null
  /** 序号（补零 3 位） */
  seq: number
}

/** 把 EXIF/ISO 日期字符串归一为 YYYYMMDD；格式不符或缺失 → 空串 */
export function normalizeDate(date: string | null | undefined): string {
  if (!date) return ''
  const s = date.trim()
  // 前 10 字节必须全 ASCII（数字/分隔符），避免多字节字符切片错乱
  if (s.length < 10 || !/^[\x00-\x7f]{10}/.test(s)) return ''
  const year = s.slice(0, 4)
  const month = s.slice(5, 7)
  const day = s.slice(8, 10)
  const sepOk = /[:/-]/.test(s[4]) && /[:/-]/.test(s[7])
  if (sepOk && /^\d{4}$/.test(year) && /^\d{2}$/.test(month) && /^\d{2}$/.test(day)) {
    return `${year}${month}${day}`
  }
  return ''
}

/** 文件名清洗：去除非法字符与控制字符，连续空白折叠，去首尾空白/尾部句点 */
export function sanitizeFilename(name: string): string {
  const out: string[] = []
  let prevSpace = false
  for (const c of name) {
    if (/[/\\:*?"<>|]/.test(c)) continue
    if (/\s/.test(c)) {
      if (!prevSpace && out.length > 0) out.push(' ')
      prevSpace = true
      continue
    }
    if (c.charCodeAt(0) < 0x20) continue // 控制字符
    prevSpace = false
    out.push(c)
  }
  let s = out.join('')
  while (s.endsWith(' ') || s.endsWith('.')) s = s.slice(0, -1)
  return s
}

/** 渲染命名模板（文件基名，不含扩展名） */
export function renderNameTemplate(template: string, ctx: NameTemplateContext): string {
  let out = ''
  let rest = template
  for (;;) {
    const pos = rest.indexOf('{')
    if (pos < 0) {
      out += rest
      break
    }
    out += rest.slice(0, pos)
    const after = rest.slice(pos + 1)
    const end = after.indexOf('}')
    if (end < 0) {
      // 无闭合花括号：按字面输出
      out += '{' + after
      break
    }
    const key = after.slice(0, end)
    switch (key) {
      case 'name':
        out += ctx.name
        break
      case 'species':
        out += ctx.species ?? ''
        break
      case 'date':
        out += normalizeDate(ctx.date)
        break
      case 'camera':
        out += ctx.camera ?? ''
        break
      case 'seq':
        out += String(ctx.seq).padStart(3, '0')
        break
      default:
        // 未知占位符原样保留（含花括号）
        out += '{' + key + '}'
        break
    }
    rest = after.slice(end + 1)
  }
  const cleaned = sanitizeFilename(out)
  return cleaned.length > 0 ? cleaned : ctx.name
}
