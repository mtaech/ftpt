//! 堆叠分组纯逻辑（对应 §4.3）。

use photo_domain::CaptureMeta;
use std::collections::{HashMap, HashSet};

use super::burst::parse_exif_date;

/// 堆叠组：网格中的一个显示项
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackGroup {
    /// 分组键：ByFileName = baseName 或 baseName@dir；ByTime = `t-<最早时间戳>`；None = `i-<下标>`
    pub key: String,
    /// 成员下标（captures.items 下标，size >= 1）
    pub members: Vec<usize>,
    /// 激活成员下标（默认主格式优先，可手动覆盖）
    pub active: usize,
}

/// 主格式优先级：JPEG 出图优先；RAW/其他排最后
pub const PRIMARY_ORDER: &[&str] = &[
    "jpg", "jpeg", "png", "tif", "tiff", "gif", "webp", "bmp", "heic", "heif", "avif",
];

/// 连拍/同组照片堆叠时间窗（毫秒）
pub const STACK_TIME_GAP_MS: i64 = 2000;

fn ext_of(path: &str) -> String {
    if let Some(pos) = path.rfind('.') {
        let ext = &path[pos + 1..];
        if !ext.contains('/') && !ext.contains('\\') {
            return ext.to_ascii_lowercase();
        }
    }
    String::new()
}

fn dir_of(path: &str) -> &str {
    if let Some(pos) = path.rfind(|c| c == '/' || c == '\\') {
        &path[..pos]
    } else {
        ""
    }
}

/// 选主成员：按主路径真实扩展名在 PRIMARY_ORDER 中取最优；全部未命中回退组内首个成员
pub fn pick_primary(members: &[usize], items: &[CaptureMeta]) -> usize {
    if members.is_empty() {
        return 0;
    }
    let mut best = members[0];
    let mut best_rank = usize::MAX;

    for &i in members {
        if let Some(item) = items.get(i) {
            let ext = ext_of(&item.primary_path);
            if let Some(rank) = PRIMARY_ORDER.iter().position(|&x| x == ext) {
                if rank < best_rank {
                    best_rank = rank;
                    best = i;
                }
            }
        }
    }

    best
}

/// 不堆叠：每成员独立成组
pub fn group_singles(indices: &[usize]) -> Vec<StackGroup> {
    indices
        .iter()
        .map(|&i| StackGroup {
            key: format!("i-{i}"),
            members: vec![i],
            active: i,
        })
        .collect()
}

/// 同文件名堆叠：显示序下标按 (父目录, baseName) 分组
pub fn group_stacks(indices: &[usize], items: &[CaptureMeta]) -> Vec<StackGroup> {
    struct Bucket {
        dir: String,
        stem: String,
        members: Vec<usize>,
    }

    let mut buckets: HashMap<String, Bucket> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut dirs_per_stem: HashMap<String, HashSet<String>> = HashMap::new();

    for &i in indices {
        let Some(item) = items.get(i) else { continue };
        let stem = item.base_name.clone();
        if stem.is_empty() {
            continue;
        }
        let dir = dir_of(&item.primary_path).to_string();
        let bucket_key = format!("{dir}\0{stem}");

        if let Some(bucket) = buckets.get_mut(&bucket_key) {
            bucket.members.push(i);
        } else {
            buckets.insert(
                bucket_key.clone(),
                Bucket {
                    dir: dir.clone(),
                    stem: stem.clone(),
                    members: vec![i],
                },
            );
            order.push(bucket_key);
        }

        dirs_per_stem.entry(stem).or_default().insert(dir);
    }

    let pos_of: HashMap<usize, usize> = indices.iter().enumerate().map(|(p, &i)| (i, p)).collect();
    let mut groups: Vec<(StackGroup, usize)> = Vec::new();

    for bucket_key in order {
        let b = buckets.remove(&bucket_key).unwrap();
        let key = if dirs_per_stem.get(&b.stem).map_or(0, |s| s.len()) > 1 {
            format!("{}@{}", b.stem, b.dir)
        } else {
            b.stem
        };

        let mut pos = usize::MAX;
        for &m in &b.members {
            if let Some(&p) = pos_of.get(&m) {
                if p < pos {
                    pos = p;
                }
            }
        }

        let active = pick_primary(&b.members, items);
        groups.push((
            StackGroup {
                key,
                members: b.members,
                active,
            },
            pos,
        ));
    }

    groups.sort_by_key(|(_, pos)| *pos);
    groups.into_iter().map(|(g, _)| g).collect()
}

/// 同组照片堆叠：按拍摄时间聚类，相邻时间戳差 <= gap_ms 归为同组
pub fn group_by_time(indices: &[usize], items: &[CaptureMeta], gap_ms: i64) -> Vec<StackGroup> {
    struct TimedItem {
        idx: usize,
        t: i64,
    }

    let mut with_time: Vec<TimedItem> = Vec::new();
    let mut no_time: Vec<usize> = Vec::new();

    for &i in indices {
        let date_taken = items.get(i).and_then(|meta| meta.date_taken.as_deref());
        match parse_exif_date(date_taken) {
            Some(t) => with_time.push(TimedItem { idx: i, t }),
            None => no_time.push(i),
        }
    }

    // 稳定按时间排序（同时间戳保持显示序）
    with_time.sort_by_key(|item| item.t);

    struct Cluster {
        members: Vec<usize>,
        t0: i64,
    }

    let mut clusters: Vec<Cluster> = Vec::new();
    let mut cur: Vec<usize> = Vec::new();
    let mut t0 = 0i64;
    let mut prev_t: Option<i64> = None;

    for item in with_time {
        if !cur.is_empty() && prev_t.is_some_and(|pt| item.t - pt > gap_ms) {
            clusters.push(Cluster {
                members: std::mem::take(&mut cur),
                t0,
            });
        }
        if cur.is_empty() {
            t0 = item.t;
        }
        cur.push(item.idx);
        prev_t = Some(item.t);
    }
    if !cur.is_empty() {
        clusters.push(Cluster { members: cur, t0 });
    }

    let pos_of: HashMap<usize, usize> = indices.iter().enumerate().map(|(p, &i)| (i, p)).collect();
    let mut groups: Vec<(StackGroup, usize)> = Vec::new();

    for c in clusters {
        let mut pos = usize::MAX;
        for &m in &c.members {
            if let Some(&p) = pos_of.get(&m) {
                if p < pos {
                    pos = p;
                }
            }
        }
        let active = pick_primary(&c.members, items);
        groups.push((
            StackGroup {
                key: format!("t-{t0}", t0 = c.t0),
                members: c.members,
                active,
            },
            pos,
        ));
    }

    for i in no_time {
        let pos = pos_of.get(&i).copied().unwrap_or(usize::MAX);
        groups.push((
            StackGroup {
                key: format!("x-{i}"),
                members: vec![i],
                active: i,
            },
            pos,
        ));
    }

    groups.sort_by_key(|(_, pos)| *pos);
    groups.into_iter().map(|(g, _)| g).collect()
}
