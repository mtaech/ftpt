//! 连拍组自动选优纯逻辑（对应 §4.5）。

use photo_domain::CaptureMeta;
use std::cmp::Ordering;

fn size_of(m: &CaptureMeta) -> u64 {
    m.file_size.unwrap_or(0)
}

/// 两帧比较（选优全序）：Ordering::Less 表示 a 优于 b
///
/// 2026-09-22 起鸟眼锐度已整体移除，选优回到「文件尺寸优先」——
/// 尺寸更接近「连拍多帧里参数/构图最完整的一张」，再用路径字典序保证确定性。
fn compare_best(a: &CaptureMeta, b: &CaptureMeta) -> Ordering {
    // 1. fileSize 更大者优先（原 eye_sharpness 档已删除）
    let d_size = size_of(b).cmp(&size_of(a));
    if d_size != Ordering::Equal {
        return d_size;
    }

    // 2. 并列取 primary_path 字典序小者（确定性收尾）
    a.primary_path.cmp(&b.primary_path)
}

/// 连拍组内选最优帧（确定性）：按 fileSize 降序，并列取 primary_path 升序
pub fn pick_best_frame(members: &[CaptureMeta]) -> Option<String> {
    if members.len() < 2 {
        return None;
    }
    let mut best = &members[0];
    for m in &members[1..] {
        if compare_best(m, best) == Ordering::Less {
            best = m;
        }
    }
    Some(best.primary_path.clone())
}

/// 组内非最优帧路径列表（保持原组序，供批量标 Reject）
pub fn non_best_paths(group: &[CaptureMeta]) -> Vec<String> {
    if group.len() < 2 {
        return Vec::new();
    }
    let Some(best) = pick_best_frame(group) else {
        return Vec::new();
    };
    group
        .iter()
        .filter(|m| m.primary_path != best)
        .map(|m| m.primary_path.clone())
        .collect()
}
