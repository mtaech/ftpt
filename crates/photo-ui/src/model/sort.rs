//! 排序比较纯逻辑（对应 §4.2）。

use std::cmp::Ordering;
use std::collections::HashMap;

use photo_domain::{CaptureMeta, SortBy, SortDirection};

use super::filter::{FilterCriteria, filter_captures, rating_value};

/// 两条拍摄记录的比较器
pub fn compare_captures(
    sort_by: SortBy,
    a: &CaptureMeta,
    b: &CaptureMeta,
    quality_scores: Option<&HashMap<String, f64>>,
) -> Ordering {
    match sort_by {
        SortBy::FileName => a.base_name.to_lowercase().cmp(&b.base_name.to_lowercase()),
        SortBy::DateTaken => {
            let da = a.date_taken.as_deref().unwrap_or("");
            let db = b.date_taken.as_deref().unwrap_or("");
            da.cmp(db)
        }
        SortBy::FileSize => {
            let sa = a.file_size.unwrap_or(0);
            let sb = b.file_size.unwrap_or(0);
            sa.cmp(&sb)
        }
        SortBy::Rating => rating_value(a.rating).cmp(&rating_value(b.rating)),
        SortBy::Modified => match (&a.date_taken, &b.date_taken) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Less,
            (Some(_), None) => Ordering::Greater,
            (Some(ta), Some(tb)) => ta.cmp(tb),
        },
        SortBy::EyeSharpness => match (a.eye_sharpness, b.eye_sharpness) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Less, // null 排最前
            (Some(_), None) => Ordering::Greater,
            (Some(sa), Some(sb)) => sa.partial_cmp(&sb).unwrap_or(Ordering::Equal),
        },
        SortBy::Quality => {
            let sa = quality_scores.and_then(|m| m.get(&a.primary_path).copied());
            let sb = quality_scores.and_then(|m| m.get(&b.primary_path).copied());
            match (sa, sb) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater, // null 排最后（未评分照片不参与机筛排序）
                (Some(_), None) => Ordering::Less,
                (Some(va), Some(vb)) => va.partial_cmp(&vb).unwrap_or(Ordering::Equal),
            }
        }
    }
}

/// 过滤 + 排序 -> display_order（即 captures.items 的下标数组）
pub fn apply_filter_and_sort(
    items: &[CaptureMeta],
    criteria: &FilterCriteria,
    sort_by: SortBy,
    sort_direction: SortDirection,
    quality_scores: Option<&HashMap<String, f64>>,
) -> Vec<usize> {
    let mut indices = filter_captures(items, criteria);

    // 稳定排序（同键保持 display 原序）
    indices.sort_by(|&a, &b| {
        let cmp = compare_captures(sort_by, &items[a], &items[b], quality_scores);
        match sort_direction {
            SortDirection::Ascending => cmp,
            SortDirection::Descending => cmp.reverse(),
        }
    });

    indices
}
