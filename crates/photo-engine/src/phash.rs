//! pHash 近重复检测：dHash（差分哈希）+ 汉明距离 + 贪心聚类。
//!
//! 用途：内容级重复照片检测（跨目录、不同导出版本——缩放/JPEG 压缩/轻微编辑后
//! dHash 保持接近，汉明距离小；无关照片哈希远）。
//! 哈希输入优先复用缩略图磁盘缓存（`.pt/thumbs/{key}.jpg`，std@440 键），命中即
//! 零解码成本；未命中走 `thumbnail::ThumbnailCache` 现有生成路径（含 RAW 母版解码）。

use std::path::Path;

use photo_domain::{ImageFormat, SourceFile};

use crate::thumbnail::{ThumbnailCache, ThumbnailError};

/// 默认汉明距离阈值：≤10 bit 差异视为近重复（dHash 64bit，抗缩放/压缩/轻微编辑）
pub const DEFAULT_HASH_THRESHOLD: u32 = 10;

/// 哈希用缩略图尺寸：与网格缩略图预生成同尺寸（`config.thumbnail_size * 2` 的
/// std@size 缓存键），最大化磁盘缓存命中；dHash 只需 9x8 灰度网格，440px 足够。
const HASH_THUMB_SIZE: u32 = 440;

/// 差分哈希：JPEG 字节 → 64bit（缩到 9x8 灰度网格，逐行比较相邻像素亮度，
/// 左 < 右 记 1 位）。对缩放/JPEG 压缩/轻微亮度平移鲁棒；与宽高比无关
/// （网格直接拉伸，指纹只关心相对亮度分布）。
pub fn dhash(jpeg_bytes: &[u8]) -> Result<u64, ThumbnailError> {
    let img = image::load_from_memory(jpeg_bytes)?;
    let luma = img.to_luma8();
    // 精确缩放到 9x8：dHash 约定网格，不做保宽高比
    let grid = image::imageops::resize(&luma, 9, 8, image::imageops::FilterType::Triangle);
    let mut hash: u64 = 0;
    for y in 0..8u32 {
        for x in 0..8u32 {
            let left = grid.get_pixel(x, y)[0];
            let right = grid.get_pixel(x + 1, y)[0];
            if left < right {
                hash |= 1 << (y * 8 + x);
            }
        }
    }
    Ok(hash)
}

/// 汉明距离：两哈希差异 bit 数（0 = 完全相同；64 = 完全无关）
pub fn hamming(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// 贪心聚类：按输入顺序，每张与已有组锚点（组内首张）比较，汉明距离
/// ≤ threshold 并入该组，否则新建一组（自身成为新锚点）。
/// 只返回成员 ≥2 的组（近重复检测输出；单张不成组）。组序与组内序 = 输入序
/// （扫描序），结果确定；组内首张即「保留锚点」，供前端一键保留首张标其余。
pub fn group_duplicates(pairs: Vec<(String, u64)>, threshold: u32) -> Vec<Vec<String>> {
    let mut groups: Vec<(u64, Vec<String>)> = Vec::new();
    for (path, hash) in pairs {
        let mut joined = false;
        for (anchor, members) in groups.iter_mut() {
            if hamming(*anchor, hash) <= threshold {
                members.push(path.clone());
                joined = true;
                break;
            }
        }
        if !joined {
            groups.push((hash, vec![path]));
        }
    }
    groups
        .into_iter()
        .map(|(_, members)| members)
        .filter(|members| members.len() >= 2)
        .collect()
}

/// 批量计算 dHash：对每个路径构造 SourceFile（扩展名格式 + 元数据大小），
/// 优先读磁盘缩略图缓存（std@440 键，命中零解码），未命中走
/// `ThumbnailCache::get_or_generate` 生成（常规图 DCT 缩放 / RAW 母版解码派生，
/// 生成结果落盘复用）。
/// 单张失败（文件被删/解码异常）跳过并记日志，不中止整体；返回
/// (完整路径, 64bit dHash) 成功列表（保持 paths 顺序）。progress_cb 每张完成后
/// 回调 1..=paths.len()。
pub fn compute_hashes(
    cache: &ThumbnailCache,
    paths: &[String],
    progress_cb: impl Fn(u32) + Send,
) -> Result<Vec<(String, u64)>, ThumbnailError> {
    let mut out = Vec::with_capacity(paths.len());
    for (i, path) in paths.iter().enumerate() {
        match hash_one(cache, Path::new(path)) {
            Ok(hash) => out.push((path.clone(), hash)),
            Err(_) => tracing::warn!("近重复哈希计算失败，跳过: {path}"),
        }
        progress_cb((i + 1) as u32);
    }
    Ok(out)
}

/// 单张路径 → dHash。读缓存命中零解码；未命中走生成函数。
fn hash_one(cache: &ThumbnailCache, path: &Path) -> Result<u64, ThumbnailError> {
    let file_size = std::fs::metadata(path).map(|m| m.len()).ok();
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return Err(ThumbnailError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "无扩展名",
        )));
    };
    let Some(format) = ImageFormat::from_extension(&ext.to_lowercase()) else {
        return Err(ThumbnailError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "非图片扩展名",
        )));
    };
    let source = SourceFile {
        path: path.to_path_buf(),
        format,
        file_size,
    };
    // 优先读磁盘缓存：键与 ThumbnailCache::cache_key 同约定
    //（DefaultHasher(CACHE_VERSION + variant + path + size + file_size)，文件覆盖自动失效）
    let key = cache.cache_key(&source, HASH_THUMB_SIZE, "std");
    let cache_path = cache.cache_dir().join(&key);
    let jpeg = if cache_path.exists() {
        // 命中预生成缩略图：零解码成本
        std::fs::read(&cache_path)?
    } else {
        // 未命中：走现有生成函数（内部也会先查缓存；生成结果落盘供后续复用）
        cache.get_or_generate(&source, HASH_THUMB_SIZE, None)?
    };
    let hash = dhash(&jpeg)?;
    Ok(hash)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// 确定性合成场景图：水平亮度渐变 + 中央亮块 + 左缘竖亮条。
    /// 全部为大尺度结构（无像素级噪声），缩放/平移后 dHash 基本不变。
    fn scene(w: u32, h: u32) -> image::RgbImage {
        let mut img = image::RgbImage::new(w, h);
        // 线性水平渐变（亮度随 x 增加，左暗右亮）
        for y in 0..h {
            for x in 0..w {
                let v = (x as f32 / w as f32 * 200.0 + 30.0) as u8;
                img.put_pixel(x, y, image::Rgb([v, v, v]));
            }
        }
        // 中央亮块
        let (bw, bh) = (w / 4, h / 4);
        for y in h / 2 - bh / 2..h / 2 + bh / 2 {
            for x in w / 2 - bw / 2..w / 2 + bw / 2 {
                img.put_pixel(x, y, image::Rgb([245, 245, 245]));
            }
        }
        // 左缘竖亮条
        for y in h / 4..h * 3 / 4 {
            for x in w / 6..w / 6 + w / 20 {
                img.put_pixel(x, y, image::Rgb([250, 250, 250]));
            }
        }
        img
    }

    /// 无关场景图：反向渐变（右暗左亮）+ 异位亮块/亮条 → 哈希方向相反
    fn scene_unrelated(w: u32, h: u32) -> image::RgbImage {
        let mut img = image::RgbImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let v = (255.0 - x as f32 / w as f32 * 200.0 - 25.0) as u8;
                img.put_pixel(x, y, image::Rgb([v, v, v]));
            }
        }
        let (bw, bh) = (w / 4, h / 4);
        for y in h / 2 - bh / 2..h / 2 + bh / 2 {
            for x in w / 3 - bw / 2..w / 3 + bw / 2 {
                img.put_pixel(x, y, image::Rgb([245, 245, 245]));
            }
        }
        for y in h / 4..h * 3 / 4 {
            for x in w * 2 / 3..w * 2 / 3 + w / 20 {
                img.put_pixel(x, y, image::Rgb([250, 250, 250]));
            }
        }
        img
    }

    fn encode_jpeg(img: &image::RgbImage) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Jpeg).unwrap();
        buf.into_inner()
    }

    #[test]
    fn test_hamming_basic() {
        assert_eq!(hamming(0, 0), 0);
        assert_eq!(hamming(u64::MAX, u64::MAX), 0);
        assert_eq!(hamming(0, u64::MAX), 64);
        assert_eq!(hamming(0b1010, 0b1111), 2);
        // 对称性 + popcount 语义
        let a = 0xDEAD_BEEF_CAFE_F00Du64;
        let b = 0x1234_5678_9ABC_DEF0u64;
        assert_eq!(hamming(a, b), hamming(b, a));
        assert_eq!(hamming(a, b), (a ^ b).count_ones());
    }

    #[test]
    fn test_dhash_identical_bytes() {
        let jpeg = encode_jpeg(&scene(320, 240));
        let h1 = dhash(&jpeg).unwrap();
        let h2 = dhash(&jpeg).unwrap();
        assert_eq!(h1, h2);
        assert_eq!(hamming(h1, h2), 0);
    }

    #[test]
    fn test_dhash_scale_approx() {
        // 缩放近似（模拟不同导出尺寸）：400x300 原图 vs 150x113 缩放，哈希应接近
        let a = scene(400, 300);
        let b = image::imageops::resize(&a, 150, 113, image::imageops::FilterType::Triangle);
        let ha = dhash(&encode_jpeg(&a)).unwrap();
        let hb = dhash(&encode_jpeg(&b)).unwrap();
        assert!(hamming(ha, hb) <= 4, "缩放图哈希应接近，实际距离 {}", hamming(ha, hb));
    }

    #[test]
    fn test_dhash_translate_approx() {
        // 平移近似（模拟画面轻微位移）：裁去 (12,10) 偏移内容再拉伸回同尺寸
        let a = scene(400, 300);
        let shifted = image::imageops::crop_imm(&a, 12, 10, 388, 290).to_image();
        let c = image::imageops::resize(&shifted, 400, 300, image::imageops::FilterType::Triangle);
        let ha = dhash(&encode_jpeg(&a)).unwrap();
        let hc = dhash(&encode_jpeg(&c)).unwrap();
        assert!(hamming(ha, hc) <= 12, "平移图哈希应接近，实际距离 {}", hamming(ha, hc));
    }

    #[test]
    fn test_dhash_unrelated_far() {
        // 无关图（反向渐变 + 异位结构）：哈希应远
        let a = scene(400, 300);
        let d = scene_unrelated(400, 300);
        let ha = dhash(&encode_jpeg(&a)).unwrap();
        let hd = dhash(&encode_jpeg(&d)).unwrap();
        assert!(hamming(ha, hd) >= 16, "无关图哈希应远，实际距离 {}", hamming(ha, hd));
    }

    #[test]
    fn test_group_duplicates_clusters() {
        // 阈值 10：距离 ≤10 合并，>10 不合并；组序 = 锚点首现序，组内序 = 输入序。
        // 位掩码构造距离：hamming(a, b) = popcount(a ^ b)
        let mask10 = 0b11_1111_1111u64; // 10 个 1
        let mask11 = 0b111_1111_1111u64; // 11 个 1
        let mask12 = 0b1111_1111_1111u64; // 12 个 1
        let pairs = vec![
            ("a".to_string(), 0),
            ("b".to_string(), mask10),          // 距 a = 10 → 并入 a 组
            ("c".to_string(), mask11),          // 距 a = 11 → 新组（锚点 c）
            ("d".to_string(), 1),               // 距 a = 1 → 并入 a 组（不比较 c）
            ("e".to_string(), mask12),          // 距 a = 12 > 10；距 c = 1 → 并入 c 组
        ];
        let groups = group_duplicates(pairs, 10);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0], vec!["a", "b", "d"]);
        assert_eq!(groups[1], vec!["c", "e"]);
    }

    #[test]
    fn test_group_duplicates_singletons_filtered() {
        let pairs = vec![
            ("x".to_string(), 0),
            ("y".to_string(), u64::MAX), // 距 x = 64，独苗
            ("z".to_string(), 0),        // 距 x = 0 → 并入
        ];
        let groups = group_duplicates(pairs, 10);
        assert_eq!(groups, vec![vec!["x".to_string(), "z".to_string()]]);
    }

    #[test]
    fn test_group_duplicates_deterministic() {
        // 相同输入两次运行结果完全一致（组序 + 组内序）
        let pairs = vec![
            ("a".to_string(), 0),
            ("b".to_string(), 2),          // 距 a = 1
            ("c".to_string(), 4),          // 距 a = 1
            ("d".to_string(), u64::MAX),   // 距 a = 64 → 新组
            ("e".to_string(), u64::MAX ^ 1), // 距 d = 1
        ];
        let g1 = group_duplicates(pairs.clone(), 10);
        let g2 = group_duplicates(pairs, 10);
        assert_eq!(g1, g2);
        assert_eq!(g1.len(), 2);
        assert_eq!(g1[0], vec!["a", "b", "c"]);
        assert_eq!(g1[1], vec!["d", "e"]);
    }

    #[test]
    fn test_group_duplicates_threshold_boundary() {
        // 距离恰好 = 阈值 → 合并；阈值 + 1 → 不合并（单独成组）
        let mask10 = 0b11_1111_1111u64;
        let c_hash = mask10 | (1 << 63); // 距 a = 11（阈值 + 1）
        let pairs = vec![
            ("a".to_string(), 0),
            ("b".to_string(), mask10),   // 距 a = 10 → 并入 a 组
            ("c".to_string(), c_hash),   // 距 a = 11 → 不并 a，新组锚点
            ("d".to_string(), c_hash),   // 距 c = 0 → 并入 c 组（距 a 仍为 11）
        ];
        let groups = group_duplicates(pairs, 10);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0], vec!["a", "b"]);
        assert_eq!(groups[1], vec!["c", "d"]);
    }
}
