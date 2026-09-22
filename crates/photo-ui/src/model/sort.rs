//! 排序比较纯逻辑（对应 §4.2）。

use std::cmp::Ordering;

use photo_domain::{CaptureMeta, SortBy, SortDirection};

use super::filter::{FilterCriteria, filter_captures, rating_value};

/// 两条拍摄记录的比较器
pub fn compare_captures(sort_by: SortBy, a: &CaptureMeta, b: &CaptureMeta) -> Ordering {
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
    }
}

/// 过滤 + 排序 -> display_order（即 captures.items 的下标数组）
pub fn apply_filter_and_sort(
    items: &[CaptureMeta],
    criteria: &FilterCriteria,
    sort_by: SortBy,
    sort_direction: SortDirection,
) -> Vec<usize> {
    let mut indices = filter_captures(items, criteria);

    // 稳定排序（同键保持 display 原序）
    indices.sort_by(|&a, &b| {
        let cmp = compare_captures(sort_by, &items[a], &items[b]);
        match sort_direction {
            SortDirection::Ascending => cmp,
            SortDirection::Descending => cmp.reverse(),
        }
    });

    indices
}
// ── 筛选栏下拉选项（排序方式 / 网格列数）──

/// 排序方式下拉选项：(value = 机器可读稳定值，label = 中文名)。
/// 顺序 = 下拉里的显示顺序。
pub const SORT_OPTIONS: [(&str, &str); 5] = [
    ("file_name", "文件名"),
    ("date_taken", "拍摄时间"),
    ("file_size", "文件大小"),
    ("rating", "星级评分"),
    ("modified", "修改时间"),
];

/// 排序方式 → 下拉 value
pub fn sort_by_value(sort: SortBy) -> &'static str {
    match sort {
        SortBy::FileName => "file_name",
        SortBy::DateTaken => "date_taken",
        SortBy::FileSize => "file_size",
        SortBy::Rating => "rating",
        SortBy::Modified => "modified",
    }
}

/// 下拉 value → 排序方式；未知值回退「文件名」（下拉数据坏掉也不至于乱排）
pub fn sort_by_from_value(value: &str) -> SortBy {
    SORT_OPTIONS
        .iter()
        .find(|(v, _)| *v == value)
        .map(|(v, _)| match *v {
            "date_taken" => SortBy::DateTaken,
            "file_size" => SortBy::FileSize,
            "rating" => SortBy::Rating,
            "modified" => SortBy::Modified,
            _ => SortBy::FileName,
        })
        .unwrap_or(SortBy::FileName)
}

/// 网格列数下拉选项（value = 列数字符串，label = "N 列"）
pub const GRID_COL_OPTIONS: [(&str, &str); 4] =
    [("2", "2 列"), ("3", "3 列"), ("4", "4 列"), ("5", "5 列")];

/// 下拉 value → 列数；钳制到 2–5，非法值回退 4
pub fn grid_columns_from_value(value: &str) -> usize {
    value.parse::<usize>().map(|c| c.clamp(2, 5)).unwrap_or(4)
}
