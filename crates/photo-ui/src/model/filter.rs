//! 筛选纯逻辑（对应 §4.1）。

use chrono::NaiveDate;
use photo_domain::{
    CaptureMeta, ColorLabel, Flag, ImageFormat, Rating, RecognitionFilter, RecognitionStatus,
};
use serde::{Deserialize, Serialize};

/// UI 级筛选条件（包含 photo_domain::FilterCriteria 全部字段 + ISO/焦距/镜头/关键词筛选）
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FilterCriteria {
    /// 格式精确匹配（大小写归一；RAW 为通配语义）
    pub format_filter: Option<ImageFormat>,
    /// 物种多选（命中任一即保留，空 = 不限）
    pub taxon_names: Vec<String>,
    /// 拍摄日期范围（闭区间，比较 YYYY-MM-DD）
    pub date_from: Option<NaiveDate>,
    pub date_to: Option<NaiveDate>,
    /// 最低评分（>= N 语义）
    pub min_rating: Option<Rating>,
    /// 颜色标签精确匹配
    pub color_label: Option<ColorLabel>,
    /// 旗标精确匹配（Pick/Reject）
    pub flag_filter: Option<Flag>,
    /// 只显示无旗标（与 flag_filter 互斥）
    pub unflagged_filter: bool,
    /// 只看**源文件为空**（0 字节）的照片：复制/传输中断的残骸，便于筛出来重拷或删除
    pub empty_source_only: bool,
    /// 识别状态过滤
    pub recognition_filter: RecognitionFilter,
    /// ISO 闭区间 [iso_min, iso_max]
    pub iso_min: Option<u32>,
    pub iso_max: Option<u32>,
    /// 焦距闭区间 [focal_min, focal_max]（mm）
    pub focal_min: Option<f64>,
    pub focal_max: Option<f64>,
    /// 镜头多选（精确匹配 EXIF lens 串，空 = 不限）
    pub lens_filter: Vec<String>,
    /// 关键词筛选（包含任一选中关键词即中，空 = 不限）
    pub keyword_filter: Vec<String>,
}

/// 从当前目录照片收集「镜头」筛选候选项（去重 + 排序；空白忽略）。
///
/// 候选只用于下拉展示；筛选语义仍由 matches_criteria 的 lens_filter 精确匹配决定
/// （候选里没有的旧筛选值不会被悄悄清掉，见 §13.3）。
pub fn lens_options(items: &[CaptureMeta]) -> Vec<String> {
    let mut out: Vec<String> = items
        .iter()
        .filter_map(|m| m.lens.as_deref())
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    out.sort();
    out.dedup();
    out
}

/// 从当前目录照片收集「物种」筛选候选项：顶层展示名 + 每个主体的展示名。
///
/// 为什么必须带主体名：筛选语义是「任一主体命中即保留」（见 matches_criteria），
/// 只列顶层名会让多主体照片的次要物种根本选不到。占位名「未识别」不作为候选。
pub fn taxon_options(items: &[CaptureMeta]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for meta in items {
        if let Some(name) = meta
            .taxon_name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
        {
            out.push(name.to_string());
        }
        for subject in &meta.subjects {
            let name = subject.display_name.trim();
            if !name.is_empty() && name != "<未识别>" {
                out.push(name.to_string());
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

impl From<FilterCriteria> for photo_domain::FilterCriteria {
    fn from(c: FilterCriteria) -> Self {
        photo_domain::FilterCriteria {
            format_filter: c.format_filter,
            taxon_names: c.taxon_names,
            date_from: c.date_from,
            date_to: c.date_to,
            min_rating: c.min_rating,
            color_label: c.color_label,
            flag_filter: c.flag_filter,
            unflagged_filter: c.unflagged_filter,
            recognition_filter: c.recognition_filter,
        }
    }
}

/// Rating 转数值 (0..=5)，None=0, One=1, ..., Five=5
pub fn rating_value(r: Rating) -> u8 {
    match r {
        Rating::None => 0,
        Rating::One => 1,
        Rating::Two => 2,
        Rating::Three => 3,
        Rating::Four => 4,
        Rating::Five => 5,
    }
}

/// 非 RAW 格式的标准显示名集合
const NON_RAW_FORMAT_NAMES: &[&str] =
    &["jpeg", "png", "tiff", "heif", "webp", "bmp", "gif", "other"];

/// 格式匹配：RAW 是通配语义（若扩展名为 "RAW" 匹配任意非标准格式；若是具体扩展名如 "NEF" 精确匹配）
pub fn matches_format(primary_format: &str, fmt: &ImageFormat) -> bool {
    match fmt {
        ImageFormat::Raw(ext) => {
            let want = ext.to_ascii_uppercase();
            if want == "RAW" {
                !NON_RAW_FORMAT_NAMES
                    .iter()
                    .any(|&f| f.eq_ignore_ascii_case(primary_format))
            } else {
                primary_format.eq_ignore_ascii_case(&want)
            }
        }
        _ => primary_format.eq_ignore_ascii_case(&fmt.to_string()),
    }
}

/// 焦距字符串解析：提取第一个数值（如 "70-200mm" -> 70.0）
pub fn parse_focal_length_mm(s: &str) -> Option<f64> {
    let mut num_str = String::new();
    let mut found_num = false;
    let mut has_dot = false;

    for c in s.chars() {
        if c.is_ascii_digit() {
            num_str.push(c);
            found_num = true;
        } else if c == '.' && found_num && !has_dot {
            num_str.push(c);
            has_dot = true;
        } else if found_num {
            break;
        }
    }

    if found_num {
        num_str.parse::<f64>().ok()
    } else {
        None
    }
}

/// 拍摄时间解析为 NaiveDate
fn parse_date_taken(date_str: &str) -> Option<NaiveDate> {
    let s = date_str.trim();
    if s.len() < 10 {
        return None;
    }
    let b = s.as_bytes();
    if !b[0..4].iter().all(u8::is_ascii_digit)
        || (b[4] != b'-' && b[4] != b':')
        || !b[5..7].iter().all(u8::is_ascii_digit)
        || (b[7] != b'-' && b[7] != b':')
        || !b[8..10].iter().all(u8::is_ascii_digit)
    {
        return None;
    }
    let y: i32 = s[0..4].parse().ok()?;
    let m: u32 = s[5..7].parse().ok()?;
    let d: u32 = s[8..10].parse().ok()?;
    NaiveDate::from_ymd_opt(y, m, d)
}

/// 过滤照片：返回通过全部条件的下标数组（升序）
pub fn filter_captures(items: &[CaptureMeta], criteria: &FilterCriteria) -> Vec<usize> {
    let mut out = Vec::new();

    for (i, meta) in items.iter().enumerate() {
        // format_filter
        if let Some(fmt) = &criteria.format_filter {
            if !matches_format(&meta.primary_format, fmt) {
                continue;
            }
        }

        // taxon_names: 命中任一选中项即保留（多主体：任一主体的展示名命中即可）
        if !criteria.taxon_names.is_empty() {
            let matched = meta.subjects.iter().any(|s| {
                criteria.taxon_names.iter().any(|b| b == &s.display_name)
            }) || meta.taxon_name.as_ref().is_some_and(|name| {
                criteria.taxon_names.iter().any(|b| b == name)
            });
            if !matched {
                continue;
            }
        }

        // date_from / date_to
        if criteria.date_from.is_some() || criteria.date_to.is_some() {
            if let Some(date_taken) = &meta.date_taken {
                if let Some(d) = parse_date_taken(date_taken) {
                    if let Some(from) = criteria.date_from {
                        if d < from {
                            continue;
                        }
                    }
                    if let Some(to) = criteria.date_to {
                        if d > to {
                            continue;
                        }
                    }
                }
                // 解析失败：保留（对齐文档 §4.1）
            } else {
                // 无拍摄时间且设了日期范围 -> 排除
                continue;
            }
        }

        // unflagged_filter: 只显示无旗标
        if criteria.unflagged_filter && meta.flag.is_some() {
            continue;
        }

        // empty_source_only: 只看 0 字节的残骸
        if criteria.empty_source_only && !meta.is_empty_source() {
            continue;
        }

        // min_rating: 评分 >= N
        if let Some(min_r) = criteria.min_rating {
            if rating_value(meta.rating) < rating_value(min_r) {
                continue;
            }
        }

        // color_label
        if let Some(target_label) = criteria.color_label {
            if meta.color_label != target_label {
                continue;
            }
        }

        // flag_filter
        if criteria.flag_filter.is_some() && meta.flag != criteria.flag_filter {
            continue;
        }

        // iso 区间 [iso_min, iso_max]
        if criteria.iso_min.is_some() || criteria.iso_max.is_some() {
            match meta.iso {
                Some(iso) => {
                    if let Some(min_iso) = criteria.iso_min {
                        if iso < min_iso {
                            continue;
                        }
                    }
                    if let Some(max_iso) = criteria.iso_max {
                        if iso > max_iso {
                            continue;
                        }
                    }
                }
                None => continue,
            }
        }

        // 焦距区间 [focal_min, focal_max]
        if criteria.focal_min.is_some() || criteria.focal_max.is_some() {
            match &meta.focal_length {
                Some(fl) => match parse_focal_length_mm(fl) {
                    Some(mm) => {
                        if let Some(min_fl) = criteria.focal_min {
                            if mm < min_fl {
                                continue;
                            }
                        }
                        if let Some(max_fl) = criteria.focal_max {
                            if mm > max_fl {
                                continue;
                            }
                        }
                    }
                    None => continue,
                },
                None => continue,
            }
        }

        // lens_filter: 精确匹配
        if !criteria.lens_filter.is_empty() {
            let matched = match &meta.lens {
                Some(lens) => criteria.lens_filter.iter().any(|l| l == lens),
                None => false,
            };
            if !matched {
                continue;
            }
        }

        // keyword_filter: 包含任一关键词
        if !criteria.keyword_filter.is_empty() {
            let matched = meta
                .keywords
                .iter()
                .any(|k| criteria.keyword_filter.iter().any(|target| target == k));
            if !matched {
                continue;
            }
        }

        // recognition_filter
        match criteria.recognition_filter {
            RecognitionFilter::Confirmed => {
                if meta.recognition_status != Some(RecognitionStatus::Confirmed) {
                    continue;
                }
            }
            RecognitionFilter::NeedsReview => {
                if meta.recognition_status != Some(RecognitionStatus::NeedsReview) {
                    continue;
                }
            }
            RecognitionFilter::Unrecognized => {
                if meta.recognition_status != Some(RecognitionStatus::Unrecognized) {
                    continue;
                }
            }
            RecognitionFilter::NotRecognized => {
                if meta.recognition_status.is_some() {
                    continue;
                }
            }
            RecognitionFilter::All => {}
        }

        out.push(i);
    }

    out
}

/// 默认筛选条件
pub fn default_filter_criteria() -> FilterCriteria {
    FilterCriteria::default()
}

/// 是否有任一筛选生效（作为批量操作的安全边界）
pub fn has_active_filters(criteria: &FilterCriteria) -> bool {
    criteria.format_filter.is_some()
        || !criteria.taxon_names.is_empty()
        || criteria.date_from.is_some()
        || criteria.date_to.is_some()
        || criteria.min_rating.is_some()
        || criteria.color_label.is_some()
        || criteria.flag_filter.is_some()
        || criteria.unflagged_filter
        || criteria.empty_source_only
        || criteria.recognition_filter != RecognitionFilter::All
        || criteria.iso_min.is_some()
        || criteria.iso_max.is_some()
        || criteria.focal_min.is_some()
        || criteria.focal_max.is_some()
        || !criteria.lens_filter.is_empty()
        || !criteria.keyword_filter.is_empty()
}
