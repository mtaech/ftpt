//! 直方图的显示口径（§7.4 / §9.7 Hero）：降采样、柱高归一化、剪切百分比。
//!
//! 统计本身在 photo_engine::histogram（256 桶 luma/RGB + 剪切计数）；这里只放
//! 「怎么把它画出来」的纯换算，不含 GPUI 依赖——渲染在 views::histogram，冒烟在 adjust_smoke。

use photo_engine::histogram::HistogramData;

/// 柱状图柱数：256 桶降采样到 64。面板净宽只有 200px 出头，64 根柱子足够分辨形态，
/// 再多只会把每根柱子压到 1px 以下（看不出差异，还多 3 倍的元素开销）。
pub const HIST_BINS: usize = 64;

/// 把 256 桶按等宽合并成 bins 桶。bins 为 0 或不能整除 256 时**不合并**（返回原 256 桶）——
/// 调用方给错参数时宁可画得密一点，也不要悄悄丢数据。
pub fn downsample(src: &[u32; 256], bins: usize) -> Vec<u32> {
    if bins == 0 || 256 % bins != 0 {
        return src.to_vec();
    }
    let step = 256 / bins;
    src.chunks(step).map(|c| c.iter().sum()).collect()
}

/// 亮度（luma）柱状计数：显示主图形态用亮度；RGB 三线留待后续（先把「拉曝光往哪边偏」看住）。
pub fn luma_bins(data: &HistogramData) -> Vec<u32> {
    downsample(&data.luma, HIST_BINS)
}

/// 归一化柱高：按 sqrt 压缩动态范围（高光尖峰不让其余柱子全趴下），
/// 峰值取「当前图 + 基线图」的公共最大值，这样两张图的高度**可比**。
///
/// 返回 0..=max_height；全零输入返回全 0（而不是除零）。
pub fn bin_heights(counts: &[u32], other: Option<&[u32]>, max_height: f32) -> Vec<f32> {
    let peak = counts
        .iter()
        .chain(other.into_iter().flatten())
        .copied()
        .max()
        .unwrap_or(0);
    if peak == 0 {
        return vec![0.0; counts.len()];
    }
    let denom = (peak as f32).sqrt();
    counts
        .iter()
        .map(|&c| (c as f32).sqrt() / denom * max_height)
        .collect()
}

/// 剪切百分比 (高光%, 死黑%)：分母 = 参与统计的总像素；无像素时给 (0, 0) 而不是 NaN。
pub fn clip_percent(data: &HistogramData) -> (f32, f32) {
    let total = data.total_pixels();
    if total == 0 {
        return (0.0, 0.0);
    }
    let pct = |n: u32| n as f32 * 100.0 / total as f32;
    (pct(data.clip_high_count), pct(data.clip_low_count))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hist(luma: [u32; 256]) -> HistogramData {
        let total: u32 = luma.iter().sum();
        HistogramData {
            luma,
            r: [0; 256],
            g: [0; 256],
            b: [0; 256],
            clip_high_count: total / 4,
            clip_low_count: total / 8,
        }
    }

    #[test]
    fn test_downsample_preserves_total_and_falls_back_on_bad_bins() {
        let mut src = [0u32; 256];
        for (i, v) in src.iter_mut().enumerate() {
            *v = i as u32;
        }
        let bins = downsample(&src, HIST_BINS);
        assert_eq!(bins.len(), HIST_BINS);
        // 合并只做加法：总数不变（256 桶的 0..255 和）
        assert_eq!(bins.iter().sum::<u32>(), src.iter().sum::<u32>());
        // 首桶 = 0..=3 的和，末桶 = 252..=255 的和
        assert_eq!(bins[0], 0 + 1 + 2 + 3);
        assert_eq!(
            *bins.last().unwrap(),
            252 + 253 + 254 + 255
        );
        // 非法 bins：不合并、不丢数据
        assert_eq!(downsample(&src, 0).len(), 256);
        assert_eq!(downsample(&src, 100).len(), 256);
    }

    #[test]
    fn test_luma_bins_uses_luma_channel() {
        let mut luma = [0u32; 256];
        luma[255] = 1000;
        let data = hist(luma);
        let bins = luma_bins(&data);
        // 全部落在最后一桶（高光端）
        assert_eq!(*bins.last().unwrap(), 1000);
        assert_eq!(bins[0], 0);
    }

    #[test]
    fn test_bin_heights_share_peak_between_current_and_baseline() {
        let cur = vec![100u32, 0, 0, 0];
        let base = vec![0u32, 0, 0, 400];
        let h = bin_heights(&cur, Some(&base), 64.0);
        // 公共峰值是 400 → 当前 100 的高度 = sqrt(100)/sqrt(400)*64 = 32
        assert!((h[0] - 32.0).abs() < 1e-3, "{:?}", h);
        let hb = bin_heights(&base, Some(&cur), 64.0);
        assert!((hb[3] - 64.0).abs() < 1e-3, "{:?}", hb);
        // 不给基线时按自身峰值满高
        let solo = bin_heights(&cur, None, 64.0);
        assert!((solo[0] - 64.0).abs() < 1e-3);
    }

    #[test]
    fn test_bin_heights_all_zero_and_empty() {
        let zeros = vec![0u32; 8];
        assert_eq!(bin_heights(&zeros, None, 64.0), vec![0.0; 8]);
        assert_eq!(bin_heights(&zeros, Some(&zeros), 64.0), vec![0.0; 8]);
    }

    #[test]
    fn test_clip_percent_and_zero_total() {
        let mut luma = [0u32; 256];
        luma[0] = 50;
        luma[128] = 150;
        luma[255] = 300;
        // total = 500，高光 150（30%）、死黑 100（20%）——这里按 hist() 的构造口径
        let mut data = hist(luma);
        data.clip_high_count = 150;
        data.clip_low_count = 100;
        let (high, low) = clip_percent(&data);
        assert!((high - 30.0).abs() < 1e-3, "{high}");
        assert!((low - 20.0).abs() < 1e-3, "{low}");
        // 零像素：给 0 而不是 NaN
        let empty = HistogramData {
            luma: [0; 256],
            r: [0; 256],
            g: [0; 256],
            b: [0; 256],
            clip_high_count: 0,
            clip_low_count: 0,
        };
        assert_eq!(clip_percent(&empty), (0.0, 0.0));
    }
}
