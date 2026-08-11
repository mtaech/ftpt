//! 批量操作撤销日志（单槽：只记最近一次批量操作的逆操作）。
//!
//! 覆盖移动/复制/重命名的逆操作（删除走回收站，不在撤销范围）：
//! - 移动 A→B 的逆 = 把 B 移回 A
//! - 重命名 A→B 的逆 = 改回原名
//! - 复制 A→B 的逆 = 删除副本 B（仅当副本仍存在且源文件也仍在，否则跳过并报告）
//!
//! 全同步；错误处理复用 [`crate::ops::OpError`]（thiserror）。
//! 日志仅存内存（重启失效可接受），由 Tauri 端 AppState 持有。

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::ops::OpError;

/// 单条逆向操作（文件粒度；from/to 均为完整路径）
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UndoOp {
    /// 移动：from → to（逆操作 = 把 to 移回 from）
    Move { from: PathBuf, to: PathBuf },
    /// 重命名：from → to（逆操作 = 把 to 改回 from）
    Rename { from: PathBuf, to: PathBuf },
    /// 复制：from → to（逆操作 = 删除副本 to）
    Copy { from: PathBuf, to: PathBuf },
}

impl UndoOp {
    /// 撤销时操作的目标路径（副本/移动后的位置）
    pub fn target(&self) -> &Path {
        match self {
            UndoOp::Move { to, .. } | UndoOp::Rename { to, .. } | UndoOp::Copy { to, .. } => to,
        }
    }

    /// 撤销时应恢复的源路径
    pub fn origin(&self) -> &Path {
        match self {
            UndoOp::Move { from, .. } | UndoOp::Rename { from, .. } | UndoOp::Copy { from, .. } => {
                from
            }
        }
    }
}

/// 单条撤销的结果（path 用于前端报告「跳过/失败的是哪个文件」）
#[derive(Debug)]
pub struct UndoOutcome {
    pub op: UndoOp,
    pub result: Result<(), UndoError>,
}

/// 撤销失败/跳过原因
#[derive(Error, Debug)]
pub enum UndoError {
    /// 条件不满足而跳过（不算错误，前端按「跳过」展示原因）
    #[error("跳过：{0}")]
    Skipped(String),
    /// 执行失败（IO 等，复用 ops 错误）
    #[error(transparent)]
    Failed(#[from] OpError),
}

/// 批量操作日志：只存最近一次批量操作的逆操作（单槽，后记录覆盖先记录）
#[derive(Debug, Default)]
pub struct OpJournal {
    ops: Vec<UndoOp>,
}

impl OpJournal {
    pub fn new() -> Self {
        Self::default()
    }

    /// 当前是否有可撤销的记录
    pub fn has_pending(&self) -> bool {
        !self.ops.is_empty()
    }

    /// 记录一批逆操作（覆盖旧批次，单槽语义）
    pub fn record(&mut self, ops: Vec<UndoOp>) {
        self.ops = ops;
    }

    /// 清空日志（无可撤销操作）
    pub fn clear(&mut self) {
        self.ops.clear();
    }

    /// 取走逆操作并清空日志（命令异步执行时先取快照再 spawn_blocking）
    pub fn take(&mut self) -> Vec<UndoOp> {
        std::mem::take(&mut self.ops)
    }

    /// 撤销最近一批：取走日志并逐条执行逆操作（跳过/失败不中止后续）
    pub fn undo_last(&mut self) -> Vec<UndoOutcome> {
        let ops = self.take();
        undo_ops(&ops)
    }
}

/// 执行一批逆操作，逐条返回结果（跳过/失败不中止后续条目）
pub fn undo_ops(ops: &[UndoOp]) -> Vec<UndoOutcome> {
    ops.iter()
        .map(|op| UndoOutcome {
            op: op.clone(),
            result: undo_one(op),
        })
        .collect()
}

fn undo_one(op: &UndoOp) -> Result<(), UndoError> {
    match op {
        UndoOp::Move { from, to } | UndoOp::Rename { from, to } => {
            // 逆操作前提：目标存在、原位置未被占用（防覆盖已有文件造成数据丢失）
            if !to.exists() {
                return Err(UndoError::Skipped(format!("文件不存在: {}", to.display())));
            }
            if from.exists() {
                return Err(UndoError::Skipped(format!(
                    "原位置已有文件，为避免覆盖跳过: {}",
                    from.display()
                )));
            }
            if matches!(op, UndoOp::Move { .. }) {
                // 移动：与 ops::move_capture 一致，跨设备（EXDEV）走 copy + delete 回退
                move_file(to, from)?;
            } else {
                // 重命名：同目录改名，普通 rename 即可
                std::fs::rename(to, from).map_err(OpError::from)?;
            }
            Ok(())
        }
        UndoOp::Copy { from, to } => {
            // 副本必须仍存在；源文件也必须仍在（源已删 → 保留副本，跳过）
            if !to.exists() {
                return Err(UndoError::Skipped(format!("副本不存在: {}", to.display())));
            }
            if !from.exists() {
                return Err(UndoError::Skipped(format!(
                    "源文件已不存在，保留副本: {}",
                    to.display()
                )));
            }
            std::fs::remove_file(to).map_err(OpError::from)?;
            Ok(())
        }
    }
}

/// 移动文件：优先 rename，跨文件系统（EXDEV）回退 copy + 删除源（对齐 ops::move_capture）
fn move_file(src: &Path, dest: &Path) -> Result<(), OpError> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match std::fs::rename(src, dest) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::CrossesDevices => {
            // 目标已存在时报错，避免 fs::copy 静默覆盖已有目标后再删源（同 move_capture）
            if dest.exists() {
                return Err(OpError::Io(std::io::Error::new(
                    ErrorKind::AlreadyExists,
                    format!("目标文件已存在: {}", dest.display()),
                )));
            }
            std::fs::copy(src, dest)?;
            std::fs::remove_file(src)?;
            Ok(())
        }
        Err(e) => Err(OpError::Io(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// 在 dir 下写一个内容确定的文件
    fn write_file(dir: &TempDir, name: &str) -> PathBuf {
        let p = dir.path().join(name);
        std::fs::write(&p, format!("data:{name}")).unwrap();
        p
    }

    fn is_skipped(result: &Result<(), UndoError>) -> bool {
        matches!(result, Err(UndoError::Skipped(_)))
    }

    #[test]
    fn test_undo_move_restores_file() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        let from = write_file(&src, "a.jpg");
        let to = dst.path().join("a.jpg");

        // 模拟正向移动
        std::fs::rename(&from, &to).unwrap();
        assert!(!from.exists() && to.exists());

        let mut journal = OpJournal::new();
        journal.record(vec![UndoOp::Move {
            from: from.clone(),
            to: to.clone(),
        }]);

        let outcomes = journal.undo_last();
        assert_eq!(outcomes.len(), 1);
        assert!(outcomes[0].result.is_ok(), "{:?}", outcomes[0].result);
        // 文件回到原位置
        assert!(from.exists());
        assert!(!to.exists());
        assert_eq!(std::fs::read_to_string(&from).unwrap(), "data:a.jpg");
        // 撤销后日志已空（单槽）
        assert!(!journal.has_pending());
    }

    #[test]
    fn test_undo_rename_restores_name() {
        let dir = TempDir::new().unwrap();
        let from = write_file(&dir, "DSC_0001.jpg");
        let to = dir.path().join("旅行_001.jpg");

        std::fs::rename(&from, &to).unwrap();
        let mut journal = OpJournal::new();
        journal.record(vec![UndoOp::Rename {
            from: from.clone(),
            to: to.clone(),
        }]);

        let outcomes = journal.undo_last();
        assert!(outcomes[0].result.is_ok(), "{:?}", outcomes[0].result);
        assert!(from.exists());
        assert!(!to.exists());
    }

    #[test]
    fn test_undo_copy_removes_copy_keeps_source() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        let from = write_file(&src, "a.jpg");
        let to = dst.path().join("a.jpg");

        std::fs::copy(&from, &to).unwrap();
        let mut journal = OpJournal::new();
        journal.record(vec![UndoOp::Copy {
            from: from.clone(),
            to: to.clone(),
        }]);

        let outcomes = journal.undo_last();
        assert!(outcomes[0].result.is_ok(), "{:?}", outcomes[0].result);
        // 副本被删除、源文件保留
        assert!(!to.exists());
        assert!(from.exists());
    }

    #[test]
    fn test_undo_copy_skips_when_source_deleted() {
        // 源已删：副本应被保留（跳过并报告原因）
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        let from = write_file(&src, "a.jpg");
        let to = dst.path().join("a.jpg");
        std::fs::copy(&from, &to).unwrap();
        std::fs::remove_file(&from).unwrap();

        let mut journal = OpJournal::new();
        journal.record(vec![UndoOp::Copy {
            from,
            to: to.clone(),
        }]);

        let outcomes = journal.undo_last();
        assert_eq!(outcomes.len(), 1);
        assert!(is_skipped(&outcomes[0].result), "{:?}", outcomes[0].result);
        // 副本仍在
        assert!(to.exists());
        let reason = match &outcomes[0].result {
            Err(UndoError::Skipped(r)) => r.clone(),
            _ => unreachable!(),
        };
        assert!(reason.contains("源文件已不存在"), "原因应说明保留副本: {reason}");
    }

    #[test]
    fn test_undo_copy_skips_when_copy_missing() {
        // 副本不存在（复制本就失败/副本已被删）：跳过
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        let from = write_file(&src, "a.jpg");
        let to = dst.path().join("a.jpg"); // 从未创建

        let mut journal = OpJournal::new();
        journal.record(vec![UndoOp::Copy {
            from,
            to: to.clone(),
        }]);

        let outcomes = journal.undo_last();
        assert!(is_skipped(&outcomes[0].result), "{:?}", outcomes[0].result);
    }

    #[test]
    fn test_undo_move_skips_when_target_missing() {
        // 移动目标不存在（正向移动失败/文件已被再次移走）：跳过
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        let from = write_file(&src, "a.jpg");
        let to = dst.path().join("a.jpg"); // 从未移动过去

        let mut journal = OpJournal::new();
        journal.record(vec![UndoOp::Move { from, to }]);
        let outcomes = journal.undo_last();
        assert!(is_skipped(&outcomes[0].result), "{:?}", outcomes[0].result);
    }

    #[test]
    fn test_undo_move_skips_when_origin_occupied() {
        // 原位置已被新文件占用：跳过，避免覆盖（数据安全）
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        let from = write_file(&src, "a.jpg");
        let to = dst.path().join("a.jpg");
        std::fs::rename(&from, &to).unwrap();
        // 原位置出现新文件（用户已放回同名文件）
        let occupant = write_file(&src, "a.jpg");

        let mut journal = OpJournal::new();
        journal.record(vec![UndoOp::Move {
            from: from.clone(),
            to: to.clone(),
        }]);
        let outcomes = journal.undo_last();
        assert!(is_skipped(&outcomes[0].result), "{:?}", outcomes[0].result);
        assert!(to.exists(), "副本不应被覆盖删除");
        assert_eq!(
            std::fs::read_to_string(&from).unwrap(),
            std::fs::read_to_string(&occupant).unwrap(),
            "原位置文件应保持不动"
        );
    }

    #[test]
    fn test_undo_partial_failure_report() {
        // 部分失败报告：一条成功 + 一条跳过 + 一条失败，逐条独立报告、互不中止
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();

        // ① 可成功撤销的移动
        let ok_from = write_file(&src, "ok.jpg");
        let ok_to = dst.path().join("ok.jpg");
        std::fs::rename(&ok_from, &ok_to).unwrap();

        // ② 目标不存在 → 跳过
        let skip_to = dst.path().join("never_moved.jpg");

        // ③ 原位置被占用 → 跳过（防覆盖）
        let clash_from = write_file(&src, "clash.jpg");
        let clash_to = dst.path().join("clash.jpg");
        std::fs::rename(&clash_from, &clash_to).unwrap();
        write_file(&src, "clash.jpg");

        let mut journal = OpJournal::new();
        journal.record(vec![
            UndoOp::Move {
                from: ok_from.clone(),
                to: ok_to.clone(),
            },
            UndoOp::Move {
                from: src.path().join("never_moved.jpg"),
                to: skip_to,
            },
            UndoOp::Move {
                from: clash_from,
                to: clash_to,
            },
        ]);

        let outcomes = journal.undo_last();
        assert_eq!(outcomes.len(), 3);
        // ① 成功
        assert!(outcomes[0].result.is_ok(), "{:?}", outcomes[0].result);
        assert!(ok_from.exists() && !ok_to.exists());
        // ② 跳过（目标不存在）
        assert!(is_skipped(&outcomes[1].result), "{:?}", outcomes[1].result);
        // ③ 跳过（原位置占用）
        assert!(is_skipped(&outcomes[2].result), "{:?}", outcomes[2].result);
        // 部分失败不中止后续条目（① 成功后 ②③ 仍被逐条处理）
        assert!(matches!(
            &outcomes[2].result,
            Err(UndoError::Skipped(r)) if r.contains("已有文件")
        ));
    }

    #[test]
    fn test_journal_single_slot_overwrites() {
        // 单槽语义：新批次覆盖旧批次，撤销只作用于最近一次
        let src = TempDir::new().unwrap();
        let a_from = write_file(&src, "a.jpg");
        let a_to = src.path().join("a_moved.jpg");
        std::fs::rename(&a_from, &a_to).unwrap();

        let b_from = write_file(&src, "b.jpg");
        let b_to = src.path().join("b_moved.jpg");
        std::fs::rename(&b_from, &b_to).unwrap();

        let mut journal = OpJournal::new();
        journal.record(vec![UndoOp::Rename {
            from: a_from.clone(),
            to: a_to.clone(),
        }]);
        journal.record(vec![UndoOp::Rename {
            from: b_from.clone(),
            to: b_to.clone(),
        }]);

        let outcomes = journal.undo_last();
        assert_eq!(outcomes.len(), 1);
        // 只有 b 被改回；a 仍处于移动后状态
        assert!(b_from.exists() && !b_to.exists());
        assert!(!a_from.exists() && a_to.exists());
    }
}
