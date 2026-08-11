//! 命名模板渲染（T1 批次：导出预设 / 批量重命名占位符）。
//!
//! 模板语法：`{name}` 原名(无扩展)、`{species}` 鸟种名(无则空)、`{date}` 拍摄日期
//! YYYYMMDD、`{seq}` 序号(起始可配,补零 3 位)、`{camera}` 相机型号。
//! 未知占位符原样保留；渲染结果做文件名清洗（去除 `/ \ : * ? " < > |` 与控制字符，
//! 连续空白折叠）；模板渲染后为空 → fallback 原名。
//!
//! 全同步纯函数模块，无 IO、无外部依赖。

/// 命名模板渲染上下文（单条拍摄的元数据 + 序号）
#[derive(Debug, Clone, Default)]
pub struct NameTemplateContext {
    /// 原名（无扩展名）
    pub name: String,
    /// 鸟种名（识别确认后；未识别/无记录时为空串）
    pub species: Option<String>,
    /// 拍摄日期（EXIF `date_time_original` 原文，渲染时归一为 YYYYMMDD）
    pub date: Option<String>,
    /// 相机型号（EXIF model）
    pub camera: Option<String>,
    /// 序号（起始可配，补零 3 位）
    pub seq: u32,
}

/// 渲染命名模板。结果作为文件基名（不含扩展名），由调用方拼接扩展名。
///
/// - 占位符逐字替换；`{seq}` 补零 3 位
/// - 未知占位符（如 `{foo}`）原样保留（含花括号）
/// - 未闭合的 `{` 按字面输出
/// - 结果经 [`sanitize_filename`] 清洗；清洗后为空 → 返回 `ctx.name` 兜底
pub fn render_name_template(template: &str, ctx: &NameTemplateContext) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some(pos) = rest.find('{') {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + 1..];
        match after.find('}') {
            Some(end) => {
                let key = &after[..end];
                match key {
                    "name" => out.push_str(&ctx.name),
                    "species" => out.push_str(ctx.species.as_deref().unwrap_or("")),
                    "date" => out.push_str(&normalize_date(ctx.date.as_deref())),
                    "camera" => out.push_str(ctx.camera.as_deref().unwrap_or("")),
                    "seq" => out.push_str(&format!("{:03}", ctx.seq)),
                    // 未知占位符：原样保留（含花括号），避免吞掉用户自写文本
                    _ => {
                        out.push('{');
                        out.push_str(key);
                        out.push('}');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                // 无闭合花括号：`{` 连同其后文本按字面输出
                out.push('{');
                out.push_str(after);
                rest = "";
            }
        }
    }
    out.push_str(rest);

    let cleaned = sanitize_filename(&out);
    if cleaned.is_empty() {
        ctx.name.clone()
    } else {
        cleaned
    }
}

/// 把 EXIF/ISO 日期字符串归一为 `YYYYMMDD`：
/// 取前 10 个字符，按 `YYYY[:/-]MM[:/-]DD` 校验；格式不符或缺失 → 空串。
fn normalize_date(date: Option<&str>) -> String {
    let Some(s) = date else {
        return String::new();
    };
    let s = s.trim();
    // 前 10 字节必须全 ASCII（数字/分隔符），否则直接判空——避免对多字节
    // 字符切片 panic，也保证非日期文本不会写进文件名
    let b = s.as_bytes();
    if s.len() < 10 || !b[..10].iter().all(|c| c.is_ascii()) {
        return String::new();
    }
    let year = &s[..4];
    let month = &s[5..7];
    let day = &s[8..10];
    let sep_ok = matches!(b[4], b':' | b'-' | b'/') && matches!(b[7], b':' | b'-' | b'/');
    if sep_ok
        && year.bytes().all(|c| c.is_ascii_digit())
        && month.bytes().all(|c| c.is_ascii_digit())
        && day.bytes().all(|c| c.is_ascii_digit())
    {
        format!("{year}{month}{day}")
    } else {
        String::new()
    }
}

/// 文件名清洗：去除 `/ \ : * ? " < > |` 与控制字符，连续空白折叠为单个空格，
/// 去首尾空白/尾部句点（Windows 禁止文件名以空格/句点结尾）。
fn sanitize_filename(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut prev_space = false;
    for c in name.chars() {
        if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
            continue;
        }
        if c.is_whitespace() {
            // 折叠连续空白（\t/\n/\r 等也归入此支，先于控制字符判断）；
            // 首部空白不写入（out 为空时跳过）
            if !prev_space && !out.is_empty() {
                out.push(' ');
            }
            prev_space = true;
            continue;
        }
        if c.is_control() {
            // 其余控制字符（非空白类）直接丢弃
            continue;
        }
        prev_space = false;
        out.push(c);
    }
    // 去尾部空白与句点（Windows 保留名/结尾点会令文件系统操作失败）
    while out.ends_with(' ') || out.ends_with('.') {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(name: &str) -> NameTemplateContext {
        NameTemplateContext {
            name: name.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn test_render_all_placeholders() {
        let c = NameTemplateContext {
            name: "DSC_0001".into(),
            species: Some("北红尾鸲".into()),
            date: Some("2024:05:12 10:30:00".into()),
            camera: Some("NIKON Z9".into()),
            seq: 7,
        };
        assert_eq!(
            render_name_template("{name}_{species}_{date}_{seq}_{camera}", &c),
            "DSC_0001_北红尾鸲_20240512_007_NIKON Z9"
        );
    }

    #[test]
    fn test_missing_fields_render_empty() {
        let c = ctx("IMG_001");
        // 缺失字段渲染为空串；模板整体非空（保留分隔符与 seq），不触发 fallback
        assert_eq!(
            render_name_template("{species}-{date}-{camera}-{seq}", &c),
            "---000"
        );
    }

    #[test]
    fn test_unknown_placeholder_preserved() {
        let c = ctx("IMG_001");
        assert_eq!(render_name_template("a{foo}b", &c), "a{foo}b");
        // 未闭合花括号按字面输出
        assert_eq!(render_name_template("a{b", &c), "a{b");
    }

    #[test]
    fn test_seq_zero_padded() {
        let c = ctx("IMG");
        assert_eq!(render_name_template("{seq}", &c), "000");
        let mut c2 = ctx("IMG");
        c2.seq = 42;
        assert_eq!(render_name_template("{seq}", &c2), "042");
        let mut c3 = ctx("IMG");
        c3.seq = 1234;
        assert_eq!(render_name_template("{seq}", &c3), "1234");
    }

    #[test]
    fn test_illegal_chars_sanitized() {
        let c = ctx("IMG_001");
        assert_eq!(
            render_name_template("a/b\\c:d*e?f\"g<h>i|j", &c),
            "abcdefghij"
        );
    }

    #[test]
    fn test_whitespace_collapsed_and_trimmed() {
        let c = ctx("IMG_001");
        assert_eq!(render_name_template("  a   b\t\tc\n ", &c), "a b c");
        assert_eq!(render_name_template("  a  ", &c), "a");
    }

    #[test]
    fn test_trailing_dot_stripped() {
        let c = ctx("IMG_001");
        assert_eq!(render_name_template("{name}.", &c), "IMG_001");
    }

    #[test]
    fn test_empty_result_falls_back_to_original_name() {
        // 模板只含非法字符/空白 → 清洗后为空 → 原名兜底
        let c = ctx("IMG_001");
        assert_eq!(render_name_template(":::   ", &c), "IMG_001");
        assert_eq!(render_name_template("", &c), "IMG_001");
    }

    #[test]
    fn test_date_formats() {
        let c = ctx("IMG");
        let mut iso = c.clone();
        iso.date = Some("2023-11-08T09:15:00".into());
        assert_eq!(render_name_template("{date}", &iso), "20231108");
        let mut slash = c.clone();
        slash.date = Some("2023/11/08 09:15".into());
        assert_eq!(render_name_template("{date}", &slash), "20231108");
        // 格式不符 → 渲染为空 → 整模板为空 → fallback 原名（乱码不写进文件名）
        let mut bad = c.clone();
        bad.date = Some("昨天拍的".into());
        assert_eq!(render_name_template("{date}", &bad), "IMG");
    }

    #[test]
    fn test_species_none_vs_some() {
        let mut c = ctx("IMG");
        // 模板只含 {species} 且缺失 → 渲染为空 → fallback 原名
        assert_eq!(render_name_template("{species}", &c), "IMG");
        c.species = Some("白鹭".into());
        assert_eq!(render_name_template("{species}", &c), "白鹭");
    }
}
