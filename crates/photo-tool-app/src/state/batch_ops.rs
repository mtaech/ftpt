use std::path::Path;

use gpui::*;

use super::app::RootView;

// 批量文件操作（自 state/app.rs 拆出，纯移动，无逻辑改动）

impl RootView {
    /// 弹出目录选择对话框，选择对比目录用于批量文件操作
    pub fn pick_batch_compare_dir(&mut self, cx: &mut Context<Self>) {
        self.worker.spawn(
            cx,
            move || rfd::FileDialog::new().pick_folder(),
            move |this, result, cx| {
                if let Some(path) = result {
                    this.batch_compare_dir = path.display().to_string();
                    cx.notify();
                }
            },
        );
    }

    /// 执行批量文件操作（在工作线程中运行，不阻塞 UI）
    pub fn execute_batch_ops(&mut self, cx: &mut Context<Self>) {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;

        let compare_dir = self.batch_compare_dir.clone();
        let source_format = self.batch_source_format.clone();
        let compare_format = self.batch_compare_format.clone();
        let op_type = self.batch_op_type;
        let source_dir = self.dir_path.clone();

        if compare_dir.is_empty() {
            self.batch_results = vec!["请先选择对比目录".into()];
            cx.notify();
            return;
        }
        let Some(ref src_dir) = source_dir else {
            self.batch_results = vec!["请先打开一个目录".into()];
            cx.notify();
            return;
        };

        self.batch_in_progress = true;
        self.batch_results.clear();
        self.batch_progress = Some((0, 1));
        self.batch_progress_msg = "正在扫描...".into();
        cx.notify();

        let src_dir = src_dir.clone();
        let progress = Arc::new(AtomicU32::new(0));
        let total = Arc::new(AtomicU32::new(0));
        let progress_poll = progress.clone();
        let total_poll = total.clone();

        self.worker.spawn(
            cx,
            move || {
                // 1. 扫描源目录
                let source_captures = match photo_engine::scanner::scan_directory(
                    &src_dir,
                    &Default::default(),
                    None,
                ) {
                    Ok(caps) => caps,
                    Err(e) => return (vec![format!("扫描源目录失败: {e}")], progress, total),
                };

                if source_captures.is_empty() {
                    return (vec!["源目录无匹配文件".into()], progress, total);
                }

                // 2. 查找匹配
                let (matched, unmatched) = match photo_engine::batch_ops::find_matching(
                    &source_captures,
                    Path::new(&compare_dir),
                    if source_format.is_empty() { None } else { Some(source_format.as_str()) },
                    if compare_format.is_empty() { None } else { Some(compare_format.as_str()) },
                ) {
                    Ok(result) => result,
                    Err(e) => return (vec![format!("匹配失败: {e}")], progress, total),
                };

                let indices = if op_type.is_same_match() { matched } else { unmatched };
                if indices.is_empty() {
                    let hint = if op_type.is_same_match() {
                        "对比目录中没有同名文件"
                    } else {
                        "对比目录中没有非同名的文件"
                    };
                    return (vec![hint.into()], progress, total);
                }

                total.store(indices.len() as u32, Ordering::Relaxed);

                let target_dir = if op_type.needs_target_dir() {
                    Some(Path::new(&compare_dir).parent().unwrap_or(Path::new(".")).join("batch_output"))
                } else {
                    None
                };

                let p = progress.clone();
                let results = photo_engine::batch_ops::execute(
                    &source_captures,
                    &indices,
                    op_type,
                    target_dir.as_deref(),
                    move |done, _tot| {
                        p.store(done, Ordering::Relaxed);
                    },
                );
                (results, progress, total)
            },
            move |this, (results, _progress, total), cx| {
                this.batch_in_progress = false;
                this.batch_progress = Some((total.load(Ordering::Relaxed), total.load(Ordering::Relaxed)));
                this.batch_results = results;
                this.batch_progress_msg.clear();
                cx.notify();
            },
        );

        // 轮询进度：每 300ms 检查 atomic 计数器并更新 UI
        let vh = cx.entity().downgrade();
        cx.spawn(|_: WeakEntity<RootView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(300))
                        .await;
                    let done = progress_poll.load(Ordering::Relaxed);
                    let tot = total_poll.load(Ordering::Relaxed);
                    if let Some(view) = vh.upgrade() {
                        let _ = cx.update_entity(&view, |view: &mut RootView, cx: &mut Context<RootView>| {
                            if view.batch_in_progress {
                                view.batch_progress = Some((done, tot));
                                view.batch_progress_msg = format!("处理中: {done}/{tot}");
                                cx.notify();
                            }
                        });
                    } else {
                        break;
                    }
                }
            }
        })
        .detach();
    }
}
