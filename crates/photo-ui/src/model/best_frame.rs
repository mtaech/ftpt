//! 连拍组自动选优纯逻辑（对应 §4.5）。

use photo_domain::CaptureMeta;
use std::cmp::Ordering;

fn sharpness_of(m: &CaptureMeta) -> f32 {
    m.eye_sharpness.unwrap_or(-1.0)
}

fn size_of(m: &CaptureMeta) -> u64 {
    m.file_size.unwrap_or(0)
}

/// 两帧比较（选优全序）：Ordering::Less 表示 a 优于 b
fn compare_best(a: &CaptureMeta, b: &CaptureMeta) -> Ordering {
    // 1. eye_sharpness 降序（None 垫底）
    let sa = sharpness_of(a);
    let sb = sharpness_of(b);
    let d_sharp = sb.partial_cmp(&sa).unwrap_or(Ordering::Equal);
    if d_sharp != Ordering::Equal {
        return d_sharp;
    }

    // 2. 并列取 fileSize 更大者
    let d_size = size_of(b).cmp(&size_of(a));
    if d_size != Ordering::Equal {
        return d_size;
    }

    // 3. 再并列取 primary_path 字典序小者（确定性收尾）
    a.primary_path.cmp(&b.primary_path)
}

/// 连拍组内选最优帧（确定性）：按 eye_sharpness 降序，并列取 fileSize 更大者，再并列取 primary_path 升序
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
