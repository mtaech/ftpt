//! 连拍分组纯逻辑（对应 §4.4）。

use chrono::{NaiveDate, TimeZone, Utc};
use photo_domain::CaptureMeta;
use std::collections::HashMap;

/// 连拍组成员信息
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BurstEntry {
    pub group_id: String,
    pub size: usize,
    pub pos: usize,
}

/// 连拍组映射：key = 输入数组下标（显示序位置）；仅含 size >= 2 的组
pub type BurstGroupMap = HashMap<usize, BurstEntry>;

/// EXIF 拍摄时间 -> 毫秒时间戳（UTC）
///
/// 兼容两种形态：EXIF「2026:06:28 10:15:30」与 ISO「2026-08-01T10:15:30」
pub fn parse_exif_date(s: Option<&str>) -> Option<i64> {
    let s = s?.trim();
    if s.len() < 19 {
        return None;
    }
    let b = s.as_bytes();
    // 检查 4 位年与 5 个 2 位数
    if !b[0..4].iter().all(u8::is_ascii_digit)
        || b[4].is_ascii_digit()
        || !b[5..7].iter().all(u8::is_ascii_digit)
        || b[7].is_ascii_digit()
        || !b[8..10].iter().all(u8::is_ascii_digit)
        || b[10].is_ascii_digit()
        || !b[11..13].iter().all(u8::is_ascii_digit)
        || b[13].is_ascii_digit()
        || !b[14..16].iter().all(u8::is_ascii_digit)
        || b[16].is_ascii_digit()
        || !b[17..19].iter().all(u8::is_ascii_digit)
    {
        return None;
    }

    let y: i32 = s[0..4].parse().ok()?;
    let mo: u32 = s[5..7].parse().ok()?;
    let d: u32 = s[8..10].parse().ok()?;
    let h: u32 = s[11..13].parse().ok()?;
    let mi: u32 = s[14..16].parse().ok()?;
    let se: u32 = s[17..19].parse().ok()?;

    // 字段边界拦截（月 1-12，日 1-31，时 0-23，分 0-59，秒 0-59）
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || se > 59 {
        return None;
    }

    let date = NaiveDate::from_ymd_opt(y, mo, d)?;
    let dt = date.and_hms_opt(h, mi, se)?;
    Some(Utc.from_utc_datetime(&dt).timestamp_millis())
}

/// 连拍分组：按传入顺序（显示序）遍历，相邻两项拍摄时间差 <= gap_ms 归同组
pub fn compute_burst_groups(items: &[CaptureMeta], gap_ms: i64) -> BurstGroupMap {
    struct Range {
        start: usize,
        end: usize,
    }

    let mut bounds: Vec<Range> = Vec::new();
    let mut start: Option<usize> = None;
    let mut prev_t: Option<i64> = None;

    for (i, item) in items.iter().enumerate() {
        let t = parse_exif_date(item.date_taken.as_deref());
        match t {
            None => {
                // null/解析失败：结算当前组并切断链
                if let Some(s) = start {
                    bounds.push(Range { start: s, end: i });
                }
                start = None;
                prev_t = None;
            }
            Some(curr_t) => {
                match start {
                    None => {
                        start = Some(i);
                    }
                    Some(s) => {
                        if let Some(pt) = prev_t {
                            if (curr_t - pt).abs() <= gap_ms {
                                // 延续当前组
                            } else {
                                bounds.push(Range { start: s, end: i });
                                start = Some(i);
                            }
                        } else {
                            bounds.push(Range { start: s, end: i });
                            start = Some(i);
                        }
                    }
                }
                prev_t = Some(curr_t);
            }
        }
    }

    if let Some(s) = start {
        bounds.push(Range {
            start: s,
            end: items.len(),
        });
    }

    let mut map = HashMap::new();
    for (gi, b) in bounds.into_iter().enumerate() {
        let size = b.end - b.start;
        if size < 2 {
            continue;
        }
        let group_id = format!("burst-{gi}");
        for i in b.start..b.end {
            map.insert(
                i,
                BurstEntry {
                    group_id: group_id.clone(),
                    size,
                    pos: i - b.start,
                },
            );
        }
    }

    map
}
