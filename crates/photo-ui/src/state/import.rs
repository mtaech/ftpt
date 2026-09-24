//! 导入（SD 卡 / 目录 → 审阅 → 按日期建目录 → 去重 → 复制/移动）的前端状态与任务编排。
//!
//! 对照设计手册 §10.3（原 Tauri 版 ImportDialog，已随 crates/photo-tauri 删除）：
//!   ① 选源（可移动盘列表 / 浏览目录）→ 扫描（只 stat 取文件时间，不读 EXIF）
//!   ② **审阅**：源侧缩略图网格 + 勾选 + 类别过滤（缩略图缓存写配置目录，**不写卡**）
//!   ③ 目标根目录 + 子目录模式 + 重命名模板 + 冲突策略 + 校验 / 完成后弹出
//!   ④ 计划**实时重算**（选项一变 300ms 去抖重算，无「生成计划」按钮）
//!      + 分诊四分类：新 / 已导入 / 冲突 / 已过滤
//!   ⑤ 执行 → 进度（可取消）→ 结果常驻面板：成功 / 跳过 / 失败逐条 + 警告
//!   ⑥ 「重试失败项」按失败源重新干跑再执行；配方记忆让「上次这张卡」一键复用
//!
//! 三层去重（引擎侧）：源卷账本 → 断点日志 → 目标存在性 + 计划内冲突。

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui_kit::{App, Context, Entity, Window};
use photo_domain::SourceFile;
use photo_config::ImportRecipe;
use photo_engine::import::{
    ConflictPolicy, DriveInfo, ImportCandidate, ImportFilter, ImportIssue, ImportMode, ImportPlan,
    ImportReport, ImportSubfolder, PlanExtras, SourceLedger,
    eject_source, execute_import as engine_execute_import, plan_import as engine_plan_import,
    scan_import_source as engine_scan_import_source,
};
use photo_engine::import_history::{
    ImportHistory, ImportJournal, ImportRecord, volume_id_for_path,
};

use crate::image::{ImageManager, MASTER_SIZE, THUMB_SIZE_GRID};

use super::app_state::{ActiveDialog, AppState};
use super::engine_ops::{defer_entity_action, start_scan};

/// 计划去抖窗口：选项一变就等这么久再重算，避免拖滑杆/连打模板时反复干跑
const PLAN_DEBOUNCE_MS: u64 = 300;
/// 审阅缩略图并发 worker 数（与扫描缩略图同一档；读卡器是串行设备，再高也吃不满）
const PREVIEW_WORKERS: usize = 4;

/// 顶部段：导入（搬运文件）/ 添加（只打开目录浏览，不动文件）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImportTab {
    #[default]
    Import,
    Add,
}

/// 执行结果汇总（成功 / 跳过 / 失败 / 警告 / 取消）。
///
/// 失败不再只留计数：界面要能逐条告诉用户「哪个文件、为什么」。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ImportOutcome {
    pub imported: u32,
    pub skipped: u32,
    /// 失败逐条（源路径 + 中文原因）
    pub failed: Vec<ImportIssue>,
    /// 主文件已成功、附带动作失败的记录（mtime 回写 / sidecar 搬运）
    pub warnings: Vec<ImportIssue>,
    /// 是否被用户取消
    pub cancelled: bool,
    /// 取消时未处理的文件数
    pub unprocessed: u32,
}

/// 影响计划内容的取值快照：与当前不一致 = 计划已过期（界面上由实时重算兜住）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionsKey {
    /// 目标根目录（**必须在 key 里**：计划的去重结论依赖目标，换了目标就是另一份计划）
    dest: String,
    subfolder: ImportSubfolder,
    template: String,
    filter: ImportFilter,
    policy: ConflictPolicy,
    verify: bool,
    /// 选择集修订号（计划只包含勾选的候选）
    selection_rev: u64,
}

/// 导入弹窗状态（弹窗每次打开回到干净初值，再套用配方）。
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

    // ── 审阅（Phase 2）──
    /// 勾选的候选下标；`selection_ready == false` 时视为「全选」
    pub selected: HashSet<usize>,
    /// 选择集是否已被用户动过（false = 全选，供「刚扫描完/未审阅」两种情况共用）
    pub selection_ready: bool,
    /// 选择集修订号（进 OptionsKey，改选择要重算计划）
    pub selection_rev: u64,
    /// 放大查看的候选下标（None = 网格模式）
    pub review_loupe: Option<usize>,
    /// 放大图的 JPEG 路径（后台生成，None = 生成中）
    pub loupe_thumb: Option<PathBuf>,
    /// 审阅缩略图进度
    pub preview_done: u32,
    pub preview_total: u32,
    /// 取消审阅缩略图生成（关弹窗/换源/换过滤条件）。
    /// **每个生成池持有自己的 Arc**：换过滤条件时要能「取消旧池」而**不**误伤刚起的新池
    pub preview_cancel: Arc<AtomicBool>,
    /// 缩略图生成池的世代号（旧池的进度回调不许覆盖新池的计数）
    pub preview_pool: u64,

    // ── 目标与选项 ──
    /// 目标根目录（选择或手输）
    pub dest_root: String,
    pub subfolder: ImportSubfolder,
    pub rename_template: String,
    pub mode: ImportMode,
    pub filter: ImportFilter,
    pub policy: ConflictPolicy,
    /// 复制后校验落地大小
    pub verify: bool,
    /// 完成后弹出源盘（仅 Linux 生效）
    pub eject_after: bool,

    // ── 计划 ──
    pub plan: Option<ImportPlan>,
    pub planning: bool,
    /// 生成当前 plan 时的取值快照
    pub plan_key: Option<OptionsKey>,
    /// 去抖世代号：过期的定时回调不重算
    pub plan_debounce: u64,
    /// 点「导入」时计划已过期 → 先重算再自动续跑
    pub pending_execute: bool,
    /// 源卷标识（账本去重的根；None = 未识别，账本关闭）
    pub volume_id: Option<String>,
    /// 断点日志里已完成的条数（「上次导入剩 N 张」）
    pub resumed: usize,
    /// 账本/日志的非致命提示（打不开、卷未识别等）
    pub plan_note: Option<String>,

    // ── 执行 ──
    pub running: bool,
    /// 执行进度：(已完成, 总数, 当前文件)
    pub progress: Option<(u32, u32, String)>,
    pub result: Option<ImportOutcome>,
    /// 瞬态错误提示（4 秒后由状态栏/弹窗自行忽略）
    pub error: Option<(String, Instant)>,
    /// 取消标志（与引擎 `should_cancel` 共享；执行开始时清零）
    pub cancel: Arc<AtomicBool>,
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
            selected: HashSet::new(),
            selection_ready: false,
            selection_rev: 0,
            review_loupe: None,
            loupe_thumb: None,
            preview_done: 0,
            preview_total: 0,
            preview_cancel: Arc::new(AtomicBool::new(false)),
            preview_pool: 0,
            dest_root: String::new(),
            subfolder: ImportSubfolder::default(),
            rename_template: String::new(),
            mode: ImportMode::Copy,
            filter: ImportFilter::default(),
            policy: ConflictPolicy::default(),
            verify: true,
            eject_after: false,
            plan: None,
            planning: false,
            plan_key: None,
            plan_debounce: 0,
            pending_execute: false,
            volume_id: None,
            resumed: 0,
            plan_note: None,
            running: false,
            progress: None,
            result: None,
            error: None,
            cancel: Arc::new(AtomicBool::new(false)),
            generation: 0,
        }
    }
}

impl ImportState {
    /// 打开弹窗时复位；目标根目录预填当前浏览目录（原地导入是最常见用法）。
    pub fn reset(&mut self, current_dir: Option<&Path>) {
        self.generation = self.generation.wrapping_add(1);
        self.cancel.store(false, Ordering::Relaxed);
        self.preview_cancel.store(true, Ordering::Relaxed);
        self.tab = ImportTab::Import;
        self.source = None;
        self.candidates.clear();
        self.scanning = false;
        self.selected.clear();
        self.selection_ready = false;
        self.selection_rev = self.selection_rev.wrapping_add(1);
        self.review_loupe = None;
        self.loupe_thumb = None;
        self.preview_done = 0;
        self.preview_total = 0;
        self.dest_root = current_dir
            .map(|d| d.to_string_lossy().to_string())
            .unwrap_or_default();
        self.subfolder = ImportSubfolder::default();
        self.rename_template.clear();
        self.mode = ImportMode::Copy;
        self.filter = ImportFilter::default();
        self.policy = ConflictPolicy::default();
        self.verify = true;
        self.eject_after = false;
        self.plan = None;
        self.planning = false;
        self.plan_key = None;
        self.plan_debounce = self.plan_debounce.wrapping_add(1);
        self.pending_execute = false;
        self.volume_id = None;
        self.resumed = 0;
        self.plan_note = None;
        self.running = false;
        self.progress = None;
        self.result = None;
        self.error = None;
        self.drives = photo_engine::import::detect_removable_drives();
    }

    /// 计划里待导入的文件总数。
    pub fn plan_count(&self) -> usize {
        self.plan.as_ref().map(ImportPlan::file_count).unwrap_or(0)
    }

    /// 计划分诊统计（未生成计划时全 0）
    pub fn stats(&self) -> photo_engine::import::ImportPlanStats {
        self.plan.as_ref().map(ImportPlan::stats).unwrap_or_default()
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

    /// 当前取值快照（与 plan_key 比对判断计划是否过期）
    pub fn options_key(&self) -> OptionsKey {
        OptionsKey {
            dest: self.dest_root.trim().to_string(),
            subfolder: self.subfolder,
            template: self.rename_template.trim().to_string(),
            filter: self.filter,
            policy: self.policy,
            verify: self.verify,
            selection_rev: self.selection_rev,
        }
    }

    /// 某个候选是否被勾选（未动过选择集时 = 全选）
    pub fn is_selected(&self, index: usize) -> bool {
        !self.selection_ready || self.selected.contains(&index)
    }

    /// 可见候选（按类别过滤后的下标 + 引用）——网格与勾选都基于它
    pub fn visible_candidates(&self) -> Vec<(usize, ImportCandidate)> {
        self.candidates
            .iter()
            .enumerate()
            .filter(|(_, c)| self.filter.accepts(c.kind))
            .map(|(i, c)| (i, c.clone()))
            .collect()
    }

    /// 本次计划要用的候选（勾选 ∩ 类别过滤）
    pub fn selected_candidates(&self) -> Vec<ImportCandidate> {
        self.candidates
            .iter()
            .enumerate()
            .filter(|(i, c)| self.is_selected(*i) && self.filter.accepts(c.kind))
            .map(|(_, c)| c.clone())
            .collect()
    }

    /// 类别过滤后可见的候选数（渲染期每帧调用，别 clone 整份候选）
    pub fn visible_count(&self) -> usize {
        self.candidates
            .iter()
            .filter(|c| self.filter.accepts(c.kind))
            .count()
    }

    /// 勾选数（未动过选择集时 = 可见候选数）
    pub fn selected_count(&self) -> usize {
        if !self.selection_ready {
            return self.visible_count();
        }
        self.candidates
            .iter()
            .enumerate()
            .filter(|(i, c)| self.selected.contains(i) && self.filter.accepts(c.kind))
            .count()
    }

    pub fn set_error(&mut self, message: impl Into<String>) {
        self.error = Some((message.into(), Instant::now()));
    }

    /// 是否已请求取消（按钮文案「正在取消…」用）
    pub fn is_cancelling(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// 切换一个候选的勾选态
    pub fn toggle_selection(&mut self, index: usize) {
        self.ensure_selection();
        if !self.selected.remove(&index) {
            self.selected.insert(index);
        }
        self.selection_rev = self.selection_rev.wrapping_add(1);
    }

    /// 全选 / 全不选 / 反选（只作用于类别过滤后可见的候选）
    pub fn select_all(&mut self) {
        self.ensure_selection();
        for (i, _) in self.visible_candidates() {
            self.selected.insert(i);
        }
        self.selection_rev = self.selection_rev.wrapping_add(1);
    }

    pub fn deselect_all(&mut self) {
        self.ensure_selection();
        self.selected.clear();
        self.selection_rev = self.selection_rev.wrapping_add(1);
    }

    pub fn invert_selection(&mut self) {
        self.ensure_selection();
        for (i, _) in self.visible_candidates() {
            if !self.selected.remove(&i) {
                self.selected.insert(i);
            }
        }
        self.selection_rev = self.selection_rev.wrapping_add(1);
    }

    /// 把「全选」这个隐含态物化成显式集合（用户一动选择集就必须物化）
    fn ensure_selection(&mut self) {
        if self.selection_ready {
            return;
        }
        self.selected = (0..self.candidates.len()).collect();
        self.selection_ready = true;
    }
}

/// 审阅缩略图缓存目录：`<配置目录>/import_preview/<源路径哈希>/`。
///
/// 刻意**不写源盘**——现在浏览卡会在卡上生成 `.pt/thumbs`，导入审阅再往卡上写
/// 千张缩略图既慢又脏；缓存放配置目录，源路径不同互不干扰。
pub fn preview_cache_dir(source_root: &Path) -> PathBuf {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    source_root.hash(&mut hasher);
    let key = format!("{:016x}", hasher.finish());
    config_base().join("import_preview").join(key)
}

/// 导入账本数据库路径（配置目录，**不写卡**：卡可能写保护）
pub fn history_db_path() -> PathBuf {
    config_base().join("import_history.db")
}

/// 账本的源根：优先**挂载根**（换子目录仍能命中同一行账本），拿不到才退用户选的目录。
pub fn ledger_root(source: &Path) -> PathBuf {
    photo_engine::import_history::volume_root_for_path(source)
        .unwrap_or_else(|| source.to_path_buf())
}

/// 配置目录（拿不到时退临时目录，功能降级但不崩）
fn config_base() -> PathBuf {
    photo_config::config_dir().unwrap_or_else(|_| std::env::temp_dir().join("pt-config"))
}

/// 候选 → 引擎侧 SourceFile（缩略图/放大用）
fn candidate_source_file(cand: &ImportCandidate) -> SourceFile {
    let format = cand
        .path
        .extension()
        .and_then(|e| e.to_str())
        .and_then(photo_domain::ImageFormat::from_extension)
        .unwrap_or(photo_domain::ImageFormat::Jpeg);
    SourceFile {
        path: cand.path.clone(),
        format,
        file_size: Some(cand.size),
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

/// 界面取值 → 可落盘的导入配方
pub fn recipe_of(state: &ImportState) -> ImportRecipe {
    ImportRecipe {
        dest_root: Some(state.dest_root.trim().to_string()).filter(|d| !d.is_empty()),
        subfolder: match state.subfolder {
            ImportSubfolder::None => "none",
            ImportSubfolder::DateDash => "dateDash",
            ImportSubfolder::DateSlash => "dateSlash",
            ImportSubfolder::DateCompact => "dateCompact",
        }
        .to_string(),
        rename_template: state.rename_template.clone(),
        mode: match state.mode {
            ImportMode::Copy => "copy",
            ImportMode::Move => "move",
        }
        .to_string(),
        photos: state.filter.photos,
        raw: state.filter.raw,
        videos: state.filter.videos,
        skip_jpeg_with_raw: state.filter.skip_jpeg_with_raw,
        policy: match state.policy {
            ConflictPolicy::Skip => "skip",
            ConflictPolicy::Rename => "rename",
            ConflictPolicy::Overwrite => "overwrite",
        }
        .to_string(),
        verify: state.verify,
        eject_after: state.eject_after,
    }
    .clamped()
}

/// 配方 → 界面取值（未知值已在 config 侧归一为默认）
pub fn apply_recipe(state: &mut ImportState, recipe: &ImportRecipe) {
    if let Some(dest) = recipe.dest_root.as_deref().filter(|d| !d.trim().is_empty()) {
        state.dest_root = dest.to_string();
    }
    state.subfolder = match recipe.subfolder.as_str() {
        "none" => ImportSubfolder::None,
        "dateSlash" => ImportSubfolder::DateSlash,
        "dateCompact" => ImportSubfolder::DateCompact,
        _ => ImportSubfolder::DateDash,
    };
    state.rename_template = recipe.rename_template.clone();
    state.mode = match recipe.mode.as_str() {
        "move" => ImportMode::Move,
        _ => ImportMode::Copy,
    };
    state.filter = ImportFilter {
        photos: recipe.photos,
        raw: recipe.raw,
        videos: recipe.videos,
        skip_jpeg_with_raw: recipe.skip_jpeg_with_raw,
    };
    state.policy = match recipe.policy.as_str() {
        "rename" => ConflictPolicy::Rename,
        "overwrite" => ConflictPolicy::Overwrite,
        _ => ConflictPolicy::Skip,
    };
    state.verify = recipe.verify;
    state.eject_after = recipe.eject_after;
}

/// 递归扫描选中源（engine 只 stat 取文件修改时间，不读 EXIF）。
///
/// 扫完立即：① 初始化全选 ② 起审阅缩略图 ③ 排一次实时计划。
pub fn scan_source(entity: Entity<AppState>, cx: &mut App) {
    let prepared = entity.update(cx, |state, cx| {
        let source = state.import.source.clone()?;
        state.import.scanning = true;
        state.import.candidates.clear();
        state.import.selected.clear();
        state.import.selection_ready = false;
        state.import.plan = None;
        state.import.result = None;
        state.import.error = None;
        state.import.review_loupe = None;
        state.import.loupe_thumb = None;
        state.import.preview_done = 0;
        state.import.preview_total = 0;
        // 旧源的缩略图任务立即停（换源时别让它继续读卡）
        state.import.preview_cancel.store(true, Ordering::Relaxed);
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
        let ready = async_cx.update(|cx| {
            entity.update(cx, |state, cx| {
                if state.import.generation != generation {
                    return false;
                }
                state.import.scanning = false;
                match result {
                    Ok(candidates) => {
                        let count = candidates.len();
                        state.import.candidates = candidates;
                        state.import.selection_ready = false;
                        state.set_status_message(format!("导入源扫描完成：{count} 张"));
                        cx.notify();
                        true
                    }
                    Err(err) => {
                        state.import.set_error(format!("扫描导入源失败：{err}"));
                        cx.notify();
                        false
                    }
                }
            })
        });
        if ready {
            async_cx.update(|cx| {
                start_preview_thumbs(entity.clone(), cx);
                schedule_plan(entity, cx);
            });
        }
    })
    .detach();
}

/// 安排一次计划重算（300ms 去抖）。选项/选择集一变就调它。
pub fn schedule_plan(entity: Entity<AppState>, cx: &mut App) {
    let generation = entity.update(cx, |state, _cx| {
        state.import.plan_debounce = state.import.plan_debounce.wrapping_add(1);
        state.import.plan_debounce
    });
    cx.spawn(async move |async_cx| {
        async_cx
            .background_executor()
            .timer(Duration::from_millis(PLAN_DEBOUNCE_MS))
            .await;
        let go = async_cx.update(|cx| {
            entity.update(cx, |state, _cx| {
                state.import.plan_debounce == generation && state.import.can_plan()
            })
        });
        if go {
            async_cx.update(|cx| generate_plan(entity, cx));
        }
    })
    .detach();
}

/// 干跑计划：过滤 + 三层去重 + 冲突策略（不碰文件）。
///
/// 账本/断点日志都在后台任务里打开：打开失败只降级（记 `plan_note`），不阻断计划。
pub fn generate_plan(entity: Entity<AppState>, cx: &mut App) {
    let prepared = entity.update(cx, |state, cx| {
        sync_inputs(state, cx);
        if !state.import.can_plan() {
            state.import.pending_execute = false;
            return None;
        }
        let dest = state.import.dest_root.trim().to_string();
        let template = state.import.rename_template.trim().to_string();
        let options = photo_engine::import::ImportOptions {
            subfolder: state.import.subfolder,
            rename_template: (!template.is_empty()).then_some(template),
            filter: state.import.filter,
            policy: state.import.policy,
            verify: state.import.verify,
        };
        let key = OptionsKey {
            dest: dest.clone(),
            subfolder: options.subfolder,
            template: options.rename_template.clone().unwrap_or_default(),
            filter: options.filter,
            policy: options.policy,
            verify: options.verify,
            selection_rev: state.import.selection_rev,
        };
        let candidates = state.import.selected_candidates();
        let source_root = state.import.source.clone();
        state.import.planning = true;
        state.import.error = None;
        cx.notify();
        Some((
            state.import.generation,
            candidates,
            PathBuf::from(dest),
            options,
            key,
            source_root,
        ))
    });
    let Some((generation, candidates, dest, options, key, source_root)) = prepared else {
        return;
    };

    let history_path = history_db_path();
    cx.spawn(async move |async_cx| {
        let outcome = async_cx
            .background_executor()
            .spawn(async move {
                // 账本：源卷可识别才启用（识别不出来宁可关掉，也不能凭猜测的卷号跳文件）
                let volume_id = source_root.as_deref().and_then(volume_id_for_path);
                let history = ImportHistory::open(&history_path).ok();
                let journal = ImportJournal::open(&dest).ok();
                let ledger_base = source_root.as_deref().map(ledger_root);
                let ledger = match (&history, ledger_base.as_deref(), volume_id.as_deref()) {
                    (Some(history), Some(root), Some(volume_id)) => Some(SourceLedger {
                        source_root: root,
                        volume_id,
                        history,
                    }),
                    _ => None,
                };
                let extras = PlanExtras {
                    ledger,
                    resume: journal.as_ref(),
                };
                let plan = engine_plan_import(&candidates, &dest, &options, &extras);
                let resumed = journal.as_ref().map(ImportJournal::completed_count).unwrap_or(0);
                let mut note = None;
                if volume_id.is_none() {
                    note = Some("未识别源卷：账本去重关闭（仍按目标存在性与源内冲突去重）".to_string());
                } else if history.is_none() {
                    note = Some("导入账本打开失败：本次退化为目标存在性去重".to_string());
                }
                (plan, volume_id, resumed, note)
            })
            .await;

        let (plan, volume_id, resumed, note) = outcome;
        async_cx.update(|cx| {
            entity.update(cx, |state, cx| {
                if state.import.generation != generation {
                    return;
                }
                state.import.planning = false;
                state.import.plan = Some(plan);
                state.import.plan_key = Some(key);
                state.import.volume_id = volume_id;
                state.import.resumed = resumed;
                state.import.plan_note = note;
                cx.notify();
            });
        });
        // 计划过期时点的「导入」：重算完立刻续跑。
        // 「stale」这条是去抖窗口撞上飞行中计划的补救——计划在跑时又改了选项，
        // 那次 schedule 会因为 can_plan()==false 被丢掉，这里补排一次，否则界面上的
        // 数字会一直停在旧值（直到用户再碰一次选项）。
        let (pending, stale) = async_cx.update(|cx| {
            entity.update(cx, |state, _cx| {
                let stale = state.import.plan_key.as_ref() != Some(&state.import.options_key());
                (std::mem::take(&mut state.import.pending_execute), stale)
            })
        });
        if pending {
            async_cx.update(|cx| execute(entity.clone(), cx));
        } else if stale {
            async_cx.update(|cx| schedule_plan(entity.clone(), cx));
        }
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

/// 执行导入时需要的、从主线程带进后台的东西
struct RunPlan {
    generation: u64,
    plan: ImportPlan,
    dest: String,
    mode: ImportMode,
    verify: bool,
    eject_after: bool,
    cancel: Arc<AtomicBool>,
    source_root: Option<PathBuf>,
    volume_id: Option<String>,
    /// (源, 大小, mtime 纳秒)：Move 之后源不在了，账本只能靠扫描时的快照记账
    ledger_meta: Vec<(PathBuf, u64, i64)>,
    history_path: PathBuf,
}

/// 执行导入：后台逐文件复制/移动（引擎侧每文件检查一次取消标志），主线程 120ms 轮询进度。
///
/// 完成后**不关弹窗**：成功/跳过/失败逐条留在结果面板；无失败且确实搬了东西时顺手
/// 重扫结果目录；有失败可「重试失败项」。成功收尾会写账本、丢弃断点日志、可选弹出源盘。
pub fn execute(entity: Entity<AppState>, cx: &mut App) {
    let prepared: Option<Result<RunPlan, ()>> = entity.update(cx, |state, cx| {
        sync_inputs(state, cx);
        let plan = state.import.plan.clone()?;
        if state.import.running {
            return None;
        }
        // 计划过期（用户刚改了选项/选择集）：标记「算完就导」并立刻重算。
        // 但先确认确实还能算计划——否则 pending 会一直挂着，等用户之后补好目标目录时
        // 突然自己开始导入（不该发生的事比多点一次按钮更糟）。
        if state.import.plan_key.as_ref() != Some(&state.import.options_key()) {
            if !state.import.can_plan() {
                state.import.set_error("请先选好来源与目标目录");
                return None;
            }
            state.import.pending_execute = true;
            return Some(Err(()));
        }
        if state.import.plan_count() == 0 {
            state.import.set_error("没有勾选任何文件");
            return None;
        }
        let dest = state.import.dest_root.trim().to_string();
        if dest.is_empty() {
            state.import.set_error("目标目录未指定");
            return None;
        }
        state.import.running = true;
        state.import.cancel.store(false, Ordering::Relaxed);
        state.import.result = None;
        state.import.error = None;
        // 目标目录 + 配方记忆（与导出侧同一套做法，落盘失败不影响导入）
        state.app_config.import_dir = Some(dest.clone());
        state.app_config.import_recipe = recipe_of(&state.import);
        state.save_config();
        state.import.progress = Some((0, state.import.plan_count() as u32, String::new()));
        let ledger_meta = state
            .import
            .candidates
            .iter()
            .map(|c| (c.path.clone(), c.size, c.mtime_ns))
            .collect();
        cx.notify();
        Some(Ok(RunPlan {
            generation: state.import.generation,
            plan,
            dest,
            mode: state.import.mode,
            verify: state.import.verify,
            eject_after: state.import.eject_after,
            cancel: state.import.cancel.clone(),
            // 这里必须是**账本基准（挂载根）**：既要与计划期的 rel_path 基准一致，
            // 也是「弹出源盘」真正能卸载的那个挂载点（用户选的可能是 DCIM 子目录）
            source_root: state.import.source.as_deref().map(ledger_root),
            volume_id: state.import.volume_id.clone(),
            ledger_meta,
            history_path: history_db_path(),
        }))
    });

    let run = match prepared {
        None => return,
        Some(Err(())) => {
            generate_plan(entity, cx);
            return;
        }
        Some(Ok(run)) => run,
    };

    let dest = PathBuf::from(&run.dest);
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

    let dir_to_open = imported_dir(&run.plan, &run.dest);
    let RunPlan {
        generation,
        plan,
        mode,
        verify,
        eject_after,
        cancel,
        source_root,
        volume_id,
        ledger_meta,
        history_path,
        ..
    } = run;
    let skipped = plan.skipped.len() as u32;
    let total_files = plan.file_count();
    let progress_slot: Arc<Mutex<Option<(u32, u32, String)>>> = Arc::new(Mutex::new(None));
    let done = Arc::new(AtomicBool::new(false));
    let report_slot: Arc<Mutex<Option<(ImportReport, Option<String>)>>> = Arc::new(Mutex::new(None));

    let entity_poll = entity.clone();
    cx.spawn(async move |async_cx| {
        let progress_cb = progress_slot.clone();
        let done_cb = done.clone();
        let report_cb = report_slot.clone();
        let plan_bg = plan.clone();
        let dest_bg = dest.clone();
        let cancel_cb = cancel.clone();
        let exec = async_cx.background_executor().spawn(async move {
            let journal = ImportJournal::open(&dest_bg).ok();
            let report = engine_execute_import(
                &plan_bg,
                &dest_bg,
                mode,
                move || cancel_cb.load(Ordering::Relaxed),
                verify,
                journal,
                Some(Box::new(move |p| {
                    *progress_cb.lock().unwrap() =
                        Some((p.done, p.total, p.current.to_string_lossy().to_string()));
                })),
            );

            // ── 账本记账：只记真正落地的文件（Move 之后源已不存在，用扫描快照）──
            if let (Some(volume_id), Some(root)) = (volume_id.as_deref(), source_root.as_deref())
                && let Ok(history) = ImportHistory::open(&history_path)
            {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                let dest_of: HashMap<&Path, &Path> = report
                    .ok
                    .iter()
                    .map(|(s, d)| (s.as_path(), d.as_path()))
                    .collect();
                let records: Vec<ImportRecord> = ledger_meta
                    .iter()
                    .filter_map(|(path, size, mtime_ns)| {
                        let dest_path = dest_of.get(path.as_path())?;
                        let rel = path.strip_prefix(root).ok()?;
                        Some(ImportRecord {
                            volume_id: volume_id.to_string(),
                            rel_path: rel.to_string_lossy().to_string(),
                            size: *size,
                            mtime_ns: *mtime_ns,
                            dest_path: dest_path.to_string_lossy().to_string(),
                            imported_at: now,
                        })
                    })
                    .collect();
                let _ = history.record(&records);
            }

            // ── 收尾：全成功且没被取消才丢弃断点日志；否则留着续传 ──
            let all_ok = !report.cancelled && report.failed.is_empty();
            if all_ok {
                let _ = ImportJournal::discard(&dest_bg);
            }

            // ── 完成后弹出源盘（仅 Linux；失败只提示，不影响导入结果）──
            let eject_note = if eject_after && all_ok && report.imported() > 0 {
                match source_root.as_deref() {
                    Some(root) => Some(
                        eject_source(root)
                            .err()
                            .map(|err| format!("弹出源盘失败：{err}"))
                            .unwrap_or_else(|| "已弹出源盘".to_string()),
                    ),
                    None => None,
                }
            } else {
                None
            };

            *report_cb.lock().unwrap() = Some((report, eject_note));
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

        let (report, eject_note) = report_slot.lock().unwrap().take().unwrap_or_default();
        let imported = report.imported() as u32;
        let unprocessed = report.unprocessed(total_files) as u32;
        let failed = report.failed.clone();
        let warnings = report.warnings.clone();
        let cancelled = report.cancelled;
        let no_failure = failed.is_empty();

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
                    failed: failed.clone(),
                    warnings: warnings.clone(),
                    cancelled,
                    unprocessed,
                });
                if cancelled {
                    state.set_status_message(format!(
                        "导入已取消：已搬运 {imported} 张，未处理 {unprocessed} 张（下次导入会从断点续传）"
                    ));
                } else if no_failure {
                    let skipped_note = if skipped > 0 {
                        format!("，跳过 {skipped}")
                    } else {
                        String::new()
                    };
                    let warn_note = if warnings.is_empty() {
                        String::new()
                    } else {
                        format!("，{} 条警告", warnings.len())
                    };
                    let eject_note = eject_note
                        .as_deref()
                        .map(|n| format!("；{n}"))
                        .unwrap_or_default();
                    state.set_status_message(format!(
                        "导入完成：成功 {imported} 张{skipped_note}{warn_note}{eject_note}"
                    ));
                } else {
                    state.import.set_error(format!("{} 个文件导入失败", failed.len()));
                }
                // 结果常驻：不自动关窗，用户看完明细点「完成」
                cx.notify();
            });
        });

        if no_failure
            && imported > 0
            && let Some(dir) = dir_to_open
        {
            async_cx.update(|cx| start_scan(entity.clone(), dir, true, cx));
        }
        // 搬完东西就重算一次：分诊条要从「待导入」翻成「已导入」，否则界面停在导入前的数字上
        if imported > 0 {
            async_cx.update(|cx| {
                let _ = entity.update(cx, |state, _cx| {
                    if !cancelled && no_failure {
                        // 成功收尾时断点日志已被丢弃，「上次导入已完成 N 项」要跟着清零
                        state.import.resumed = 0;
                    }
                });
                schedule_plan(entity.clone(), cx);
            });
        }
    })
    .detach();
}

/// 重试失败项：按失败源重新干跑计划（已落地的会被账本/目标去重跳过）→ 直接执行。
pub fn retry_failed(entity: Entity<AppState>, cx: &mut App) {
    let prepared = entity.update(cx, |state, cx| {
        sync_inputs(state, cx);
        let failed_paths: Vec<PathBuf> = state
            .import
            .result
            .as_ref()
            .map(|r| r.failed.iter().map(|f| f.source.clone()).collect())
            .unwrap_or_default();
        if failed_paths.is_empty() {
            state.set_status_message("没有需要重试的失败项");
            return None;
        }
        let dest = state.import.dest_root.trim().to_string();
        if dest.is_empty() {
            state.import.set_error("目标目录未指定");
            return None;
        }
        let candidates: Vec<ImportCandidate> = state
            .import
            .candidates
            .iter()
            .filter(|c| failed_paths.contains(&c.path))
            .cloned()
            .collect();
        if candidates.is_empty() {
            state.import
                .set_error("失败的文件已不在扫描结果里，请重新扫描来源");
            return None;
        }
        let template = state.import.rename_template.trim().to_string();
        let options = photo_engine::import::ImportOptions {
            subfolder: state.import.subfolder,
            rename_template: (!template.is_empty()).then_some(template),
            filter: state.import.filter,
            policy: state.import.policy,
            verify: state.import.verify,
        };
        let key = OptionsKey {
            dest: dest.clone(),
            subfolder: options.subfolder,
            template: options.rename_template.clone().unwrap_or_default(),
            filter: options.filter,
            policy: options.policy,
            verify: options.verify,
            selection_rev: state.import.selection_rev,
        };
        let source_root = state.import.source.clone();
        // 刻意**不**清 result：万一重试计划为空（全被去重跳过），失败清单还得留在面板上，
        // 否则用户看到的是「结果卡凭空消失 + 状态栏一句话」。真正执行时 execute() 会清。
        state.import.error = None;
        cx.notify();
        Some((state.import.generation, candidates, PathBuf::from(dest), options, key, source_root))
    });
    let Some((generation, candidates, dest, options, key, source_root)) = prepared else {
        return;
    };

    let history_path = history_db_path();
    cx.spawn(async move |async_cx| {
        let (plan, volume_id, resumed) = async_cx
            .background_executor()
            .spawn(async move {
                let volume_id = source_root.as_deref().and_then(volume_id_for_path);
                let history = ImportHistory::open(&history_path).ok();
                let journal = ImportJournal::open(&dest).ok();
                let ledger_base = source_root.as_deref().map(ledger_root);
                let ledger = match (&history, ledger_base.as_deref(), volume_id.as_deref()) {
                    (Some(history), Some(root), Some(volume_id)) => Some(SourceLedger {
                        source_root: root,
                        volume_id,
                        history,
                    }),
                    _ => None,
                };
                let extras = PlanExtras {
                    ledger,
                    resume: journal.as_ref(),
                };
                let plan = engine_plan_import(&candidates, &dest, &options, &extras);
                let resumed = journal.as_ref().map(ImportJournal::completed_count).unwrap_or(0);
                (plan, volume_id, resumed)
            })
            .await;
        let count = plan.file_count();
        async_cx.update(|cx| {
            entity.update(cx, |state, cx| {
                if state.import.generation != generation {
                    return;
                }
                if count == 0 {
                    state.set_status_message("失败项已全部落地（被去重跳过），无需重试");
                    return;
                }
                state.import.plan = Some(plan);
                state.import.plan_key = Some(key);
                state.import.volume_id = volume_id;
                state.import.resumed = resumed;
                cx.notify();
            });
        });
        // 与上面分成两次 update：避免在同一次 entity.update 里再进 execute
        async_cx.update(|cx| execute(entity.clone(), cx));
    })
    .detach();
}

/// 视图按钮专用：改「目录与命名 / 过滤 / 策略 / 勾选」后立即安排重算。
///
/// 选择集与选项都要进 OptionsKey，所以这里统一 +1 选择集修订号；顺带把审阅缩略图
/// 也重排一次（换过滤条件后新类别要生成缩略图；已缓存的会直接命中，代价很低）。
pub fn update_import(
    entity: Entity<AppState>,
    cx: &mut Context<AppState>,
    apply: impl FnOnce(&mut ImportState) + 'static,
) {
    let entity_for_defer = entity.clone();
    defer_entity_action(cx, entity_for_defer, move |entity, cx| {
        entity.update(cx, |state, _cx| {
            apply(&mut state.import);
            state.import.selection_rev = state.import.selection_rev.wrapping_add(1);
        });
        start_preview_thumbs(entity.clone(), cx);
        schedule_plan(entity, cx);
    });
}

/// 起审阅缩略图生成（4 worker + 共享游标 + 可取消），缓存写配置目录。
pub fn start_preview_thumbs(entity: Entity<AppState>, cx: &mut App) {
    let prepared = entity.update(cx, |state, _cx| {
        let root = state.import.source.clone()?;
        if state.import.candidates.is_empty() {
            return None;
        }
        let jobs: Vec<SourceFile> = state
            .import
            .visible_candidates()
            .iter()
            .map(|(_, c)| candidate_source_file(c))
            .collect();
        if jobs.is_empty() {
            return None;
        }
        let manager = state.import_preview_manager(&root);
        // 换池：先把旧池的取消标志置真，再挂上一个**新** Arc 给新池——
        // 共用同一个 Arc 的话，「取消旧池」会连新池一起取消（旧实现就是这个毛病）
        let old = std::mem::replace(
            &mut state.import.preview_cancel,
            Arc::new(AtomicBool::new(false)),
        );
        old.store(true, Ordering::Relaxed);
        state.import.preview_pool = state.import.preview_pool.wrapping_add(1);
        let pool = state.import.preview_pool;
        let cancel = state.import.preview_cancel.clone();
        state.import.preview_done = 0;
        state.import.preview_total = jobs.len() as u32;
        Some((manager, cancel, Arc::new(jobs), state.import.generation, pool))
    });
    let Some((manager, cancel, jobs, generation, pool)) = prepared else {
        return;
    };
    let total = jobs.len();
    let done = Arc::new(AtomicUsize::new(0));
    let cursor = Arc::new(AtomicUsize::new(0));
    // 供 tick 判断「本池还在不在」：用自己那个 Arc，别用 state 里的（可能已被换掉）
    let pool_cancel = cancel.clone();

    cx.spawn(async move |async_cx| {
        for _ in 0..PREVIEW_WORKERS {
            let jobs = jobs.clone();
            let done = done.clone();
            let cursor = cursor.clone();
            let cancel = cancel.clone();
            let manager = manager.clone();
            async_cx
                .background_executor()
                .spawn(async move {
                    loop {
                        if cancel.load(Ordering::Relaxed) {
                            break;
                        }
                        let i = cursor.fetch_add(1, Ordering::Relaxed);
                        let Some(source) = jobs.get(i) else { break };
                        let _ = manager.ensure_thumb(source, THUMB_SIZE_GRID, Some(cancel.as_ref()));
                        done.fetch_add(1, Ordering::Relaxed);
                    }
                })
                .detach();
        }

        loop {
            async_cx
                .background_executor()
                .timer(Duration::from_millis(150))
                .await;
            let finished = done.load(Ordering::Relaxed).min(total);
            let keep = async_cx.update(|cx| {
                let mut keep = false;
                let _ = entity.update(cx, |state, cx| {
                    // 世代或池对不上 = 已经换源/换过滤了：不写进度，直接退场
                    if state.import.generation != generation || state.import.preview_pool != pool {
                        return;
                    }
                    // 弹窗已经关了：别再为了看不见的网格读卡（大卡上这能白读几十分钟）
                    if state.active_dialog != Some(ActiveDialog::Import) {
                        pool_cancel.store(true, Ordering::Relaxed);
                        return;
                    }
                    state.import.preview_done = finished as u32;
                    cx.notify();
                    keep = finished < total && !pool_cancel.load(Ordering::Relaxed);
                });
                keep
            });
            if !keep {
                break;
            }
        }
    })
    .detach();
}

/// 放大查看某张候选（用 2560 母版缩略图；生成中先显示占位）。
pub fn open_loupe(entity: Entity<AppState>, index: usize, cx: &mut App) {
    let prepared = entity.update(cx, |state, _cx| {
        let root = state.import.source.clone()?;
        let cand = state.import.candidates.get(index)?.clone();
        let manager = state.import_preview_manager(&root);
        state.import.review_loupe = Some(index);
        state.import.loupe_thumb = None;
        Some((manager, candidate_source_file(&cand), state.import.generation))
    });
    let Some((manager, source, generation)) = prepared else {
        return;
    };
    cx.spawn(async move |async_cx| {
        let path = async_cx
            .background_executor()
            .spawn(async move { manager.ensure_thumb(&source, MASTER_SIZE, None).ok() })
            .await;
        async_cx.update(|cx| {
            entity.update(cx, |state, cx| {
                // 必须连**下标**一起校验：快速从 A 切到 B 时，A 的解码任务可能后完成，
                // 只校验 generation 会把 B 的画面盖成 A 的图
                if state.import.generation != generation
                    || state.import.review_loupe != Some(index)
                {
                    return;
                }
                state.import.loupe_thumb = path;
                cx.notify();
            });
        });
    })
    .detach();
}

/// 关掉放大视图，回到网格（顺带取消这个候选的放大生成）
pub fn close_loupe(entity: Entity<AppState>, cx: &mut Context<AppState>) {
    // 只是退出放大视图，跟计划无关——别走 update_import（那会白排一次重算）
    defer_entity_action(cx, entity, |entity, cx| {
        entity.update(cx, |state, cx| {
            state.import.review_loupe = None;
            state.import.loupe_thumb = None;
            cx.notify();
        });
    });
}

/// 「忽略断点日志，全部重导」：删掉日志文件并重算计划。
///
/// 这是一个逃生门：续传默认生效，但用户可能就是想按新目标/新命名把这批**再导一遍**——
/// 没有这个入口，唯一办法是手工去删 `<目标>/.pt/import-journal.jsonl`。
pub fn discard_journal(entity: Entity<AppState>, cx: &mut Context<AppState>) {
    defer_entity_action(cx, entity.clone(), move |entity, cx| {
        let dest = entity.read(cx).import.dest_root.trim().to_string();
        let mut removed = false;
        if !dest.is_empty() {
            removed = ImportJournal::discard(Path::new(&dest)).is_ok();
        }
        entity.update(cx, |state, _cx| {
            state.import.resumed = 0;
            if removed {
                state.set_status_message("已忽略断点日志：本次将重新导入全部勾选项");
            }
        });
        schedule_plan(entity, cx);
    });
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
                state.import.review_loupe = None;
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
                state.import.result = None;
                if let Some(input) = state.import_dest_input.clone() {
                    input.update(cx, |input_state, cx| {
                        input_state.set_value(text.clone(), window, cx);
                    });
                }
                cx.notify();
            });
            schedule_plan(entity, cx);
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
            state.import.selected.clear();
            state.import.selection_ready = false;
            state.import.plan = None;
            state.import.result = None;
            state.import.review_loupe = None;
        });
        scan_source(entity, cx);
    });
}

/// 视图按钮专用：重扫当前源。
pub fn rescan(entity: Entity<AppState>, cx: &mut Context<AppState>) {
    defer_entity_action(cx, entity, scan_source);
}

/// 外部入口（命令行 `--import <挂载点>`）：打开导入弹窗、预选源并立即扫描。
///
/// 手册 §10.3 承诺过的入口，GPUI 版一直没接——现在补上，插卡后可以直接
/// `ftpt --import /run/media/me/CANON` 起手。
pub fn open_with_source(
    entity: Entity<AppState>,
    source: PathBuf,
    window: &mut Window,
    cx: &mut App,
) {
    entity.update(cx, |state, cx| {
        state.open_import_dialog(window, cx);
        state.import.source = Some(source.clone());
    });
    scan_source(entity, cx);
}

/// 视图按钮专用：「仅添加」——不搬运，直接打开目录浏览。
pub fn open_source_in_grid(entity: Entity<AppState>, cx: &mut Context<AppState>) {
    defer_entity_action(cx, entity.clone(), move |entity, cx| {
        let Some(source) = entity.read(cx).import.source.clone() else {
            return;
        };
        entity.update(cx, |state, _cx| {
            state.active_dialog = None;
        });
        start_scan(entity, source, true, cx);
    });
}

/// 判断某候选的缩略图是否已就绪（网格渲染用；不触发解码）
pub fn thumb_path(manager: &ImageManager, cand: &ImportCandidate) -> Option<PathBuf> {
    manager.get_cached_thumb_path(&candidate_source_file(cand), THUMB_SIZE_GRID)
}

