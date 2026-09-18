//! 导入（SD 卡 / 目录 → 按日期建目录 → 去重 → 复制/移动）的前端状态与任务编排。
//!
//! 对照 photo-tauri 的 ImportDialog 与手册 §10.3：
//!   ① 选源（可移动盘列表 / 浏览目录）→ 扫描（只 stat 取文件时间，不读 EXIF）
//!   ② 目标根目录（选择或手输，不存在自动建）+ 子目录模式 + 重命名模板
//!   ③ 复制 / 移动
//!   ④ 干跑计划预览（N 张 → M 个目标目录 + 跳过清单前 20 条）
//!   ⑤ 执行 → 进度 → 全部成功自动关闭并打开结果目录；有失败留在面板看明细

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui_kit::{App, Context, Entity, Window};
use photo_engine::import::{
    DriveInfo, ImportCandidate, ImportMode, ImportPlan, ImportSubfolder,
    execute_import as engine_execute_import, plan_import as engine_plan_import,
    scan_import_source as engine_scan_import_source,
};

use super::app_state::AppState;
use super::engine_ops::{defer_entity_action, start_scan};

/// 顶部段：导入（搬运文件）/ 添加（只打开目录浏览，不动文件）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImportTab {
    #[default]
    Import,
    Add,
}

/// 执行结果汇总（成功 / 跳过 / 失败）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ImportOutcome {
    pub imported: u32,
    pub skipped: u32,
    pub failed: u32,
}

/// 导入弹窗状态（弹窗每次打开回到干净初值）。
#[derive(Debug, Clone)]
pub struct ImportState {
    pub tab: ImportTab,
    /// 检测到的可移动驱动器（打开时刷新）
    pub drives: Vec<DriveInfo>,
    /// 已选源（驱动器根或手动浏览目录）
    pub source: Option<PathBuf>,
    /// 扫描出的候选
    pub candidates: Vec<ImportCandidate>,
    pub scanning: bool,
    /// 目标根目录（选择或手输）
    pub dest_root: String,
    /// 子目录模式（默认按日期 YYYY-MM-DD）
    pub subfolder: ImportSubfolder,
    /// 重命名模板（空 = 保留原名）
    pub rename_template: String,
    /// 复制 / 移动
    pub mode: ImportMode,
    /// 干跑计划（不碰文件）
    pub plan: Option<ImportPlan>,
    pub planning: bool,
    pub running: bool,
    /// 执行进度：(已完成, 总数, 当前文件)
    pub progress: Option<(u32, u32, String)>,
    pub result: Option<ImportOutcome>,
    /// 瞬态错误提示（4 秒后由状态栏/弹窗自行忽略）
    pub error: Option<(String, Instant)>,
    /// 任务世代号：重开弹窗 / 重扫后旧任务结果作废
    pub generation: u64,
}

impl Default for ImportState {
    fn default() -> Self {
        Self {
            tab: ImportTab::Import,
            drives: Vec::new(),
            source: None,
            candidates: Vec::new(),
            scanning: false,
            dest_root: String::new(),
            subfolder: ImportSubfolder::default(),
            rename_template: String::new(),
            mode: ImportMode::Copy,
            plan: None,
            planning: false,
            running: false,
            progress: None,
            result: None,
            error: None,
            generation: 0,
        }
    }
}

impl ImportState {
    /// 打开弹窗时复位；目标根目录预填当前浏览目录（原地导入是最常见用法）。
    pub fn reset(&mut self, current_dir: Option<&Path>) {
        self.generation = self.generation.wrapping_add(1);
        self.tab = ImportTab::Import;
        self.source = None;
        self.candidates.clear();
        self.scanning = false;
        self.dest_root = current_dir
            .map(|d| d.to_string_lossy().to_string())
            .unwrap_or_default();
        self.subfolder = ImportSubfolder::default();
        self.rename_template.clear();
        self.mode = ImportMode::Copy;
        self.plan = None;
        self.planning = false;
        self.running = false;
        self.progress = None;
        self.result = None;
        self.error = None;
        self.drives = photo_engine::import::detect_removable_drives();
    }

    /// 计划里待导入的文件总数。
    pub fn plan_count(&self) -> usize {
        self.plan
            .as_ref()
            .map(|p| p.groups.iter().map(|g| g.files.len()).sum())
            .unwrap_or(0)
    }

    pub fn can_plan(&self) -> bool {
        self.source.is_some()
            && !self.dest_root.trim().is_empty()
            && !self.planning
            && !self.running
            && !self.scanning
    }

    pub fn can_execute(&self) -> bool {
        self.plan.is_some() && !self.running && self.plan_count() > 0
    }

    pub fn set_error(&mut self, message: impl Into<String>) {
        self.error = Some((message.into(), Instant::now()));
    }
}

/// 把两个输入框（目标根目录 / 重命名模板）的当前文本同步进状态。
fn sync_inputs(state: &mut AppState, cx: &App) {
    if let Some(input) = &state.import_dest_input {
        state.import.dest_root = input.read(cx).value().to_string();
    }
    if let Some(input) = &state.import_rename_input {
        state.import.rename_template = input.read(cx).value().to_string();
    }
}

/// 递归扫描选中源（engine 只 stat 取文件修改时间，不读 EXIF）。
pub fn scan_source(entity: Entity<AppState>, cx: &mut App) {
    let prepared = entity.update(cx, |state, cx| {
        let source = state.import.source.clone()?;
        state.import.scanning = true;
        state.import.candidates.clear();
        state.import.plan = None;
        state.import.result = None;
        state.import.error = None;
        cx.notify();
        Some((state.import.generation, source))
    });
    let Some((generation, source)) = prepared else {
        return;
    };

    cx.spawn(async move |async_cx| {
        let result = async_cx
            .background_executor()
            .spawn(async move { engine_scan_import_source(&source) })
            .await;
        async_cx.update(|cx| {
            entity.update(cx, |state, cx| {
                if state.import.generation != generation {
                    return;
                }
                state.import.scanning = false;
                match result {
                    Ok(candidates) => {
                        let count = candidates.len();
                        state.import.candidates = candidates;
                        state.set_status_message(format!("导入源扫描完成：{count} 张"));
                    }
                    Err(err) => state.import.set_error(format!("扫描导入源失败：{err}")),
                }
                cx.notify();
            });
        });
    })
    .detach();
}

/// 干跑计划：按子目录模式分组 + 目标去重（不碰文件）。
pub fn generate_plan(entity: Entity<AppState>, cx: &mut App) {
    let prepared = entity.update(cx, |state, cx| {
        sync_inputs(state, cx);
        if !state.import.can_plan() {
            return None;
        }
        let dest = state.import.dest_root.trim().to_string();
        let options = photo_engine::import::ImportOptions {
            subfolder: state.import.subfolder,
            rename_template: {
                let template = state.import.rename_template.trim().to_string();
                (!template.is_empty()).then_some(template)
            },
        };
        state.import.planning = true;
        state.import.result = None;
        state.import.error = None;
        cx.notify();
        Some((
            state.import.generation,
            state.import.candidates.clone(),
            PathBuf::from(dest),
            options,
        ))
    });
    let Some((generation, candidates, dest, options)) = prepared else {
        return;
    };

    cx.spawn(async move |async_cx| {
        let plan = async_cx
            .background_executor()
            .spawn(async move { engine_plan_import(&candidates, &dest, &options) })
            .await;
        async_cx.update(|cx| {
            entity.update(cx, |state, cx| {
                if state.import.generation != generation {
                    return;
                }
                state.import.planning = false;
                state.import.plan = Some(plan);
                cx.notify();
            });
        });
    })
    .detach();
}

/// 导入成功后要展示的目录：单组 → 该组子目录；多组 → 目标根目录；无分组 → None。
fn imported_dir(plan: &ImportPlan, dest_root: &str) -> Option<PathBuf> {
    if plan.groups.is_empty() {
        return None;
    }
    let root = PathBuf::from(dest_root.trim());
    if plan.groups.len() > 1 {
        return Some(root);
    }
    let sub = plan.groups[0].sub_dir.trim();
    if sub.is_empty() {
        Some(root)
    } else {
        Some(root.join(sub))
    }
}

/// 执行导入：后台逐文件复制/移动，主线程 120ms 轮询进度；全部成功则关闭弹窗并重扫结果目录。
pub fn execute(entity: Entity<AppState>, cx: &mut App) {
    let prepared = entity.update(cx, |state, cx| {
        sync_inputs(state, cx);
        let plan = state.import.plan.clone()?;
        if state.import.running || state.import.plan_count() == 0 {
            return None;
        }
        let dest = state.import.dest_root.trim().to_string();
        if dest.is_empty() {
            state.import.set_error("目标目录未指定");
            return None;
        }
        state.import.running = true;
        state.import.result = None;
        state.import.error = None;
        state.import.progress = Some((0, state.import.plan_count() as u32, String::new()));
        cx.notify();
        Some((state.import.generation, plan, dest, state.import.mode))
    });
    let Some((generation, plan, dest_text, mode)) = prepared else {
        return;
    };

    let dest = PathBuf::from(&dest_text);
    // 目标根目录不存在时自动创建（手册 §10.3 ②）
    if let Err(err) = std::fs::create_dir_all(&dest) {
        entity.update(cx, |state, cx| {
            state.import.running = false;
            state.import.progress = None;
            state.import.set_error(format!("创建目标目录失败：{err}"));
            cx.notify();
        });
        return;
    }

    let skipped = plan.skipped.len() as u32;
    let dir_to_open = imported_dir(&plan, &dest_text);
    let progress_slot: Arc<Mutex<Option<(u32, u32, String)>>> = Arc::new(Mutex::new(None));
    let done = Arc::new(AtomicBool::new(false));
    let outcome_slot: Arc<Mutex<Option<(u32, u32)>>> = Arc::new(Mutex::new(None));

    let entity_poll = entity.clone();
    cx.spawn(async move |async_cx| {
        let progress_cb = progress_slot.clone();
        let done_cb = done.clone();
        let outcome_cb = outcome_slot.clone();
        let plan_bg = plan.clone();
        let dest_bg = dest.clone();
        let exec = async_cx.background_executor().spawn(async move {
            let results = engine_execute_import(
                &plan_bg,
                &dest_bg,
                mode,
                Some(Box::new(move |p| {
                    *progress_cb.lock().unwrap() =
                        Some((p.done, p.total, p.current.to_string_lossy().to_string()));
                })),
            );
            let imported = results.iter().filter(|(_, r)| r.is_ok()).count() as u32;
            let failed = results.len() as u32 - imported;
            *outcome_cb.lock().unwrap() = Some((imported, failed));
            done_cb.store(true, Ordering::Relaxed);
        });

        loop {
            if done.load(Ordering::Relaxed) {
                break;
            }
            async_cx
                .background_executor()
                .timer(Duration::from_millis(120))
                .await;
            let snapshot = progress_slot.lock().unwrap().clone();
            let Some((done_count, total, current)) = snapshot else {
                continue;
            };
            async_cx.update(|cx| {
                entity_poll.update(cx, |state, cx| {
                    if state.import.generation != generation {
                        return;
                    }
                    state.import.progress = Some((done_count, total, current.clone()));
                    cx.notify();
                });
            });
        }
        let _ = exec.await;

        let (imported, failed) = outcome_slot.lock().unwrap().unwrap_or((0, 0));
        async_cx.update(|cx| {
            entity.update(cx, |state, cx| {
                if state.import.generation != generation {
                    return;
                }
                state.import.running = false;
                state.import.progress = None;
                state.import.result = Some(ImportOutcome {
                    imported,
                    skipped,
                    failed,
                });
                if failed == 0 {
                    let skipped_note = if skipped > 0 {
                        format!("，跳过 {skipped}")
                    } else {
                        String::new()
                    };
                    state.set_status_message(format!("导入完成：成功 {imported} 张{skipped_note}"));
                    state.active_dialog = None;
                } else {
                    state.import.set_error(format!("{failed} 个文件导入失败"));
                }
                cx.notify();
            });
        });

        if failed == 0
            && let Some(dir) = dir_to_open
        {
            async_cx.update(|cx| start_scan(entity, dir, true, cx));
        }
    })
    .detach();
}

/// 浏览源目录（系统目录对话框）→ 设源并立即扫描。
pub fn pick_source(window: &mut Window, cx: &mut Context<AppState>) {
    cx.spawn_in(window, async move |weak, async_cx| {
        let Some(folder) = rfd::AsyncFileDialog::new().pick_folder().await else {
            return;
        };
        let path = folder.path().to_path_buf();
        let _ = async_cx.update(|_window, cx| {
            let Some(entity) = weak.upgrade() else {
                return;
            };
            entity.update(cx, |state, _cx| {
                state.import.source = Some(path.clone());
                state.import.plan = None;
                state.import.result = None;
            });
            scan_source(entity, cx);
        });
    })
    .detach();
}

/// 选择目标根目录（系统目录对话框）→ 回填状态与输入框。
pub fn pick_dest(window: &mut Window, cx: &mut Context<AppState>) {
    cx.spawn_in(window, async move |weak, async_cx| {
        let Some(folder) = rfd::AsyncFileDialog::new().pick_folder().await else {
            return;
        };
        let text = folder.path().to_string_lossy().to_string();
        let _ = async_cx.update(|window, cx| {
            let Some(entity) = weak.upgrade() else {
                return;
            };
            entity.update(cx, |state, cx| {
                state.import.dest_root = text.clone();
                state.import.plan = None;
                state.import.result = None;
                if let Some(input) = state.import_dest_input.clone() {
                    input.update(cx, |input_state, cx| {
                        input_state.set_value(text.clone(), window, cx);
                    });
                }
                cx.notify();
            });
        });
    })
    .detach();
}

/// 「添加」段：选目录直接打开浏览（递归扫描），不动文件。
pub fn pick_add_directory(window: &mut Window, cx: &mut Context<AppState>) {
    cx.spawn_in(window, async move |weak, async_cx| {
        let Some(folder) = rfd::AsyncFileDialog::new().pick_folder().await else {
            return;
        };
        let path = folder.path().to_path_buf();
        let _ = async_cx.update(|_window, cx| {
            let Some(entity) = weak.upgrade() else {
                return;
            };
            entity.update(cx, |state, _cx| {
                state.active_dialog = None;
            });
            start_scan(entity, path, true, cx);
        });
    })
    .detach();
}

/// 视图按钮专用：设源后触发扫描（走 defer，避免 listener 内双重租借）。
pub fn set_source_and_scan(entity: Entity<AppState>, source: PathBuf, cx: &mut Context<AppState>) {
    defer_entity_action(cx, entity.clone(), move |entity, cx| {
        entity.update(cx, |state, _cx| {
            state.import.source = Some(source);
            state.import.candidates.clear();
            state.import.plan = None;
            state.import.result = None;
        });
        scan_source(entity, cx);
    });
}

/// 视图按钮专用：重扫当前源。
pub fn rescan(entity: Entity<AppState>, cx: &mut Context<AppState>) {
    defer_entity_action(cx, entity, scan_source);
}
