use std::path::{Path, PathBuf};
use thiserror::Error;

use photo_domain::XmpMetadata;

#[derive(Error, Debug)]
pub enum XmpError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Regex error: {0}")]
    Regex(#[from] regex_lite::Error),
}


/// 获取与主文件对应的 XMP 旁车文件路径
pub fn xmp_path(image_path: &Path) -> PathBuf {
    let stem = image_path.file_stem().unwrap_or_default();
    let parent = image_path.parent().unwrap_or(Path::new("."));
    parent.join(format!("{}.xmp", stem.to_string_lossy()))
}

/// XMP 骨架模板，包含 PT 自定义命名空间
const XMP_SKELETON: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
   xmlns:pt="http://ns.phototool.app/pt/1.0/">
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

/// 从 XMP 文件读取元数据
pub fn read_xmp(xmp_path: &Path) -> Result<XmpMetadata, XmpError> {
    if !xmp_path.exists() {
        return Ok(XmpMetadata::default());
    }

    let content = std::fs::read_to_string(xmp_path)?;
    Ok(parse_pt_properties(&content))
}

/// 写入 XMP 元数据到旁车文件
pub fn write_xmp(xmp_path: &Path, metadata: &XmpMetadata) -> Result<(), XmpError> {
    let content = if xmp_path.exists() {
        std::fs::read_to_string(xmp_path)?
    } else {
        XMP_SKELETON.to_string()
    };

    let updated = update_pt_properties(&content, metadata)?;

    // 确保父目录存在
    if let Some(parent) = xmp_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::write(xmp_path, &updated)?;
    Ok(())
}

/// 解析 XMP 内容中的 pt:* 自定义属性
fn parse_pt_properties(content: &str) -> XmpMetadata {
    let mut meta = XmpMetadata::default();

    if let Some(r) = extract_xml_attr(content, "pt:Rating") {
        meta.rating = r.parse().unwrap_or(0);
    }
    if let Some(c) = extract_xml_attr(content, "pt:ColorLabel") {
        meta.color_label = c;
    }
    if let Some(f) = extract_xml_attr(content, "pt:Flag") {
        meta.flag = f;
    }

    meta
}

/// 从 XML 属性字符串中提取属性值: pt:Rating="3" → Some("3")
fn extract_xml_attr(content: &str, attr_name: &str) -> Option<String> {
    let search = format!("{}=\"", attr_name);
    let start = content.find(&search)? + search.len();
    let rest = &content[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// 替换或插入 pt:* 属性
fn update_pt_properties(content: &str, metadata: &XmpMetadata) -> Result<String, XmpError> {
    // 构建新的属性字符串
    let new_attrs = format!(
        "pt:Rating=\"{}\" pt:ColorLabel=\"{}\" pt:Flag=\"{}\"",
        metadata.rating, metadata.color_label, metadata.flag
    );

    // 移除所有旧的 pt:* 属性
    let re = regex_lite::Regex::new(r#"pt:\w+="[^"]*""#)?;
    let cleaned = re.replace_all(content, "");

    // 清理多余空白（替换多个空格为一个）
    let re_ws = regex_lite::Regex::new(r#"\s+"#)?;
    let cleaned = re_ws.replace_all(&cleaned, " ");

    // 在 xmlns:pt 命名空间声明后面插入属性
    let xmlns_pt = "xmlns:pt=\"http://ns.phototool.app/pt/1.0/\"";
    if let Some(pos) = cleaned.find(xmlns_pt) {
        let insert_pos = pos + xmlns_pt.len();
        let mut result = cleaned.to_string();
        result.insert_str(insert_pos, &format!(" {}", new_attrs));
        // 修复可能的 " >" → ">" 问题
        let re_trailing = regex_lite::Regex::new(r#"" >"#)?;
        let result = re_trailing.replace_all(&result, "\">").to_string();
        // 修复空格重复
        let re_double = regex_lite::Regex::new(r#"\s{2,}"#)?;
        let result = re_double.replace_all(&result, " ").to_string();
        Ok(result)
    } else {
        // 保底：追加到内容末尾（通常不会触发）
        let mut result = cleaned.to_string();
        result.push_str(&format!(" {}", new_attrs));
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use photo_domain::{ColorLabel, Flag, Rating};

    #[test]
    fn test_xmp_metadata_default() {
        let m = XmpMetadata::default();
        assert_eq!(m.rating, 0);
        assert_eq!(m.color_label, "");
        assert_eq!(m.flag, "");
    }

    #[test]
    fn test_xmp_rating_conversion() {
        let mut m = XmpMetadata::default();
        m.set_rating(Rating::Four);
        assert_eq!(m.rating, 4);
        assert_eq!(m.rating(), Rating::Four);
    }

    #[test]
    fn test_xmp_color_label_conversion() {
        let mut m = XmpMetadata::default();
        m.set_color_label(ColorLabel::Red);
        assert_eq!(m.color_label, "red");
        assert_eq!(m.color_label(), ColorLabel::Red);
    }

    #[test]
    fn test_xmp_flag_conversion() {
        let mut m = XmpMetadata::default();
        m.set_flag(Some(Flag::Pick));
        assert_eq!(m.flag, "pick");
        assert_eq!(m.flag(), Some(Flag::Pick));

        m.set_flag(None);
        assert_eq!(m.flag, "");
        assert_eq!(m.flag(), None);
    }

    #[test]
    fn test_xmp_path() {
        let p = xmp_path(Path::new("/photos/DSC_0001.JPG"));
        assert_eq!(p, PathBuf::from("/photos/DSC_0001.xmp"));
    }

    #[test]
    fn test_read_write_xmp_roundtrip() {
        let dir = TempDir::new().unwrap();
        let xmp = dir.path().join("test.xmp");

        let meta = XmpMetadata {
            rating: 4,
            color_label: "red".into(),
            flag: "pick".into(),
        };
        write_xmp(&xmp, &meta).unwrap();
        assert!(xmp.exists());

        let read_back = read_xmp(&xmp).unwrap();
        assert_eq!(read_back.rating, 4, "rating mismatch");
        assert_eq!(read_back.color_label, "red", "color label mismatch");
        assert_eq!(read_back.flag, "pick", "flag mismatch");
    }

    #[test]
    fn test_read_nonexistent_xmp_returns_default() {
        let dir = TempDir::new().unwrap();
        let xmp = dir.path().join("nonexistent.xmp");
        let meta = read_xmp(&xmp).unwrap();
        assert_eq!(meta, XmpMetadata::default());
    }

    #[test]
    fn test_write_updates_existing_xmp() {
        let dir = TempDir::new().unwrap();
        let xmp = dir.path().join("update.xmp");

        // Write initial
        let initial = XmpMetadata {
            rating: 2,
            color_label: "".into(),
            flag: "".into(),
        };
        write_xmp(&xmp, &initial).unwrap();

        // Update
        let updated = XmpMetadata {
            rating: 5,
            color_label: "blue".into(),
            flag: "reject".into(),
        };
        write_xmp(&xmp, &updated).unwrap();

        let read_back = read_xmp(&xmp).unwrap();
        assert_eq!(read_back.rating, 5);
        assert_eq!(read_back.color_label, "blue");
        assert_eq!(read_back.flag, "reject");
    }
}
