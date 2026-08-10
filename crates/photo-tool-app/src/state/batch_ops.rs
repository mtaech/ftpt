use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use gpui::*;

use photo_domain::BatchOpType;

use super::app::RootView;

// 批量文件操作（ADR 0006：筛选驱动 + 画面粒度）
//
// 交互模型：
// - 操作对象 = 当前筛选结果（display_order），纯筛选驱动
// - [移动到…] [复制到…]：一步式——弹目录选择，选完即执行
// - [删除]：先算「将删除 N 个（其中 M 个来自同名同步）」→ 确认弹窗 → 执行
// - 「同步同名文件」开关 + 格式多选：按 stem 把兄弟文件纳入操作集

/// 删除确认弹窗数据：清单（前 20 条文件名）+ 总数 + 其中同步拉入数
#[derive(Debug, Clone)]
pub struct BatchDeletePreview {
    pub files: Vec<String>,
    pub total: usize,
    pub synced: usize,
}

impl RootView {
    /// 切换「同步同名文件」开关；开启时初始化同步格式为目录全量
    pub fn set_batch_sync_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.batch_sync_enabled = enabled;
        if enabled && self.batch_sync_formats.is_empty() {
            // 默认全选：与 engine 匹配键一致（ImageFormat Display 大写，如 JPEG/NEF）
            let fmts: HashSet<String> = self
                .captures
                .iter()
                .map(|m| m.primary_format.to_string().to_uppercase())
                .collect();
            self.batch_sync_formats = fmts;
        }
        self.refresh_batch_sync_extra();
        cx.notify();
    }

    /// 勾选/取消某个同步格式
    pub fn toggle_batch_sync_format(&mut self, fmt: &str, cx: &mut Context<Self>) {
        if !self.batch_sync_formats.remove(fmt) {
            self.batch_sync_formats.insert(fmt.to_string());
        }
        self.refresh_batch_sync_extra();
        cx.notify();
    }

    /// 重算同步预估额外文件数（UI 线程 O(n) 哈希，毫秒级）；筛选变化后也需调用
    pub fn refresh_batch_sync_extra(&mut self) {
        self.batch_sync_extra = self.compute_sync_extra();
    }

    fn meta_in_sync_formats(&self, meta: &photo_domain::CaptureMeta) -> bool {
        if self.batch_sync_formats.is_empty() {
            return false;
        }
        let fmt = meta.primary_format.to_string().to_uppercase();
        self.batch_sync_formats.contains(&fmt)
    }

    /// 同步会额外拉入的文件数（不在筛选集内的同名兄弟）
    fn compute_sync_extra(&self) -> usize {
        if !self.batch_sync_enabled {
            return 0;
        }
        let mut by_stem: HashMap<&str, Vec<usize>> = HashMap::new();
        for (i, m) in self.captures.iter().enumerate() {
            if self.meta_in_sync_formats(m) {
                by_stem.entry(m.base_name.as_str()).or_default().push(i);
            }
        }
        let in_order: HashSet<usize> = self.display_order.iter().copied().collect();
        let mut extra = 0;
        for &ci in &self.display_order {
            let Some(meta) = self.captures.get(ci) else { continue };
            if let Some(sibs) = by_stem.get(meta.base_name.as_str()) {
                extra += sibs.iter().filter(|s| !in_order.contains(s)).count();
            }
        }
        extra
    }

    /// 点「删除」：计算将删除的文件（筛选集 + 同步扩展），打开确认弹窗
    pub fn batch_delete(&mut self, cx: &mut Context<Self>) {
        if self.display_order.is_empty() {
            return;
        }
        let (total, synced, files) = self.compute_delete_preview();
        self.batch_delete_confirm = Some(BatchDeletePreview { files, total, synced });
        cx.notify();
    }

    fn compute_delete_preview(&self) -> (usize, usize, Vec<String>) {
        let in_order: HashSet<usize> = self.display_order.iter().copied().collect();
        let mut by_stem: HashMap<&str, Vec<usize>> = HashMap::new();
        if self.batch_sync_enabled {
            for (i, m) in self.captures.iter().enumerate() {
                if self.meta_in_sync_formats(m) {
                    by_stem.entry(m.base_name.as_str()).or_default().push(i);
                }
            }
        }
        // 顺序：筛选集在前，同步兄弟在后（去重）
        let mut order: Vec<usize> = Vec::new();
        let mut seen: HashSet<usize> = HashSet::new();
        for &ci in &self.display_order {
            if seen.insert(ci) {
                order.push(ci);
            }
        }
        let mut synced = 0;
        if self.batch_sync_enabled {
            for &ci in &self.display_order {
                let Some(meta) = self.captures.get(ci) else { continue };
                if let Some(sibs) = by_stem.get(meta.base_name.as_str()) {
                    for &s in sibs {
                        if !in_order.contains(&s) && seen.insert(s) {
                            order.push(s);
                            synced += 1;
                        }
                    }
                }
            }
        }
        let total = order.len();
        let files: Vec<String> = order
            .iter()
            .take(20)
            .filter_map(|&ci| {
                self.captures
                    .get(ci)
                    .map(|m| m.display_name())
            })
            .collect();
        (total, synced, files)
    }

    /// 确认删除（弹窗点确认后）
    pub fn confirm_batch_delete(&mut self, cx: &mut Context<Self>) {
        self.batch_delete_confirm = None;
        self.run_batch_op(BatchOpType::Delete, None, cx);
    }

    /// 取消删除弹窗
    pub fn cancel_batch_delete(&mut self, cx: &mut Context<Self>) {
        self.batch_delete_confirm = None;
        cx.notify();
    }

    /// 点「移动到…」/「复制到…」：一步式——弹目录选择，选完即执行
    pub fn batch_move_or_copy(&mut self, op: BatchOpType, cx: &mut Context<Self>) {
        let src_dir = self.dir_path.clone();
        self.worker.spawn(
            cx,
            move || rfd::FileDialog::new().pick_folder(),
            move |this, result, cx| {
                let Some(dir) = result else { return };
                // 校验：目标不能与源目录相同
                if Some(&dir) == src_dir.as_ref() {
                    this.show_toast("目标目录不能与当前目录相同", cx);
                    cx.notify();
                    return;
                }
                this.run_batch_op(op, Some(dir), cx);
            },
        );
    }

    /// 执行批量操作（worker 线程）：重扫源目录 → 按路径定位筛选集 → 同步扩展 → 执行
    fn run_batch_op(&mut self, op: BatchOpType, target_dir: Option<PathBuf>, cx: &mut Context<Self>) {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;

        // 无筛选条件拒绝执行：操作集 = 全部文件，防误操作（UI 已禁用，此处兜底）
        if !self.filter.has_active_filter() {
            self.show_toast("未设置筛选条件，已拒绝执行（请先在筛选栏设置条件）", cx);
            cx.notify();
            return;
        }

        let Some(src_dir) = self.dir_path.clone() else { return };
        let paths: Vec<PathBuf> = self
            .display_order
            .iter()
            .filter_map(|&ci| self.captures.get(ci))
            .map(|m| PathBuf::from(&m.primary_path))
            .collect();
        if paths.is_empty() {
            self.show_toast("当前筛选结果为空", cx);
            cx.notify();
            return;
        }
        let sync_enabled = self.batch_sync_enabled;
        let sync_formats = self.batch_sync_formats.clone();
        let generation = self.scan_generation;

        self.batch_in_progress = true;
        self.batch_results.clear();
        self.batch_progress = Some((0, 1));
        self.batch_progress_msg = "正在扫描...".into();
        cx.notify();

        let progress = Arc::new(AtomicU32::new(0));
        let total = Arc::new(AtomicU32::new(0));
        let progress_poll = progress.clone();
        let total_poll = total.clone();

        self.worker.spawn(
            cx,
            move || {
                // 1. 重扫源目录，取完整 Capture（ops 层需要 source_files）
                let caps = match photo_engine::scanner::scan_directory(
                    &src_dir,
                    &Default::default(),
                    None,
                ) {
                    Ok(c) => c,
                    Err(e) => return (vec![format!("扫描失败: {e}")], progress, total),
                };
                // 2. 按 primary_path 定位筛选集索引（重扫索引 ≠ app 层索引）
                let by_path: HashMap<PathBuf, usize> = caps
                    .iter()
                    .enumerate()
                    .map(|(i, c)| (c.source_files[c.primary_index].path.clone(), i))
                    .collect();
                let mut indices: Vec<usize> =
                    paths.iter().filter_map(|p| by_path.get(p).copied()).collect();
                if indices.is_empty() {
                    return (vec!["当前筛选结果在目录中无匹配文件".into()], progress, total);
                }
                // 3. 同步扩展：按 stem 并入兄弟文件
                if sync_enabled && !sync_formats.is_empty() {
                    indices =
                        photo_engine::batch_ops::expand_with_siblings(&caps, &indices, &sync_formats);
                }
                total.store(indices.len() as u32, Ordering::Relaxed);
                // 4. 执行
                let p = progress.clone();
                let results = photo_engine::batch_ops::execute(
                    &caps,
                    &indices,
                    op,
                    target_dir.as_deref(),
                    move |done, _tot| {
                        p.store(done, Ordering::Relaxed);
                    },
                );
                (results, progress, total)
            },
            move |this, (results, _progress, total), cx| {
                // B4：无条件先复位哨兵与轮询终止条件——即使代际不匹配也必须解除
                // batch_in_progress，否则 300ms 轮询任务永不退出、面板永久「执行中」
                this.batch_in_progress = false;
                this.batch_progress_msg.clear();
                if generation != this.scan_generation {
                    // 重扫导致代际不匹配：结果基于旧索引，丢弃，不应用不刷新
                    tracing::warn!(
                        "批量操作结果代际不匹配（{} != {}），丢弃结果",
                        generation,
                        this.scan_generation
                    );
                    cx.notify();
                    return;
                }
                this.batch_progress =
                    Some((total.load(Ordering::Relaxed), total.load(Ordering::Relaxed)));
                this.batch_results = results;
                // 完成反馈：toast 摘要
                let ok = this.batch_results.iter().filter(|m| !m.contains("失败")).count();
                let fail = this.batch_results.len() - ok;
                this.show_toast(
                    format!("{}完成：成功 {} / 失败 {}", op.action_label(), ok, fail),
                    cx,
                );
                // 移动/删除改变目录内容 → 全量重扫刷新（复制不影响源列表）
                if op != BatchOpType::Copy
                    && let Some(dir) = this.dir_path.clone()
                {
                    this.scan_directory(dir, cx);
                }
                cx.notify();
            },
        );

        // 轮询进度：每 300ms 检查 atomic 计数器并更新 UI；
        // on_done 置 batch_in_progress=false 后轮询退出，不再永久空转
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
                    let Some(view) = vh.upgrade() else {
                        break;
                    };
                    let running = cx.update_entity(
                        &view,
                        |view: &mut RootView, cx: &mut Context<RootView>| {
                            if !view.batch_in_progress {
                                return false;
                            }
                            view.batch_progress = Some((done, tot));
                            view.batch_progress_msg = format!("处理中: {done}/{tot}");
                            cx.notify();
                            true
                        },
                    );
                    if !running {
                        break;
                    }
                }
            }
        })
        .detach();
    }
}
