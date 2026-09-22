//! eBird 记录导出的入口门控（手册 §10.6 统计页「导出记录 → CSV」）。
//!
//! 引擎侧 `export_ebird::build_rows(dir)` 按文件夹汇总 **全部有物种结论**的记录
//! （Confirmed / NeedsReview 且带物种名），而 eBird 只收录鸟类——但 `recognition` 表
//! 不持久化类群（`ranks` 见 docs/todo.md #11，暂缓），所以 UI 无法可靠区分鸟与非鸟。
//! 折中：入口门控退一步用「当前目录有可导出的物种结论」，非鸟记录由用户自行剔除
//! （弹窗 tooltip 与 docs 都写明这一点；要真按类群门控，得先把类群落库）。
//!
//! 纯函数、不依赖 GPUI，可单元测试。

use photo_domain::{CaptureMeta, RecognitionStatus};

/// 可导出为 eBird CSV 的照片数：识别状态为 Confirmed / NeedsReview 且带物种名。
///
/// 与引擎 `build_rows` 的计入口径保持一致（Unrecognized、无物种结论的不计），
/// 所以「门控放行」必然能导出行数 > 0 的 CSV。
pub fn ebird_candidates(items: &[CaptureMeta]) -> usize {
    items
        .iter()
        .filter(|m| {
            matches!(
                m.recognition_status,
                Some(RecognitionStatus::Confirmed) | Some(RecognitionStatus::NeedsReview)
            ) && m.taxon_name.as_deref().is_some_and(|n| !n.is_empty())
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use photo_domain::Capture;

    fn meta(name: &str) -> CaptureMeta {
        let capture = Capture {
            base_name: name.to_string(),
            source_files: Vec::new(),
            primary_index: 0,
        };
        CaptureMeta::from_capture(&capture, 0)
    }

    fn identified(name: &str, status: RecognitionStatus, taxon: Option<&str>) -> CaptureMeta {
        let mut m = meta(name);
        m.recognition_status = Some(status);
        m.taxon_name = taxon.map(|t| t.to_string());
        m
    }

    #[test]
    fn test_confirmed_and_needs_review_count() {
        let items = vec![
            identified("a.jpg", RecognitionStatus::Confirmed, Some("大嘴乌鸦")),
            identified("b.jpg", RecognitionStatus::NeedsReview, Some("白鹭")),
        ];
        assert_eq!(ebird_candidates(&items), 2);
    }

    #[test]
    fn test_unrecognized_or_missing_taxon_not_counted() {
        let items = vec![
            identified("a.jpg", RecognitionStatus::Unrecognized, Some("大嘴乌鸦")),
            identified("b.jpg", RecognitionStatus::Confirmed, None),
            identified("c.jpg", RecognitionStatus::Confirmed, Some("")),
            meta("d.jpg"),
        ];
        assert_eq!(ebird_candidates(&items), 0);
    }
}
