//! ftpt 的 Tauri v2 后端（GPUI → Tauri 迁移 Phase 1 底座）。
//!
//! 编排语义照抄 GPUI 版 `state/scan.rs` / `state/metadata.rs`：
//! - 扫描闭包只查缓存（快返）；EXIF 增量提取 + 缩略图预生成放后台任务
//! - 评分/旗标/色标先写 folder_db xmp_meta 真相表，再更新内存，失败由前端重拉回滚
//! - 图像经 `ptimg://` 自定义协议流式 serve，不走 IPC base64

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;

use photo_config::AppConfig;
use photo_domain::{
    AdjustParams, CaptureMeta, ColorLabel, FilterCriteria, Flag, ImageFormat, Rating,
    Recognition, RecognitionFailureStage, RecognitionStatus, SourceFile,
};
use photo_engine::folder_db::{FileEntry, FolderDb};
use photo_engine::global_db::{GlobalDb, SpeciesRow};
use photo_engine::template::NameTemplateContext;
use photo_engine::thumbnail::ThumbnailCache;
use photo_engine::{exif, scanner};

// ============================================================================
// 事件负载（经 app.emit 推送；类型经 specta 导出到 bindings.ts）
// ============================================================================

/// 扫描进度阶段：scan = 目录扫描 + DB 三表同步；exif = EXIF 增量提取；thumb = 缩略图预生成
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum ScanStage {
    Scan,
    Exif,
    Thumb,
}

/// `scan:progress` 事件负载
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgress {
    pub stage: ScanStage,
    pub done: u32,
    pub total: u32,
}

/// `scan:done` 事件负载（directory 供前端同步当前目录状态，含启动自动恢复场景）
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ScanDone {
    pub total: u32,
    pub directory: String,
}

/// `capture:enriched` 事件负载：EXIF 回填完成的 capture 索引（前端据此重排）
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct CaptureEnriched {
    pub indices: Vec<u32>,
}

/// `thumb:ready` 事件负载：某张缩略图缓存已就绪（前端据此刷新对应 cell）
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ThumbReady {
    pub path: String,
}

/// `recognize:progress` 事件负载：批量识别逐张进度
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RecognizeProgress {
    pub done: u32,
    pub total: u32,
    pub current_path: String,
}

/// `recognize:done` 事件负载：批量识别汇总（failed = 系统级失败张数，单张失败不中止整体）
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RecognizeDone {
    pub total: u32,
    pub confirmed: u32,
    pub needs_review: u32,
    pub unrecognized: u32,
    pub failed: u32,
}

/// `batch:progress` 事件负载：批量文件操作逐文件进度
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BatchProgress {
    pub done: u32,
    pub total: u32,
    pub current_path: String,
}

/// `batch:done` 事件负载：批量文件操作汇总
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BatchDone {
    pub success: u32,
    pub failed: u32,
}

/// `export:progress` 事件负载：批量导出逐张进度（T1 批次，与 batch:progress 同形态，
/// 独立通道避免与批量操作进度互相覆盖）
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportProgress {
    pub done: u32,
    pub total: u32,
    pub current_path: String,
}

/// `export:done` 事件负载：批量导出汇总（T1 批次）
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportDone {
    pub success: u32,
    pub failed: u32,
}

// ============================================================================
// 批量操作 / 调整参数 command 出入参（经 specta 导出到 bindings.ts）
// ============================================================================

/// `batch_op_preview`/`batch_op_execute` 的选项：
/// targetDir = Move/Copy 必填、Delete 忽略；syncSiblings + formats = 画面粒度同步同名兄弟
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BatchOpOptions {
    /// 目标目录（Move/Copy 必填；Delete 忽略）
    pub target_dir: Option<String>,
    /// 是否按同名扩展操作集（同步兄弟文件，画面粒度 ADR 0006）
    pub sync_siblings: bool,
    /// 同步扩展时参与兄弟匹配的格式白名单（空 = 不过滤/不扩展）
    pub formats: Vec<ImageFormat>,
}

/// 干跑预览条目：path = 源主文件，targetPath = 目标位置（Delete 或无目标目录时为 null）
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BatchOpItem {
    pub path: String,
    pub target_path: Option<String>,
}

/// `batch_op_preview` 返回：干跑结果（不碰文件）
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BatchOpPreview {
    pub op: photo_domain::BatchOpType,
    pub count: u32,
    pub items: Vec<BatchOpItem>,
    /// 同步扩展额外拉入的兄弟文件数
    pub sibling_count: u32,
}

/// 单个文件失败明细
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BatchOpFailure {
    pub path: String,
    pub error: String,
}

/// `batch_op_execute` 返回
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BatchOpResult {
    pub success: u32,
    pub failed: u32,
    pub failures: Vec<BatchOpFailure>,
}

// ============================================================================
// 全局鸟种索引（统计视图）出入参（经 specta 导出到 bindings.ts）
// ============================================================================

/// 单鸟种聚合统计（get_species_stats 返回项；与 engine global_db::SpeciesStat 同构，
/// 核心层不含 serde，边界层在此补 serde + specta）
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SpeciesStat {
    pub bird_name: String,
    pub photo_count: i64,
    pub first_date: Option<String>,
    pub last_date: Option<String>,
    pub avg_sharpness: Option<f64>,
}

/// 单张照片定位（get_species_photos 返回项；前端拼绝对路径 = folder + '/' + rel_path）
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SpeciesPhoto {
    pub folder: String,
    pub rel_path: String,
}

/// 单鸟种识别命中率（get_correction_stats 返回项；与 engine global_db::CorrectionStat
/// 同构，核心层不含 serde，边界层在此补 serde + specta）。按 species_index 当前鸟种
/// 聚合：predicted = 被预测张数，corrected_away = 其中被人工改成别种的张数，
/// accuracy = 1 - corrected_away/predicted。specta 对浮点导出 number|null（防御性
/// NaN/Infinity 序列化），前端按 null 兜底。
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CorrectionStat {
    pub bird_name: String,
    pub predicted_count: i64,
    pub corrected_away_count: i64,
    pub accuracy: f64,
}

/// 统计概览（get_species_stats 返回）：鸟种列表（张数降序）+ 覆盖文件夹数（汇总条用）
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SpeciesOverview {
    pub stats: Vec<SpeciesStat>,
    pub folder_count: i64,
}

// ============================================================================
// 批量撤销（undo_batch_operation）出入参（经 specta 导出到 bindings.ts）
// ============================================================================

/// `undo_batch_operation` 返回：reverted = 成功撤销的文件数；
/// skipped = (路径, 原因) 列表，原因区分「条件不满足跳过」与「执行失败」。
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct UndoBatchResult {
    pub reverted: usize,
    pub skipped: Vec<(String, String)>,
}

// ============================================================================
// 直方图 / 剪切叠加 command 出入参（经 specta 导出到 bindings.ts）
// ============================================================================

/// `get_histogram` 返回：256 级 luma（BT.601 加权）+ RGB 三通道计数 + 剪切统计。
/// 各通道 Vec 恒为 256 个元素；totalPixels = 任一通道 bin 之和。
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct HistogramPayload {
    pub luma: Vec<u32>,
    pub r: Vec<u32>,
    pub g: Vec<u32>,
    pub b: Vec<u32>,
    /// 高光溢出像素数（luma >= 250）
    pub clip_high_count: u32,
    /// 死黑像素数（luma <= 5）
    pub clip_low_count: u32,
    /// 参与统计的总像素数
    pub total_pixels: u64,
}

/// 直方图/剪切叠加内存缓存（按 (完整路径, 文件大小)；同一张图重复查看避免重复解码，
/// 同名文件被覆盖时大小变化自动失效）。histogram 与 mask 独立惰性填充，
/// 命中时返回 Arc 克隆（不持锁解码）。
#[derive(Default)]
struct HistCache {
    histogram: HashMap<(String, u64), Arc<HistogramPayload>>,
    mask: HashMap<(String, u64), Arc<Vec<u8>>>,
}

// ============================================================================
// 后端状态
// ============================================================================

/// 后端权威状态。扫描结果全量持有（前端经 get_captures 一次拉取）。
pub struct AppState {
    /// 当前打开的照片目录（None = 尚未打开）
    current_dir: Option<PathBuf>,
    /// 当前扫描结果（权威副本，与前端全量同步）
    captures: Vec<CaptureMeta>,
    /// 文件夹级中心数据库（.pt/data.db）
    folder_db: Option<FolderDb>,
    /// 全局鸟种索引库（exe 同级 data/global.db；打开失败降级 None 不阻塞启动）
    global_db: Option<GlobalDb>,
    /// 缩略图缓存（.pt/thumbs，跟随目录）
    thumb_cache: Option<ThumbnailCache>,
    /// 直方图/剪切叠加内存缓存（按完整路径；切图后复用，避免重复解码）
    hist_cache: parking_lot::Mutex<HistCache>,
    /// 应用配置（PT.db）
    config: AppConfig,
    /// 配置文件路径（determine_config_path：便携优先）
    config_path: PathBuf,
    /// 扫描换代号：后台 EXIF/缩略图任务按代数丢弃过期结果（防张冠李戴）
    scan_generation: u64,
    /// 批量识别取消令牌：cancel_recognition 置位，识别循环逐张检查后提前退出
    recognition_cancel: Arc<AtomicBool>,
    /// 批量识别进行中标记：防并发识别（与 GPUI batch_recognizing 守卫一致）
    recognition_running: bool,
    /// 批量操作撤销日志（内存单槽，重启失效可接受：只记最近一次 Move/Copy 的逆操作）
    op_journal: photo_engine::undo::OpJournal,
}

impl AppState {
    fn new(config: AppConfig, config_path: PathBuf) -> Self {
        // 全局鸟种索引库：exe 同级 data/global.db（便携约定，与 pica_ref.db 同路径）；
        // 打开失败降级 None 不阻塞启动（统计视图显示空数据），失败仅记日志
        let global_db = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("data")))
            .and_then(|data_dir| match GlobalDb::open(&data_dir) {
                Ok(db) => Some(db),
                Err(e) => {
                    tracing::warn!("打开全局鸟种索引库失败（统计视图不可用）: {e}");
                    None
                }
            });
        Self {
            current_dir: None,
            captures: Vec::new(),
            folder_db: None,
            thumb_cache: None,
            hist_cache: parking_lot::Mutex::new(HistCache::default()),
            global_db,
            config,
            config_path,
            scan_generation: 0,
            recognition_cancel: Arc::new(AtomicBool::new(false)),
            recognition_running: false,
            op_journal: photo_engine::undo::OpJournal::new(),
        }
    }
}

/// 保存配置（失败仅记日志，与 GPUI 版 save_config 语义一致）
fn save_config(state: &AppState) {
    if let Err(e) = photo_config::save_config(&state.config_path, &state.config) {
        tracing::error!("保存配置失败: {e}");
    }
}

// ============================================================================
// commands（契约 §Commands；snake_case 函数名经 tauri-specta 转 camelCase 导出）
// ============================================================================

/// 弹出目录选择对话框，返回选中目录（取消返回 null）。
/// 只选目录，扫描由前端随后调 scan_directory 触发。
#[tauri::command]
#[specta::specta]
async fn pick_directory(app: AppHandle) -> Option<String> {
    // blocking_pick_folder 在 async command 的工作线程上调用（Windows 安全；
    // GPUI 版的 rfd RefCell 重入问题在 Tauri 下不存在）
    app.dialog()
        .file()
        .blocking_pick_folder()
        .and_then(|p| p.as_path().map(|p| p.to_string_lossy().to_string()))
}

/// 扫描目录：spawn_blocking 内 scanner 扫描 → folder_db 打开/三表同步 →
/// 读 exif_cache 回填 → 返回总数；随后后台任务做 EXIF 增量提取 + 缩略图预生成。
#[tauri::command]
#[specta::specta]
async fn scan_directory(app: AppHandle, path: String) -> u32 {
    scan_impl(app, PathBuf::from(path)).await
}

/// scan_directory 的实现主体（command 与启动自动恢复共用）
async fn scan_impl(app: AppHandle, dir: PathBuf) -> u32 {
    // 换代：在途的 EXIF/缩略图后台任务按代数丢弃（旧索引对新 captures 无意义）
    // + 读取扫描递归开关（AppConfig.include_subdirectories：true = 递归扫全部子层）
    let (generation, recursive) = {
        let state = app.state::<Mutex<AppState>>();
        let mut st = state.lock().expect("AppState 锁中毒");
        st.scan_generation += 1;
        (st.scan_generation, st.config.include_subdirectories)
    };
    match do_scan(&app, &dir, recursive).await {
        Ok((metas, folder_db)) => {
            let total = metas.len() as u32;
            let dir_str = dir.to_string_lossy().to_string();
            {
                let state = app.state::<Mutex<AppState>>();
                let mut st = state.lock().expect("AppState 锁中毒");
                st.captures = metas;
                st.folder_db = folder_db;
                st.current_dir = Some(dir.clone());
                // 缩略图缓存跟随文件夹：每目录独立 .pt/thumbs（与 .pt/data.db 同级）
                st.thumb_cache = Some(ThumbnailCache::new(dir.join(".pt").join("thumbs")));
                // 直方图/剪切叠加内存缓存按绝对路径键，换目录后旧路径条目失效，一并清空
                st.hist_cache.lock().histogram.clear();
                st.hist_cache.lock().mask.clear();
                // 记住最后打开的目录，下次启动自动恢复；最近目录去重、最新在前、最多 10 个
                st.config.last_directory = Some(dir_str.clone());
                st.config.recent_directories.retain(|d| d != &dir_str);
                st.config.recent_directories.insert(0, dir_str.clone());
                st.config.recent_directories.truncate(10);
                save_config(&st);
            }
            let _ = app.emit(
                "scan:done",
                ScanDone {
                    total,
                    directory: dir_str,
                },
            );
            // 后台：EXIF 增量提取（完成后 emit capture:enriched）+ 缩略图预生成（逐张 thumb:ready）
            let app_bg = app.clone();
            tauri::async_runtime::spawn(async move {
                enrich_and_pregen_thumbs(app_bg, generation).await;
            });
            total
        }
        Err(e) => {
            tracing::error!("扫描失败: {e}");
            // 哨兵复位：前端按 scan:done 结束加载态（GPUI 的 worker panic 兜底由前端状态机承担）
            let _ = app.emit(
                "scan:done",
                ScanDone {
                    total: 0,
                    directory: String::new(),
                },
            );
            0
        }
    }
}

/// 全量下推当前扫描结果（前端筛选/排序在 TS 侧做，零 IPC）
#[tauri::command]
#[specta::specta]
fn get_captures(state: State<'_, Mutex<AppState>>) -> Vec<CaptureMeta> {
    state.lock().expect("AppState 锁中毒").captures.clone()
}

/// 设置评分（0 = 清除）。写 folder_db xmp_meta 真相表 + 更新内存副本；
/// 失败返回 Err，前端重拉回滚（乐观更新回滚语义由前端承担）。
#[tauri::command]
#[specta::specta]
fn set_rating(
    state: State<'_, Mutex<AppState>>,
    paths: Vec<String>,
    rating: u8,
) -> Result<(), String> {
    let rating = match rating {
        0 => Rating::None,
        1 => Rating::One,
        2 => Rating::Two,
        3 => Rating::Three,
        4 => Rating::Four,
        5 => Rating::Five,
        _ => return Err(format!("评分必须在 0-5 之间，收到 {rating}")),
    };
    update_xmp(&state, &paths, |xmp| xmp.set_rating(rating), |meta| {
        meta.rating = rating
    })
}

/// 设置 Pick/Reject 旗标（None = 清除，对应 GPUI 的 U 键）
#[tauri::command]
#[specta::specta]
fn set_flag(
    state: State<'_, Mutex<AppState>>,
    paths: Vec<String>,
    flag: Option<Flag>,
) -> Result<(), String> {
    update_xmp(&state, &paths, |xmp| xmp.set_flag(flag), |meta| {
        meta.flag = flag
    })
}

/// 设置颜色标签（None = 清除）
#[tauri::command]
#[specta::specta]
fn set_color_label(
    state: State<'_, Mutex<AppState>>,
    paths: Vec<String>,
    label: Option<ColorLabel>,
) -> Result<(), String> {
    let label = label.unwrap_or(ColorLabel::None);
    update_xmp(
        &state,
        &paths,
        |xmp| xmp.set_color_label(label),
        |meta| meta.color_label = label,
    )
}

/// 设置关键词标签（全量替换语义）：写 folder_db keywords 真相表 + 更新内存副本。
/// 关键词归一化（去首尾空白/去空串/去重）与 set_rating 同语义：失败返回 Err，
/// 前端重拉回滚（乐观更新回滚语义由前端承担）。空数组 = 清空全部关键词。
#[tauri::command]
#[specta::specta]
fn set_keywords(
    state: State<'_, Mutex<AppState>>,
    paths: Vec<String>,
    keywords: Vec<String>,
) -> Result<(), String> {
    let mut st = state.lock().expect("AppState 锁中毒");
    let db = st.folder_db.clone().ok_or("尚未打开目录")?;
    let mut seen: HashSet<String> = HashSet::new();
    let cleaned: Vec<String> = keywords
        .into_iter()
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
        .filter(|k| seen.insert(k.clone()))
        .collect();
    for path in &paths {
        db.set_keywords(path, &cleaned).map_err(|e| e.to_string())?;
        if let Some(meta) = st.captures.iter_mut().find(|m| m.primary_path == *path) {
            meta.keywords = cleaned.clone();
        }
    }
    Ok(())
}

/// 评分/旗标/色标共用的持久化路径（照 metadata.rs：先读后写 xmp_meta，再同步内存）
fn update_xmp(
    state: &State<'_, Mutex<AppState>>,
    paths: &[String],
    set_xmp: impl Fn(&mut photo_domain::XmpMetadata),
    set_meta: impl Fn(&mut CaptureMeta),
) -> Result<(), String> {
    let mut st = state.lock().expect("AppState 锁中毒");
    let db = st.folder_db.clone().ok_or("尚未打开目录")?;
    for path in paths {
        let p = Path::new(path);
        let mut xmp = db.get_xmp(p).map_err(|e| e.to_string())?.unwrap_or_default();
        set_xmp(&mut xmp);
        db.put_xmp(p, &xmp).map_err(|e| e.to_string())?;
        if let Some(meta) = st.captures.iter_mut().find(|m| m.primary_path == *path) {
            set_meta(meta);
        }
    }
    Ok(())
}

// ============================================================================
// 辅助函数（Phase 2 新增 commands 共用）
// ============================================================================

/// 完整路径 → 相对当前目录的正斜杠相对路径（folder_db 键约定）
fn rel_path_of(dir: &Path, path: &str) -> Option<String> {
    let rel = Path::new(path).strip_prefix(dir).ok()?;
    Some(rel.to_string_lossy().replace('\\', "/"))
}

/// 从识别记录构造全局索引行（仅 bird 匹配成功者才有鸟种名；bird None 返回 None，
/// 由调用方跳过——未识别/无鸟记录不进鸟种索引）。updated_at 语义 = 识别/修正时间。
fn species_row(
    folder: &str,
    rel_path: &str,
    rec: &photo_domain::Recognition,
    date_taken: Option<String>,
) -> Option<SpeciesRow> {
    let bird = rec.bird.as_ref()?;
    Some(SpeciesRow {
        folder: folder.to_string(),
        rel_path: rel_path.to_string(),
        bird_name: bird.cn_name.clone(),
        confidence: rec.confidence.map(|c| c as f64),
        status: rec.status.as_str().to_string(),
        eye_sharpness: rec.eye_sharpness.map(|v| v as f64),
        date_taken,
        updated_at: rec.recognized_at.clone(),
    })
}

/// 从 CaptureMeta 构建单源 Capture（识别管线输入；批量操作执行请用重扫全量，
/// 因为 ops 层需要完整 source_files 才能操作同名兄弟文件）
fn build_capture_from_meta(meta: &CaptureMeta) -> photo_domain::Capture {
    let primary_path = Path::new(&meta.primary_path);
    let ext = primary_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let format = ImageFormat::from_extension(&ext).unwrap_or(ImageFormat::Jpeg);
    let source = SourceFile {
        path: primary_path.to_path_buf(),
        format,
        file_size: meta.file_size,
    };
    photo_domain::Capture {
        base_name: meta.base_name.clone(),
        source_files: vec![source],
        primary_index: 0,
    }
}

/// 构建识别器（模型目录 = exe 同级 models/ + data/pica_ref.db，照 GPUI build_recognizer）
fn build_recognizer() -> Result<photo_recognize::Recognizer, String> {
    let exe_dir = std::env::current_exe()
        .map_err(|e| format!("获取 exe 路径失败: {e}"))?
        .parent()
        .ok_or_else(|| "无法确定 exe 目录".to_string())?
        .to_path_buf();
    let models_dir = exe_dir.join("models");
    let catalog_db = exe_dir.join("data").join("pica_ref.db");
    photo_recognize::Recognizer::new(&models_dir, &catalog_db)
        .map_err(|e| format!("加载识别模型失败: {e}"))
}

/// ImageFormat 列表 → 大写格式字符串集合（与 engine capture_format_matches 同键约定）
fn formats_to_set(formats: &[ImageFormat]) -> HashSet<String> {
    formats.iter().map(|f| f.to_string().to_uppercase()).collect()
}

/// capture 主文件格式是否命中格式集合（只查主文件；engine 的 capture_format_matches 查任一源文件）
fn primary_format_matches(capture: &photo_domain::Capture, formats: &HashSet<String>) -> bool {
    let primary = &capture.source_files[capture.primary_index];
    formats.contains(&primary.format.to_string().to_uppercase())
}

/// 查询指定完整路径的调整参数（无记录/未打开目录/路径越界一律返回 None）。
/// 供 ptimg:// 协议 master 预览判定是否走调整渲染。
fn query_adjustments(app: &AppHandle, path: &Path) -> Option<AdjustParams> {
    let (db, dir) = {
        let state = app.state::<Mutex<AppState>>();
        let st = state.lock().expect("AppState 锁中毒");
        (st.folder_db.clone()?, st.current_dir.clone()?)
    };
    let rel = rel_path_of(&dir, &path.to_string_lossy())?;
    db.get_adjustments(&rel).ok().flatten()
}

// ============================================================================
// Phase 2 commands：收藏 / 最近 / 名录 / 批量识别 / 批量操作 / 调整参数
// ============================================================================

/// 列出收藏目录（AppConfig.favorite_dirs）
#[tauri::command]
#[specta::specta]
fn list_favorites(state: State<'_, Mutex<AppState>>) -> Vec<String> {
    state
        .lock()
        .expect("AppState 锁中毒")
        .config
        .favorite_dirs
        .clone()
}

/// 添加收藏目录（已存在则忽略；保存配置）
#[tauri::command]
#[specta::specta]
fn add_favorite(state: State<'_, Mutex<AppState>>, path: String) {
    let mut st = state.lock().expect("AppState 锁中毒");
    if !st.config.favorite_dirs.iter().any(|d| d == &path) {
        st.config.favorite_dirs.push(path);
        save_config(&st);
    }
}

/// 移除收藏目录并保存配置
#[tauri::command]
#[specta::specta]
fn remove_favorite(state: State<'_, Mutex<AppState>>, path: String) {
    let mut st = state.lock().expect("AppState 锁中毒");
    st.config.favorite_dirs.retain(|d| d != &path);
    save_config(&st);
}

/// 列出最近打开的目录（最新在前，最多 10 个；scan_impl 扫描时维护）
#[tauri::command]
#[specta::specta]
fn list_recent(state: State<'_, Mutex<AppState>>) -> Vec<String> {
    state
        .lock()
        .expect("AppState 锁中毒")
        .config
        .recent_directories
        .clone()
}

// ============================================================================
// 子目录树（T1 批次）：侧栏当前目录卡片下的懒加载目录树数据源
// ============================================================================

/// 侧栏目录树节点：name = 目录名，path = 完整路径，photoCount = 该目录**一层**
/// 直接包含的照片数（与 scanner 单层扫描同判据：viewable 扩展名，不含更深层）。
/// 更深层的子目录由前端在展开该节点时再调 list_subdirs 懒加载。
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SubdirInfo {
    pub name: String,
    pub path: String,
    pub photo_count: u32,
}

/// 列出 path 的一层子目录（侧栏目录树数据源；逐层展开逐层调本命令）。
/// photo_count 只统计每个子目录**直接**包含的照片数（单层语义，与扫描一致），
/// 不做递归——目录树按需懒加载，避免一次遍历全树。
/// 跳过 `.pt` 元数据目录（缩略图/中心库，非照片目录）。
#[tauri::command]
#[specta::specta]
async fn list_subdirs(path: String) -> Result<Vec<SubdirInfo>, String> {
    let base = PathBuf::from(path);
    tauri::async_runtime::spawn_blocking(move || {
        let mut out: Vec<SubdirInfo> = Vec::new();
        let rd = std::fs::read_dir(&base).map_err(|e| format!("读取目录失败: {e}"))?;
        for entry in rd.flatten() {
            let p = entry.path();
            if !p.is_dir() {
                continue;
            }
            // 跳过 .pt 元数据目录（缩略图缓存 / 中心库 data.db，非照片目录）
            if p.file_name().and_then(|n| n.to_str()) == Some(".pt") {
                continue;
            }
            // 单层照片计数：与 scanner 同扩展名判据（viewable 含图片/RAW/视频）
            let mut count = 0u32;
            if let Ok(sub) = std::fs::read_dir(&p) {
                for f in sub.flatten() {
                    let fp = f.path();
                    if !fp.is_file() {
                        continue;
                    }
                    let Some(ext) = fp.extension().and_then(|e| e.to_str()) else {
                        continue;
                    };
                    if ImageFormat::is_viewable(&ext.to_lowercase()) {
                        count += 1;
                    }
                }
            }
            out.push(SubdirInfo {
                name: p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default()
                    .to_string(),
                path: p.to_string_lossy().to_string(),
                photo_count: count,
            });
        }
        // 目录名小写排序，树内顺序稳定（Windows 大小写不敏感，展示统一小写序）
        out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        Ok(out)
    })
    .await
    .map_err(|e| format!("任务中断: {e}"))?
}

/// 名录库全量鸟种（拼音排序，筛选下拉数据源）。
/// 名录库在 exe 同级 data/pica_ref.db（与 Recognizer 同路径约定）；
/// photo-recognize 的 list_all_species 已按 cn_name_pinyin 排序（缺失回退中文名）。
#[tauri::command]
#[specta::specta]
fn list_bird_species() -> Result<Vec<String>, String> {
    let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
    else {
        return Err("无法确定 exe 目录".to_string());
    };
    let catalog_db = exe_dir.join("data").join("pica_ref.db");
    let list = photo_recognize::list_all_species(&catalog_db).map_err(|e| format!("加载名录失败: {e}"))?;
    let mut names: Vec<String> = list.into_iter().map(|b| b.cn_name).collect();
    // 与 GPUI ensure_correction_species 一致：双保险再按 Rust 字节序排一次
    names.sort();
    Ok(names)
}

/// 批量识别：spawn_blocking 后台逐张识别（多线程分块，每线程独占 Recognizer——
/// Session 需 &mut，不可跨线程共享，照 GPUI spawn_batch_recognize）。
/// 逐张 emit recognize:progress；每张完成后写 folder_db recognition 表（rel 键与代际
/// 无关，工作线程直接 upsert）+ 锁 AppState 更新内存 CaptureMeta（带代际校验）；
/// 全部完成 emit recognize:done 并复位进行中标记。单张失败不中止整体（failed 计数）。
/// 取消：cancel_recognition 置位令牌，各线程逐张检查后提前退出（取消也照常 emit done）。
#[tauri::command]
#[specta::specta]
async fn recognize_captures(app: AppHandle, paths: Vec<String>) -> Result<(), String> {
    // 守卫：已有识别任务进行中时拒绝并发（与 GPUI recognize_selected 一致）
    let (cancel, folder_db, thumb_cache, thread_count, dir, generation, global_db, dir_str, use_focus) = {
        let st = app.state::<Mutex<AppState>>();
        let mut st = st.lock().expect("AppState 锁中毒");
        if st.recognition_running {
            return Err("已有识别任务进行中".to_string());
        }
        st.recognition_running = true;
        st.recognition_cancel.store(false, Ordering::Relaxed);
        let dir = st
            .current_dir
            .clone()
            .ok_or_else(|| "尚未打开目录".to_string())?;
        (
            st.recognition_cancel.clone(),
            st.folder_db.clone(),
            st.thumb_cache.clone(),
            st.config.recognition_thread_count,
            dir.clone(),
            st.scan_generation,
            st.global_db.clone(),
            dir.to_string_lossy().to_string(),
            // 识别鸟体定位来源：Focus = 优先相机对焦点 ROI（无对焦点回退 YOLO）
            st.config.detection_source == photo_config::DetectionSource::Focus,
        )
    };

    // 构建目标列表：path（主文件绝对路径）→ (rel_path, Capture, focus_point, date_taken)；
    // 找不到对应 CaptureMeta 的路径跳过（文件可能已被移动/删除）
    let targets: Vec<(String, String, photo_domain::Capture, Option<photo_domain::FocusPoint>, Option<String>)> = {
        let st = app.state::<Mutex<AppState>>();
        let st = st.lock().expect("AppState 锁中毒");
        paths
            .iter()
            .filter_map(|path| {
                let meta = st.captures.iter().find(|m| m.primary_path == *path)?;
                let rel = rel_path_of(&dir, path)?;
                Some((
                    path.clone(),
                    rel,
                    build_capture_from_meta(meta),
                    meta.focus_point,
                    meta.date_taken.clone(),
                ))
            })
            .collect()
    };
    let total = targets.len();
    if total == 0 {
        let st = app.state::<Mutex<AppState>>();
        st.lock().expect("AppState 锁中毒").recognition_running = false;
        return Err("没有可识别的照片".to_string());
    }

    let app_work = app.clone();
    let generation_work = generation;
    tauri::async_runtime::spawn_blocking(move || {
        // 共享进度：工作线程逐张写原子计数 + 互斥当前文件（照 GPUI BatchProgress）
        #[derive(Default)]
        struct Shared {
            done: std::sync::atomic::AtomicUsize,
            confirmed: std::sync::atomic::AtomicUsize,
            needs_review: std::sync::atomic::AtomicUsize,
            unrecognized: std::sync::atomic::AtomicUsize,
            failed: std::sync::atomic::AtomicUsize,
            current: Mutex<String>,
        }
        let shared = Arc::new(Shared::default());
        let n_threads = (thread_count as usize).min(total).max(1);
        let chunk_size = total.div_ceil(n_threads).max(1);

        std::thread::scope(|s| {
            // 以引用形式供 move 闭包捕获，避免 FnMut 中移出外层变量
            let cancel = &cancel;
            let shared = &shared;
            let app = &app_work;
            let db = folder_db.as_ref();
            let gdb = global_db.as_ref();
            let cache = &thumb_cache;
            let generation = &generation_work;
            let targets = &targets;
            let dir_str = &dir_str;
            let use_focus = &use_focus;

            let handles: Vec<_> = targets
                .chunks(chunk_size)
                .map(|chunk| {
                    s.spawn(move || {
                        // 每线程懒加载自己的 Recognizer（DirectML 初始化 2-5s 一次性）
                        let mut recognizer = build_recognizer();
                        for (path, rel, cap, focus_point, date_taken) in chunk {
                            // 取消令牌：逐张检查，置位即提前退出
                            if cancel.load(Ordering::Relaxed) {
                                break;
                            }
                            *shared.current.lock().expect("进度锁中毒") = path.clone();
                            // 缩略图优先输入（JPEG DCT 毫秒级 / RAW 母版派生），
                            // 省每张全图 image::open；取消令牌顺带跳过慢 IO
                            let primary = &cap.source_files[cap.primary_index];
                            let thumb_bytes = cache.as_ref().and_then(|c| {
                                c.get_or_generate(primary, 2048, Some(cancel.as_ref())).ok()
                            });
                            // 焦点优先设置：有对焦点 → 走 ROI 路径跳过 YOLO；无对焦点 → 回退全图 YOLO
                            let focus_override = if *use_focus { *focus_point } else { None };
                            let rec_result = match &mut recognizer {
                                Ok(rec) => rec
                                    .recognize_with_thumbnail(cap, thumb_bytes.as_deref(), focus_override, None)
                                    .map_err(|e| format!("识别失败: {e}")),
                                Err(e) => Err(e.clone()),
                            };
                            // 计数（系统级失败单独统计，不并入 needs_review）
                            match &rec_result {
                                Ok(rec) => match rec.status {
                                    RecognitionStatus::Confirmed => {
                                        shared.confirmed.fetch_add(1, Ordering::Relaxed)
                                    }
                                    RecognitionStatus::NeedsReview => {
                                        shared.needs_review.fetch_add(1, Ordering::Relaxed)
                                    }
                                    RecognitionStatus::Unrecognized => {
                                        shared.unrecognized.fetch_add(1, Ordering::Relaxed)
                                    }
                                },
                                Err(_) => shared.failed.fetch_add(1, Ordering::Relaxed),
                            };
                            shared.done.fetch_add(1, Ordering::Relaxed);
                            // 逐张进度
                            let done = shared.done.load(Ordering::Relaxed);
                            let _ = app.emit(
                                "recognize:progress",
                                RecognizeProgress {
                                    done: done as u32,
                                    total: total as u32,
                                    current_path: path.clone(),
                                },
                            );
                            // 写库（rel 键与代际无关，工作线程直接 upsert，照 GPUI B3①）
                            if let Ok(rec) = &rec_result
                                && let Some(db) = db
                            {
                                if let Err(e) = db.upsert_recognition(rel, rec) {
                                    tracing::error!("写入识别结果失败 {rel}: {e}");
                                }
                            }
                            // 全局鸟种索引 upsert（仅 bird 匹配成功者；失败只记日志不中断识别）
                            if let Ok(rec) = &rec_result
                                && let Some(gdb) = gdb
                                && let Some(row) = species_row(&dir_str, rel, rec, date_taken.clone())
                            {
                                if let Err(e) = gdb.upsert_rows(&[row]) {
                                    tracing::error!("全局鸟种索引 upsert 失败 {rel}: {e}");
                                }
                            }
                            // 更新内存 CaptureMeta（锁 AppState；代际不匹配则丢弃，防张冠李戴）
                            {
                                let st = app.state::<Mutex<AppState>>();
                                let mut st = st.lock().expect("AppState 锁中毒");
                                if st.scan_generation == *generation
                                    && let Some(meta) =
                                        st.captures.iter_mut().find(|m| m.primary_path == *path)
                                    && let Ok(rec) = &rec_result
                                {
                                    meta.enrich_with_recognition(rec);
                                }
                            }
                        }
                    })
                })
                .collect();
            // 单线程 panic 不应吞掉其他线程：join 只为检出 panic
            for h in handles {
                if h.join().is_err() {
                    tracing::error!("批量识别某工作线程 panic，该线程未完成的结果丢失");
                }
            }
        });
        // 汇总 emit recognize:done（取消也照常 emit，前端据此结束加载态）
        let _ = app_work.emit(
            "recognize:done",
            RecognizeDone {
                total: total as u32,
                confirmed: shared.confirmed.load(Ordering::Relaxed) as u32,
                needs_review: shared.needs_review.load(Ordering::Relaxed) as u32,
                unrecognized: shared.unrecognized.load(Ordering::Relaxed) as u32,
                failed: shared.failed.load(Ordering::Relaxed) as u32,
            },
        );
        // 复位进行中标记
        let st = app_work.state::<Mutex<AppState>>();
        st.lock().expect("AppState 锁中毒").recognition_running = false;
    })
    .await
    .map_err(|e| format!("识别任务中断: {e}"))?;
    Ok(())
}

/// 请求取消批量识别：置位取消令牌，识别循环逐张检查后提前退出
#[tauri::command]
#[specta::specta]
fn cancel_recognition(state: State<'_, Mutex<AppState>>) {
    state
        .lock()
        .expect("AppState 锁中毒")
        .recognition_cancel
        .store(true, Ordering::Relaxed);
}

/// 批量操作干跑：只计算操作集与目标路径，不碰文件。
/// 操作集 = 当前扫描结果全量（Tauri 端筛选在 TS 侧，后端无筛选状态，前端在无筛选
/// 条件时自行禁用）；formats 非空时按主文件格式过滤；sync_siblings 时按引擎
/// expand_with_siblings 把同名兄弟并入（formats 即兄弟格式白名单）。siblingCount = 扩展新增数。
#[tauri::command]
#[specta::specta]
fn batch_op_preview(
    state: State<'_, Mutex<AppState>>,
    op: photo_domain::BatchOpType,
    options: BatchOpOptions,
) -> Result<BatchOpPreview, String> {
    let st = state.lock().expect("AppState 锁中毒");
    // 从内存 CaptureMeta 构建单源 Capture（干跑只需 primary 信息；执行时另行重扫取全量）
    let caps: Vec<photo_domain::Capture> =
        st.captures.iter().map(build_capture_from_meta).collect();
    let formats = formats_to_set(&options.formats);
    let mut indices: Vec<usize> = (0..caps.len()).collect();
    if !formats.is_empty() {
        indices.retain(|&i| primary_format_matches(&caps[i], &formats));
    }
    let base_count = indices.len() as u32;
    if options.sync_siblings && !formats.is_empty() {
        indices = photo_engine::batch_ops::expand_with_siblings(&caps, &indices, &formats);
    }
    let sibling_count = (indices.len() as u32).saturating_sub(base_count);
    let items: Vec<BatchOpItem> = indices
        .iter()
        .filter_map(|&i| caps.get(i))
        .map(|cap| {
            let primary = &cap.source_files[cap.primary_index];
            let target_path = match (op.needs_target_dir(), &options.target_dir) {
                (true, Some(dir)) => Some(format!(
                    "{}/{}",
                    dir.trim_end_matches(['/', '\\']),
                    cap.base_name
                )),
                _ => None,
            };
            BatchOpItem {
                path: primary.path.to_string_lossy().to_string(),
                target_path,
            }
        })
        .collect();
    Ok(BatchOpPreview {
        op,
        count: items.len() as u32,
        items,
        sibling_count,
    })
}

/// 批量操作执行：spawn_blocking 后台执行（engine::batch_ops::execute），逐文件 emit
/// batch:progress，完成 emit batch:done。语义照 GPUI run_batch_op：
/// 1. 重扫源目录取完整 Capture（ops 层需要 source_files 全列表操作兄弟文件）
/// 2. 操作集 = 全量；formats 非空按主文件格式过滤；sync_siblings 时 expand_with_siblings
/// 3. Delete 走 ops::delete_capture（回收站）；Move/Delete 后的重扫由前端负责
///    （前端会重调 scan_directory，本命令不触发）
#[tauri::command]
#[specta::specta]
async fn batch_op_execute(
    app: AppHandle,
    op: photo_domain::BatchOpType,
    options: BatchOpOptions,
) -> Result<BatchOpResult, String> {
    // 目标目录校验：需要目录的操作必须提供非空目录（与 engine execute 内部校验一致，提前报错）
    if op.needs_target_dir()
        && options.target_dir.as_deref().map_or(true, |d| d.is_empty())
    {
        return Err("目标目录未指定".to_string());
    }
    let (dir, _generation, recursive, global_db, folder_db) = {
        let st = app.state::<Mutex<AppState>>();
        let st = st.lock().expect("AppState 锁中毒");
        (
            st.current_dir
                .clone()
                .ok_or_else(|| "尚未打开目录".to_string())?,
            st.scan_generation,
            // 重扫范围与原始扫描一致：递归模式下子目录文件也在批量操作集内
            st.config.include_subdirectories,
            st.global_db.clone(),
            st.folder_db.clone(),
        )
    };
    let target_dir = options.target_dir.map(PathBuf::from);
    let formats = formats_to_set(&options.formats);
    let sync_siblings = options.sync_siblings;
    let app_work = app.clone();

    let (result, journal_ops) = tauri::async_runtime::spawn_blocking(
        move || -> Result<(BatchOpResult, Vec<photo_engine::undo::UndoOp>), String> {
            // 1. 重扫源目录，取完整 Capture（批量操作需要 source_files 全列表）；
            //    扫描深度跟随配置（递归模式下子目录文件也在操作集内）
            let caps = if recursive {
                scanner::scan_directory_recursive(&dir, &FilterCriteria::default(), None)
            } else {
                scanner::scan_directory(&dir, &FilterCriteria::default(), None)
            }
            .map_err(|e| format!("扫描失败: {e}"))?;
            // 2. 操作集 = 全量；formats 过滤（主文件格式）；同步扩展
            let mut indices: Vec<usize> = (0..caps.len()).collect();
            if !formats.is_empty() {
                indices.retain(|&i| primary_format_matches(&caps[i], &formats));
            }
            if sync_siblings && !formats.is_empty() {
                indices = photo_engine::batch_ops::expand_with_siblings(&caps, &indices, &formats);
            }
            // 与索引同序的路径表：on_progress(done, _) 的 done-1 即当前处理项
            // （execute 内部按 indices 顺序处理，越界项跳过时也不回调，序号恒对齐）
            let paths: Vec<String> = indices
                .iter()
                .filter_map(|&i| caps.get(i))
                .map(|c| c.source_files[c.primary_index].path.to_string_lossy().to_string())
                .collect();
            // 3. 撤销日志预快照：Copy 只记录「副本之前不存在」的条目（避免撤销时误删
            //    操作前就存在的同名文件）；Move 全量记录（撤销时存在性自检兜底）
            let pre_existing: HashSet<PathBuf> = if op == photo_domain::BatchOpType::Copy {
                // Copy 必带目标目录（命令入口已校验），此处兜底空集
                let mut set = HashSet::new();
                if let Some(td) = target_dir.as_deref() {
                    set = indices
                        .iter()
                        .filter_map(|&i| caps.get(i))
                        .flat_map(|c| c.source_files.iter())
                        .filter_map(|sf| sf.path.file_name().map(|n| td.join(n)))
                        .filter(|p| p.exists())
                        .collect();
                }
                set
            } else {
                HashSet::new()
            };
            // 3.5 执行；进度回调逐文件 emit（闭包借用 paths 与 app 的 clone，仅在线程闭包体内引用）
            let paths_ref = &paths;
            let app_progress = app_work.clone();
            let results = photo_engine::batch_ops::execute(
                &caps,
                &indices,
                op,
                target_dir.as_deref(),
                move |done, total| {
                    let current = paths_ref.get(done as usize - 1).cloned().unwrap_or_default();
                    let _ = app_progress.emit(
                        "batch:progress",
                        BatchProgress {
                            done,
                            total,
                            current_path: current,
                        },
                    );
                },
            );
            // 4. 汇总：成功消息形如 "复制: name"，失败消息含 "失败"
            let mut success = 0;
            let mut failures = Vec::new();
            for (result, path) in results.iter().zip(paths.iter()) {
                if result.contains("失败") {
                    failures.push(BatchOpFailure {
                        path: path.clone(),
                        error: result.clone(),
                    });
                } else {
                    success += 1;
                }
            }
            // 5. 构建撤销日志（仅 Move/Copy；Delete 走回收站不在撤销范围）：
            //    按 capture 粒度对齐 results —— 该 capture 失败则不记录（无法确定部分状态）；
            //    Copy 排除操作前已存在的目标文件（撤销时按存在性自检兜底）
            let journal_ops = if matches!(
                op,
                photo_domain::BatchOpType::Move | photo_domain::BatchOpType::Copy
            ) {
                let td = target_dir.as_deref();
                indices
                    .iter()
                    .zip(results.iter())
                    .filter(|(_, r)| !r.contains("失败"))
                    .filter_map(|(&i, _)| caps.get(i))
                    .flat_map(|c| c.source_files.iter())
                    .filter_map(move |sf| {
                        let from = sf.path.clone();
                        let to = td.and_then(|d| from.file_name().map(|n| d.join(n)))?;
                        match op {
                            photo_domain::BatchOpType::Move => {
                                Some(photo_engine::undo::UndoOp::Move { from, to })
                            }
                            photo_domain::BatchOpType::Copy => {
                                (!pre_existing.contains(&to))
                                    .then(|| photo_engine::undo::UndoOp::Copy { from, to })
                            }
                            _ => None,
                        }
                    })
                    .collect()
            } else {
                Vec::new()
            };
            let failed = failures.len() as u32;
            // 5.5 全局鸟种索引同步：Move/Delete 成功项删除源文件夹对应行
            //（文件已不在源目录；目标目录的索引行由该目录后续扫描 replace_folder 补入）
            let dir_str = dir.to_string_lossy().to_string();
            if matches!(
                op,
                photo_domain::BatchOpType::Move | photo_domain::BatchOpType::Delete
            ) && let Some(gdb) = &global_db
            {
                let rels: Vec<String> = results
                    .iter()
                    .zip(paths.iter())
                    .filter(|(result, _)| !result.contains("失败"))
                    .filter_map(|(_, path)| rel_path_of(&dir, path))
                    .collect();
                if let Err(e) = gdb.delete_rows(&dir_str, &rels) {
                    tracing::error!("全局鸟种索引删除行失败: {e}");
                }
            }
            let _ = app_work.emit("batch:done", BatchDone { success, failed });
            // 5.6 folder_db 四表正向同步（Move=迁到目标库并删源行 / Copy=复制到目标库保留源行）。
            //     best-effort 失败仅记日志；目标库不存在则创建（承接元数据，与扫描建库语义一致）。
            //     键约定：xmp_meta/keywords=完整路径；recognition/adjustments=相对各自文件夹根。
            if matches!(
                op,
                photo_domain::BatchOpType::Move | photo_domain::BatchOpType::Copy
            ) && let (Some(src_db), Some(td)) = (&folder_db, target_dir.as_deref())
            {
                let is_move = op == photo_domain::BatchOpType::Move;
                let entries: Vec<(String, String)> = indices
                    .iter()
                    .zip(results.iter())
                    .filter(|(_, r)| !r.contains("失败"))
                    .filter_map(|(&i, _)| caps.get(i))
                    .flat_map(|c| c.source_files.iter())
                    .filter_map(|sf| {
                        let from = sf.path.to_string_lossy().to_string();
                        let to = sf
                            .path
                            .file_name()
                            .map(|n| td.join(n).to_string_lossy().to_string())?;
                        Some((from, to))
                    })
                    .collect();
                if !entries.is_empty() {
                    match FolderDb::open_in_dir(td) {
                        Ok(mut dst_db) => {
                            if is_move {
                                if let Err(e) = photo_engine::ops::sync_move_xmp(src_db, &mut dst_db, &entries) {
                                    tracing::warn!("批量移动：xmp_meta 同步失败: {e}");
                                }
                                if let Err(e) = photo_engine::ops::sync_move_keywords(src_db, &mut dst_db, &entries) {
                                    tracing::warn!("批量移动：关键词同步失败: {e}");
                                }
                            } else {
                                if let Err(e) = src_db.copy_xmp_rows_to(&mut dst_db, &entries) {
                                    tracing::warn!("批量复制：xmp_meta 同步失败: {e}");
                                }
                                if let Err(e) = photo_engine::ops::sync_copy_keywords(src_db, &mut dst_db, &entries) {
                                    tracing::warn!("批量复制：关键词同步失败: {e}");
                                }
                            }
                            let rel_entries: Vec<(String, String)> = entries
                                .iter()
                                .filter_map(|(from, to)| {
                                    Some((rel_path_of(&dir, from)?, rel_path_of(td, to)?))
                                })
                                .collect();
                            if is_move {
                                if let Err(e) = photo_engine::ops::sync_move_recognitions(src_db, &mut dst_db, &rel_entries) {
                                    tracing::warn!("批量移动：识别行同步失败: {e}");
                                }
                                if let Err(e) = photo_engine::ops::sync_move_adjustments(src_db, &mut dst_db, &rel_entries) {
                                    tracing::warn!("批量移动：调整行同步失败: {e}");
                                }
                            } else {
                                if let Err(e) = photo_engine::ops::sync_copy_recognitions(src_db, &mut dst_db, &rel_entries) {
                                    tracing::warn!("批量复制：识别行同步失败: {e}");
                                }
                                if let Err(e) = photo_engine::ops::sync_copy_adjustments(src_db, &mut dst_db, &rel_entries) {
                                    tracing::warn!("批量复制：调整行同步失败: {e}");
                                }
                            }
                        }
                        Err(e) => tracing::warn!("批量操作：打开目标库失败，元数据未同步 ({td:?}): {e}"),
                    }
                }
            }
            Ok((
                BatchOpResult {
                    success,
                    failed,
                    failures,
                },
                journal_ops,
            ))
        },
    )
    .await
    .map_err(|e| format!("批量操作任务中断: {e}"))??;
    // 6. 记录撤销日志（单槽；仅当有成功项，失败批次不覆盖旧日志）
    if result.success > 0 && !journal_ops.is_empty() {
        let st = app.state::<Mutex<AppState>>();
        st.lock()
            .expect("AppState 锁中毒")
            .op_journal
            .record(journal_ops);
    }
    Ok(result)
}

/// 撤销最近一次批量操作（移动/复制/模板重命名；删除走回收站不在范围）：
/// 1. 取走内存日志（单槽：撤销后即空，再次 Ctrl+Z 报「没有可撤销的批量操作」）
/// 2. 逐条执行逆操作（move 回移 / rename 改回 / copy 删副本），跳过/失败不中止后续
/// 3. 成功撤销的 Move/Rename 逆向同步 xmp_meta/识别/调整行（复用 sync_move_xmp /
///    sync_rename_xmp 反向调用；正向批次未做 sidecar 同步时为空操作，幂等无害）
/// 4. scan_impl 全量重扫（更新内存 captures + emit scan:done 驱动前端 reload）
#[tauri::command]
#[specta::specta]
async fn undo_batch_operation(app: AppHandle) -> Result<UndoBatchResult, String> {
    // 取走日志快照与当前目录/库（文件 IO 不持 AppState 锁；folder_db 供元数据逆向同步复用）
    let (ops, dir, current_dir, current_db) = {
        let st = app.state::<Mutex<AppState>>();
        let mut st = st.lock().expect("AppState 锁中毒");
        let ops = st.op_journal.take();
        if ops.is_empty() {
            return Err("没有可撤销的批量操作".to_string());
        }
        let dir = st
            .current_dir
            .clone()
            .ok_or_else(|| "尚未打开目录".to_string())?;
        (ops, dir.clone(), Some(dir.clone()), st.folder_db.clone())
    };

    let result = tauri::async_runtime::spawn_blocking(move || -> Result<UndoBatchResult, String> {
        use photo_engine::undo::UndoError;
        let mut current_db = current_db;
        // 1. 文件逆操作：逐条独立，跳过/失败进 skipped（reverted + skipped = 总条目）
        let outcomes = photo_engine::undo::undo_ops(&ops);
        let mut reverted = 0usize;
        let mut skipped = Vec::new();
        for outcome in &outcomes {
            match &outcome.result {
                Ok(()) => reverted += 1,
                Err(UndoError::Skipped(reason)) => {
                    skipped.push((outcome.op.target().to_string_lossy().to_string(), reason.clone()));
                }
                Err(UndoError::Failed(e)) => {
                    skipped.push((outcome.op.target().to_string_lossy().to_string(), e.to_string()));
                }
            }
        }
        // 2. 元数据反向同步（best-effort：失败仅记日志，不中止撤销结果）
        reverse_sync_metadata(&outcomes, current_dir.as_deref(), current_db.as_mut());
        Ok(UndoBatchResult { reverted, skipped })
    })
    .await
    .map_err(|e| format!("撤销任务中断: {e}"))??;

    // 3. 撤销后全量重扫（文件已回到原位置，captures 需重建 + scan:done 驱动前端 reload）
    let _ = scan_impl(app, dir).await;
    Ok(result)
}

/// 成功撤销的 Move/Rename 条目，把 xmp_meta/识别/调整行逆向迁回（best-effort，失败仅记日志）。
/// 键约定：xmp_meta 用完整路径；recognition/adjustments 用相对文件夹根的正斜杠路径。
/// 目标库（行当前所在）→ 源库（行应回迁处）；源目录 = 当前目录时优先复用其 folder_db。
fn reverse_sync_metadata(
    outcomes: &[photo_engine::undo::UndoOutcome],
    current_dir: Option<&Path>,
    mut current_db: Option<&mut FolderDb>,
) {
    use photo_engine::undo::UndoOp;

    // 收集成功撤销的 (to, from) 全路径对：Move 跨库迁移；Rename 同库改名
    let mut moves: Vec<(PathBuf, PathBuf)> = Vec::new(); // (to, from)
    let mut renames: Vec<(PathBuf, PathBuf)> = Vec::new(); // (to, from)
    for o in outcomes {
        if o.result.is_err() {
            continue;
        }
        match &o.op {
            UndoOp::Move { from, to } => moves.push((to.clone(), from.clone())),
            UndoOp::Rename { from, to } => renames.push((to.clone(), from.clone())),
            UndoOp::Copy { .. } => {} // 复制不产生元数据迁移
        }
    }

    // Move：按 (to 目录, from 目录) 分组，一次打开一对库批量同步
    let mut by_dir_pair: std::collections::HashMap<(PathBuf, PathBuf), Vec<(String, String)>> =
        std::collections::HashMap::new();
    for (to, from) in &moves {
        if let (Some(to_dir), Some(from_dir)) = (to.parent().map(Path::to_path_buf), from.parent().map(Path::to_path_buf)) {
            by_dir_pair
                .entry((to_dir, from_dir))
                .or_default()
                .push((to.to_string_lossy().to_string(), from.to_string_lossy().to_string()));
        }
    }
    for ((to_dir, from_dir), entries) in by_dir_pair {
        // 源库：from 目录即当前目录 → 复用 state 的 folder_db；否则按需打开
        let mut from_db_owned;
        let from_db: &mut FolderDb = if current_dir == Some(from_dir.as_path()) {
            let Some(db) = current_db.as_mut() else { continue };
            db
        } else {
            let Some(db) = open_db_if_exists(&from_dir) else { continue };
            from_db_owned = db;
            &mut from_db_owned
        };
        let Some(mut to_db) = open_db_if_exists(&to_dir) else { continue };
        // xmp/keywords 键 = 完整路径（entries 直接可用）
        if let Err(e) = photo_engine::ops::sync_move_xmp(&to_db, from_db, &entries) {
            tracing::warn!("撤销移动：xmp_meta 逆向同步失败 ({to_dir:?} → {from_dir:?}): {e}");
        }
        if let Err(e) = photo_engine::ops::sync_move_keywords(&to_db, from_db, &entries) {
            tracing::warn!("撤销移动：关键词逆向同步失败 ({to_dir:?} → {from_dir:?}): {e}");
        }
        // 识别/调整键 = 相对各自文件夹根的正斜杠路径
        let rel_entries: Vec<(String, String)> = entries
            .iter()
            .filter_map(|(to, from)| {
                Some((
                    Path::new(to).strip_prefix(&to_dir).ok()?.to_string_lossy().replace('\\', "/"),
                    Path::new(from)
                        .strip_prefix(&from_dir)
                        .ok()?
                        .to_string_lossy()
                        .replace('\\', "/"),
                ))
            })
            .collect();
        if let Err(e) = photo_engine::ops::sync_move_recognitions(&to_db, from_db, &rel_entries) {
            tracing::warn!("撤销移动：识别行逆向同步失败: {e}");
        }
        if let Err(e) = photo_engine::ops::sync_move_adjustments(&to_db, from_db, &rel_entries) {
            tracing::warn!("撤销移动：调整行逆向同步失败: {e}");
        }
    }

    // Rename：同目录单库改名（xmp 完整路径；识别/调整相对路径）
    if renames.is_empty() {
        return;
    }
    let Some(dir) = renames[0].0.parent().map(Path::to_path_buf) else {
        return;
    };
    let Some(db) = open_db_if_exists(&dir) else { return };
    for (to, from) in &renames {
        let to_s = to.to_string_lossy().to_string();
        let from_s = from.to_string_lossy().to_string();
        if let Err(e) = photo_engine::ops::sync_rename_xmp(&db, &to_s, &from_s) {
            tracing::warn!("撤销重命名：xmp_meta 逆向同步失败: {e}");
        }
        if let Err(e) = photo_engine::ops::sync_rename_keywords(&db, &to_s, &from_s) {
            tracing::warn!("撤销重命名：关键词逆向同步失败: {e}");
        }
        if let (Some(to_rel), Some(from_rel)) = (
            Path::new(to).strip_prefix(&dir).ok().map(|r| r.to_string_lossy().replace('\\', "/")),
            Path::new(from).strip_prefix(&dir).ok().map(|r| r.to_string_lossy().replace('\\', "/")),
        ) {
            if let Err(e) = photo_engine::ops::sync_rename_recognition(&db, &to_rel, &from_rel) {
                tracing::warn!("撤销重命名：识别行逆向同步失败: {e}");
            }
            if let Err(e) = photo_engine::ops::sync_rename_adjustment(&db, &to_rel, &from_rel) {
                tracing::warn!("撤销重命名：调整行逆向同步失败: {e}");
            }
        }
    }
}

/// 目录已有 .pt/data.db 才打开（避免为不存在的库凭空创建空文件）；无库/打开失败返回 None
fn open_db_if_exists(dir: &Path) -> Option<FolderDb> {
    if !dir.join(".pt").join("data.db").exists() {
        return None;
    }
    FolderDb::open_in_dir(dir).ok()
}

/// 读取单张调整参数（ADR 0007；无记录返回全零 = 无调整）
#[tauri::command]
#[specta::specta]
fn get_adjustments(state: State<'_, Mutex<AppState>>, path: String) -> AdjustParams {
    let st = state.lock().expect("AppState 锁中毒");
    let Some(db) = st.folder_db.as_ref() else {
        return AdjustParams::default();
    };
    let Some(dir) = st.current_dir.as_ref() else {
        return AdjustParams::default();
    };
    let Some(rel) = rel_path_of(dir, &path) else {
        return AdjustParams::default();
    };
    db.get_adjustments(&rel).ok().flatten().unwrap_or_default()
}

/// 设置调整参数：持久化到 folder_db adjustments 表 + emit thumb:ready 触发预览刷新
/// （前端按事件失效缓存并以新 ?v= 重载 master 预览；ptimg handler 侧带调整参数时
/// 经引擎渲染输出）。防御 DB 坏值：Q15 定点饱和要求 saturation∈[-100,100]，
/// 钳制后再入内存（同 GPUI refresh_adjustments_sync）。
#[tauri::command]
#[specta::specta]
fn set_adjustments(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    path: String,
    params: AdjustParams,
) -> Result<(), String> {
    let params = AdjustParams {
        exposure: params.exposure.clamp(-2.0, 2.0),
        contrast: params.contrast.clamp(-100, 100),
        saturation: params.saturation.clamp(-100, 100),
    };
    // 克隆句柄后释放锁：SQLite 写不持 AppState 锁
    let (db, rel) = {
        let st = state.lock().expect("AppState 锁中毒");
        let db = st.folder_db.clone().ok_or("尚未打开目录")?;
        let dir = st.current_dir.clone().ok_or("尚未打开目录")?;
        let rel = rel_path_of(&dir, &path).ok_or("路径不在当前目录内")?;
        (db, rel)
    };
    db.put_adjustments(&rel, &params).map_err(|e| e.to_string())?;
    // 触发预览刷新：前端收到后按 thumb:ready 失效缓存并以新 ?v= 重载 master
    let _ = app.emit("thumb:ready", ThumbReady { path });
    Ok(())
}

/// 内存缓存键：完整路径 + 文件大小（对齐 thumbnail 缓存键的失效语义——
/// 同名文件被覆盖/重导回时大小变化自动失效，避免直方图/剪切叠加显示旧图数据）
fn hist_cache_key(path: &str) -> (String, u64) {
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    (path.to_string(), size)
}

/// 计算直方图（预览尺寸解码，JPEG DCT 降采样 / RAW half_size 预览，不解 24MP 全尺寸）。
/// spawn_blocking 内调用（不阻塞 IPC 线程）；失败（解码异常/非图片）返回错误文案。
fn compute_histogram_payload(path: &Path) -> Result<HistogramPayload, String> {
    let data = photo_engine::histogram::compute_histogram_from_file(path)
        .map_err(|e| e.to_string())?;
    Ok(HistogramPayload {
        luma: data.luma.to_vec(),
        r: data.r.to_vec(),
        g: data.g.to_vec(),
        b: data.b.to_vec(),
        clip_high_count: data.clip_high_count,
        clip_low_count: data.clip_low_count,
        total_pixels: data.total_pixels(),
    })
}

/// 计算直方图（预览尺寸解码，JPEG DCT 降采样 / RAW half_size 预览，不解 24MP 全尺寸）。
/// 内存缓存按 (路径, 文件大小)（InfoPanel 切图回来复用；文件覆盖自动失效）；
/// 失败返回文案（如 RAW 解码异常）。
#[tauri::command]
#[specta::specta]
async fn get_histogram(
    state: State<'_, Mutex<AppState>>,
    path: String,
) -> Result<HistogramPayload, String> {
    let key = hist_cache_key(&path);
    // 命中缓存：克隆 Arc 直接返回（不持锁解码）
    if let Some(hit) = {
        let st = state.lock().expect("AppState 锁中毒");
        st.hist_cache.lock().histogram.get(&key).cloned()
    } {
        return Ok((*hit).clone());
    }
    let payload = {
        let path_buf = PathBuf::from(&path);
        tauri::async_runtime::spawn_blocking(move || compute_histogram_payload(&path_buf))
            .await
            .map_err(|e| format!("直方图计算任务失败: {e}"))?
    }?;
    let st = state.lock().expect("AppState 锁中毒");
    st.hist_cache
        .lock()
        .histogram
        .insert(key, Arc::new(payload.clone()));
    Ok(payload)
}

/// 剪切叠加图（RGBA PNG 字节：红 = 高光溢出、蓝 = 死黑，其余透明；长边 ~800）。
/// 加载不阻塞主图：前端异步拉取后以主图同尺寸叠放；内存缓存按 (路径, 文件大小)。
#[tauri::command]
#[specta::specta]
async fn get_clipping_mask(state: State<'_, Mutex<AppState>>, path: String) -> Result<Vec<u8>, String> {
    let key = hist_cache_key(&path);
    if let Some(hit) = {
        let st = state.lock().expect("AppState 锁中毒");
        st.hist_cache.lock().mask.get(&key).cloned()
    } {
        return Ok((*hit).clone());
    }
    let bytes = {
        let path_buf = PathBuf::from(&path);
        tauri::async_runtime::spawn_blocking(move || {
            photo_engine::histogram::clipping_mask_png(
                &path_buf,
                photo_engine::histogram::CLIP_MASK_LONG_EDGE,
            )
            .map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| format!("剪切叠加生成任务失败: {e}"))?
    }?;
    let st = state.lock().expect("AppState 锁中毒");
    st.hist_cache
        .lock()
        .mask
        .insert(key, Arc::new(bytes.clone()));
    Ok(bytes)
}

/// 读取完整配置（前端启动时据此应用主题/字体等；AppConfig 已 derive specta）
#[tauri::command]
#[specta::specta]
fn get_app_config(state: State<'_, Mutex<AppState>>) -> AppConfig {
    state
        .lock()
        .expect("AppState 锁中毒")
        .config
        .clone()
}

// ============================================================================
// Phase 3 commands：识别读取 / 手动修正 / 单张删除 / 调整导出 / 字体 / 设置
// ============================================================================

/// 读取单张完整识别结果（InfoPanel 识别 tab）。
/// 按完整路径查 folder_db recognition 表（与 get_adjustments 同键约定：正斜杠 rel）；
/// 无记录 / 未打开目录 / 路径越界一律返回 None（前端据此显示「未识别」）。
#[tauri::command]
#[specta::specta]
fn get_recognition(
    state: State<'_, Mutex<AppState>>,
    path: String,
) -> Result<Option<Recognition>, String> {
    let st = state.lock().expect("AppState 锁中毒");
    let Some(db) = st.folder_db.as_ref() else {
        return Ok(None);
    };
    let Some(dir) = st.current_dir.as_ref() else {
        return Ok(None);
    };
    let Some(rel) = rel_path_of(dir, &path) else {
        return Ok(None);
    };
    db.get_recognition(&rel).map_err(|e| e.to_string())
}

/// 手动修正鸟种（对齐 GPUI correct_bird_by_name）：名录库按中文名匹配 → 构造
/// Confirmed Recognition（人工指定即权威结论：置信度 100%，保留原检测框/眼锐度/
/// 眼框数据）→ 写 recognition 表 → 同步内存 CaptureMeta（bird_name/confidence/
/// status）→ emit thumb:ready 供前端刷新网格识别 chip。
#[tauri::command]
#[specta::specta]
fn correct_bird(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    path: String,
    bird_name: String,
) -> Result<(), String> {
    // 名录库查找（exe 同级 data/pica_ref.db，与 list_bird_species 同路径约定）
    let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
    else {
        return Err("无法确定 exe 目录".to_string());
    };
    let catalog_db = exe_dir.join("data").join("pica_ref.db");
    let bird = photo_recognize::list_all_species(&catalog_db)
        .map_err(|e| format!("加载名录失败: {e}"))?
        .into_iter()
        .find(|b| b.cn_name == bird_name)
        .ok_or_else(|| format!("名录中不存在鸟种: {bird_name}"))?;

    // 克隆句柄后释放锁：SQLite 读写不持 AppState 锁
    let (db, dir) = {
        let st = state.lock().expect("AppState 锁中毒");
        (
            st.folder_db.clone().ok_or("尚未打开目录")?,
            st.current_dir.clone().ok_or("尚未打开目录")?,
        )
    };
    let rel = rel_path_of(&dir, &path).ok_or("路径不在当前目录内")?;

    // 人工指定鸟种名在 rec 构造前取出（bird 随后被 move 进 rec，日志引用需独立副本）
    let new_bird_name = bird.cn_name.clone();

    // 保留原检测框/眼数据，只改鸟种与状态（照 GPUI correct_bird_by_name）
    let prev = db.get_recognition(&rel).map_err(|e| e.to_string())?;
    let rec = Recognition {
        status: RecognitionStatus::Confirmed,
        bird: Some(bird),
        class_index: prev.as_ref().and_then(|r| r.class_index),
        confidence: Some(100.0),
        bbox: prev.as_ref().and_then(|r| r.bbox),
        eye_sharpness: prev.as_ref().and_then(|r| r.eye_sharpness),
        eye_bbox: prev.as_ref().and_then(|r| r.eye_bbox),
        candidates: vec![],
        failure_stage: RecognitionFailureStage::None,
        recognized_at: chrono::Utc::now().to_rfc3339(),
    };
    db.upsert_recognition(&rel, &rec).map_err(|e| e.to_string())?;

    // 同步内存 CaptureMeta + 全局鸟种索引 upsert（人工修正 = 权威结论；
    // bird 恒为 Some，行必然写入；date_taken 取内存 meta 的 EXIF 拍摄时间）
    {
        let mut st = state.lock().expect("AppState 锁中毒");
        let dir_str = dir.to_string_lossy().to_string();
        let date_taken = st
            .captures
            .iter()
            .find(|m| m.primary_path == path)
            .and_then(|m| m.date_taken.clone());
        if let Some(gdb) = &st.global_db
            && let Some(row) = species_row(&dir_str, &rel, &rec, date_taken)
        {
            if let Err(e) = gdb.upsert_rows(&[row]) {
                tracing::error!("全局鸟种索引 upsert 失败 {rel}: {e}");
            }
            // 修正审计日志（只追加）：old = 修正前模型预测（无预测记录则跳过，
            // 如从未识别直接人工指定），new = 人工指定鸟种；供命中率统计反查
            if let Some(old) = prev.as_ref().and_then(|r| r.bird.as_ref()).map(|b| b.cn_name.as_str())
                && let Err(e) = gdb.log_correction(
                    &dir_str,
                    &rel,
                    old,
                    &new_bird_name,
                    prev.as_ref().and_then(|r| r.confidence).map(|c| c as f64),
                )
            {
                tracing::error!("修正审计日志写入失败 {rel}: {e}");
            }
        }
        if let Some(meta) = st.captures.iter_mut().find(|m| m.primary_path == path) {
            meta.enrich_with_recognition(&rec);
        }
    }
    // emit thumb:ready：前端按事件刷新网格识别 chip
    let _ = app.emit("thumb:ready", ThumbReady { path });
    Ok(())
}

/// 单张/多张删除（回收站，无确认——对齐 GPUI Delete 键语义）。
/// 与 batch_op_execute 的 Delete 分支同编排：重扫源目录取完整 Capture（ops 层
/// delete_capture 需要 source_files 才能操作同名兄弟文件）→ 逐个删除 → 仅删除
/// 成功者同步 sidecar 三表（识别/调整/评分色标旗标行，防孤儿行）→ 从内存
/// captures 移除并重索引 → scan_generation 换代（在途 EXIF/缩略图任务按代数
/// 丢弃，防张冠李戴）→ emit scan:done {total, directory}，前端 store 据此自动
/// reload()。任一失败不中止整体（与批量语义一致）；全部失败才返回 Err。
#[tauri::command]
#[specta::specta]
async fn delete_captures(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    paths: Vec<String>,
) -> Result<(), String> {
    if paths.is_empty() {
        return Ok(());
    }
    // 换代 + 快照目录/DB 句柄（克隆后释放锁，文件 IO 不持 AppState 锁）
    let (dir, db, recursive, global_db) = {
        let mut st = state.lock().expect("AppState 锁中毒");
        st.scan_generation += 1;
        (
            st.current_dir.clone().ok_or("尚未打开目录")?,
            st.folder_db.clone(),
            // 重扫范围必须与原始扫描一致：递归模式下子目录照片也要能被找到删除
            st.config.include_subdirectories,
            st.global_db.clone(),
        )
    };
    let target: HashSet<String> = paths.into_iter().collect();
    let app_work = app.clone();
    let dir_str = dir.to_string_lossy().to_string();

    let result = tauri::async_runtime::spawn_blocking(move || -> Result<u32, String> {
        // 1. 重扫源目录取完整 Capture（同 batch_op_execute）；扫描深度跟随配置
        let caps = if recursive {
            scanner::scan_directory_recursive(&dir, &FilterCriteria::default(), None)
        } else {
            scanner::scan_directory(&dir, &FilterCriteria::default(), None)
        }
        .map_err(|e| format!("扫描失败: {e}"))?;
        // 2. 删除命中的 capture（回收站）；失败只记日志，成功才进 sidecar 同步
        let mut deleted: Vec<(String, Option<String>)> = Vec::new(); // (主路径, rel)
        let mut errors: Vec<String> = Vec::new();
        for cap in &caps {
            let primary = &cap.source_files[cap.primary_index];
            let primary_str = primary.path.to_string_lossy().to_string();
            if !target.contains(&primary_str) {
                continue;
            }
            match photo_engine::ops::delete_capture(cap) {
                Ok(()) => {
                    let rel = primary
                        .path
                        .strip_prefix(&dir)
                        .ok()
                        .map(|r| r.to_string_lossy().replace('\\', "/"));
                    deleted.push((primary_str, rel));
                }
                Err(e) => errors.push(format!("{primary_str}: {e}")),
            }
        }
        if deleted.is_empty() {
            return Err(if errors.is_empty() {
                "没有找到要删除的照片".to_string()
            } else {
                format!("删除失败: {}", errors.join("；"))
            });
        }
        // 3. sidecar 三表同步（仅删除成功者；识别/调整按 rel 键，xmp 按完整路径键）
        if let Some(db) = &db {
            let rels: Vec<String> = deleted.iter().filter_map(|(_, rel)| rel.clone()).collect();
            let full_paths: Vec<String> = deleted.iter().map(|(p, _)| p.clone()).collect();
            if let Err(e) = photo_engine::ops::sync_delete_recognitions(db, &rels) {
                tracing::error!("删除识别行失败: {e}");
            }
            if let Err(e) = photo_engine::ops::sync_delete_adjustments(db, &rels) {
                tracing::error!("删除调整行失败: {e}");
            }
            if let Err(e) = photo_engine::ops::sync_delete_xmp(db, &full_paths) {
                tracing::error!("删除评分/色标/旗标行失败: {e}");
            }
            if let Err(e) = photo_engine::ops::sync_delete_keywords(db, &full_paths) {
                tracing::error!("删除关键词行失败: {e}");
            }
        }
        // 3.5 全局鸟种索引同步：删除对应行（folder + rel 复合主键）。
        // dir_str 在闭包外用（scan:done 负载），这里就地计算避免移出捕获
        if let Some(gdb) = &global_db {
            let rels: Vec<String> = deleted.iter().filter_map(|(_, rel)| rel.clone()).collect();
            let folder_str = dir.to_string_lossy().to_string();
            if let Err(e) = gdb.delete_rows(&folder_str, &rels) {
                tracing::error!("全局鸟种索引删除行失败: {e}");
            }
        }
        // 4. 从内存 captures 移除 + 重索引（照 GPUI delete_selected）
        {
            let st = app_work.state::<Mutex<AppState>>();
            let mut st = st.lock().expect("AppState 锁中毒");
            let deleted_primaries: HashSet<&str> =
                deleted.iter().map(|(p, _)| p.as_str()).collect();
            st.captures.retain(|m| !deleted_primaries.contains(m.primary_path.as_str()));
            for (i, meta) in st.captures.iter_mut().enumerate() {
                meta.index = i;
            }
        }
        // 部分失败不中止整体：日志记录，总数只计成功者
        if !errors.is_empty() {
            tracing::warn!("部分删除失败: {}", errors.join("；"));
        }
        Ok(deleted.len() as u32)
    })
    .await
    .map_err(|e| format!("删除任务中断: {e}"))??;

    // 5. emit scan:done：前端 store 据此结束哨兵并 reload() 全量
    let _ = app.emit(
        "scan:done",
        ScanDone {
            total: result,
            directory: dir_str,
        },
    );
    Ok(())
}

/// 导出调整结果（全尺寸烘焙，ADR 0007）：engine adjustments 渲染 + convert 保存
/// JPEG。命名 `{stem}_adjusted.jpg`，已存在自动追加 `_1/_2` 序号（不覆盖原文件，
/// 照 GPUI export_adjusted）。output_dir = None 时导出到源文件所在目录。RAW
/// 全尺寸 16-bit 解码约 3-5s，spawn_blocking 异步执行。返回最终输出路径（前端
/// 状态栏展示）。导出是一次性烘焙，不改动内存 CaptureMeta（无对应字段）。
#[tauri::command]
#[specta::specta]
async fn export_adjusted(
    state: State<'_, Mutex<AppState>>,
    path: String,
    output_dir: Option<String>,
) -> Result<String, String> {
    // 锁内快照：源文件信息 + 调整参数 + 目标目录（克隆后释放锁，烘焙不持锁）
    let (source, params, out_dir) = {
        let st = state.lock().expect("AppState 锁中毒");
        let db = st.folder_db.clone().ok_or("尚未打开目录")?;
        let dir = st.current_dir.clone().ok_or("尚未打开目录")?;
        let rel = rel_path_of(&dir, &path).ok_or("路径不在当前目录内")?;
        let params = db
            .get_adjustments(&rel)
            .map_err(|e| e.to_string())?
            .unwrap_or_default();
        let meta = st
            .captures
            .iter()
            .find(|m| m.primary_path == path)
            .ok_or("找不到该照片")?;
        let source = SourceFile {
            path: PathBuf::from(&meta.primary_path),
            format: ImageFormat::from_extension(
                Path::new(&meta.primary_path)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or(""),
            )
            .unwrap_or(ImageFormat::Jpeg),
            file_size: meta.file_size,
        };
        let out_dir = match output_dir {
            Some(d) if !d.trim().is_empty() => PathBuf::from(d),
            _ => PathBuf::from(&meta.primary_path)
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default(),
        };
        (source, params, out_dir)
    };

    tauri::async_runtime::spawn_blocking(move || {
        let stem = source
            .path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "photo".to_string());
        // 防覆盖：已存在则追加 _1/_2 序号（照 GPUI export_adjusted）
        let mut out = out_dir.join(format!("{stem}_adjusted.jpg"));
        let mut n = 1;
        while out.exists() {
            out = out_dir.join(format!("{stem}_adjusted_{n}.jpg"));
            n += 1;
        }
        photo_engine::convert::export_adjusted(&source, &params, &out)
            .map(|p| p.to_string_lossy().to_string())
            .map_err(|e| format!("导出失败: {e}"))
    })
    .await
    .map_err(|e| format!("导出任务中断: {e}"))?
}

/// 系统字体枚举（设置面板字体下拉数据源）。
/// zed-font-kit（font-kit 的 zed fork，workspace 已 pin）：Windows 走 DirectWrite
/// 系统字体集合（SystemSource::new() 无失败路径）；family 名排序后返回。
#[tauri::command]
#[specta::specta]
fn list_system_fonts() -> Result<Vec<String>, String> {
    let source = font_kit::source::SystemSource::new();
    let mut families = source
        .all_families()
        .map_err(|e| format!("枚举系统字体失败: {e}"))?;
    families.sort();
    Ok(families)
}

/// 更新并保存配置（设置面板）：钳制校验后替换 st.config + save_config。
/// 钳制范围：leftPanelWidth 200–480、rightPanelWidth 200–480、
/// recognitionThreadCount 1–4、thumbnailSize 64–1024（网格 cell = 尺寸 + 56，
/// 越界值钳到合理区间，与 GPUI 设置语义一致，非法输入不报错）。
#[tauri::command]
#[specta::specta]
fn set_app_config(state: State<'_, Mutex<AppState>>, config: AppConfig) -> Result<(), String> {
    let mut st = state.lock().expect("AppState 锁中毒");
    st.config = AppConfig {
        thumbnail_size: config.thumbnail_size.clamp(64, 1024),
        favorite_dirs: config.favorite_dirs,
        last_directory: config.last_directory,
        recent_directories: config.recent_directories,
        theme: config.theme,
        left_panel_width: config.left_panel_width.clamp(200, 480),
        right_panel_visible: config.right_panel_visible,
        right_panel_width: config.right_panel_width.clamp(200, 480),
        font_family: config.font_family,
        recognition_thread_count: config.recognition_thread_count.clamp(1, 4),
        // 识别鸟体定位来源：两态枚举（Yolo/Focus），无钳制直接透传（下次批量识别生效）
        detection_source: config.detection_source,
        // 布尔开关无钳制范围，直接透传（决定下次扫描单层/递归）
        include_subdirectories: config.include_subdirectories,
        // 导出预设：质量钳制 1-100、长边 0 → None（ExportPreset::clamped）
        export_presets: config
            .export_presets
            .into_iter()
            .map(photo_config::ExportPreset::clamped)
            .collect(),
        // 堆叠模式：三态枚举（None/ByFileName/ByTime），无钳制直接透传（网格按配置即时重排）
        stack_mode: config.stack_mode,
        // 网格每行图片数：钳制 2-5（下拉栏只出这 4 个选项，防手改配置越界）
        grid_columns: config.grid_columns.clamp(2, 5),
        // 界面缩放比例：钳制 70-200%（下拉栏 75/100/125/150/175/200 选项，25% 递增）
        ui_scale: config.ui_scale.clamp(70, 200),
    };
    save_config(&st);
    Ok(())
}

// ============================================================================
// T1 批次 commands：批量重命名（模板）+ 批量导出（预设）
// ============================================================================

/// 批量重命名（模板模式，T1 批次）：对 `paths`（前端按筛选结果主路径传入，
/// 顺序即序号顺序）应用命名模板。占位符数据从内存 CaptureMeta 取——鸟种
/// `bird_name`、拍摄日期 `date_taken`、相机型号 `camera_model`，序号从
/// `start_seq` 递增（补零 3 位，template.rs 渲染）。
///
/// 重扫源目录取完整 Capture（ops 层需要 source_files 全列表同步改名兄弟文件）；
/// 成功后同步 folder_db 三表键（recognition/adjustments 相对路径、xmp_meta 完整
/// 路径）。逐张 emit `batch:progress`，完成 emit `batch:done`（复用批量操作进度
/// 通道，与 batch_op_execute 语义一致）。返回 `BatchOpResult` 形态汇总。
#[tauri::command]
#[specta::specta]
async fn batch_rename(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    paths: Vec<String>,
    template: String,
    start_seq: u32,
) -> Result<BatchOpResult, String> {
    let (dir, metas, db) = {
        let st = state.lock().expect("AppState 锁中毒");
        (
            st.current_dir
                .clone()
                .ok_or_else(|| "尚未打开目录".to_string())?,
            st.captures.clone(),
            st.folder_db.clone(),
        )
    };
    let meta_by_path: HashMap<String, CaptureMeta> = metas
        .into_iter()
        .map(|m| (m.primary_path.clone(), m))
        .collect();
    let app_work = app.clone();

    let result = tauri::async_runtime::spawn_blocking(
        move || -> Result<(BatchOpResult, Vec<photo_engine::undo::UndoOp>), String> {
        // 1. 重扫源目录取完整 Capture（与 batch_op_execute 一致）
        let caps = scanner::scan_directory(&dir, &FilterCriteria::default(), None)
            .map_err(|e| format!("扫描失败: {e}"))?;
        // 2. 按 paths 顺序组装 (Capture, 元数据) 对；重扫后找不到的路径跳过
        let mut ordered: Vec<photo_domain::Capture> = Vec::new();
        let mut ordered_metas: Vec<CaptureMeta> = Vec::new();
        for path in &paths {
            let Some(cap) = caps.iter().find(|c| {
                c.source_files
                    .get(c.primary_index)
                    .is_some_and(|f| f.path.to_string_lossy() == *path)
            }) else {
                continue;
            };
            let Some(meta) = meta_by_path.get(path) else {
                continue;
            };
            ordered.push(cap.clone());
            ordered_metas.push(meta.clone());
        }
        if ordered.is_empty() {
            return Err("没有可重命名的文件（路径可能已失效）".to_string());
        }
        // 3. 主路径 → ctx 引用表（鸟种/日期/相机来自内存 CaptureMeta）
        let ctx_by_path: HashMap<String, NameTemplateContext> = ordered
            .iter()
            .zip(ordered_metas.iter())
            .map(|(cap, meta)| {
                let primary = cap.source_files[cap.primary_index]
                    .path
                    .to_string_lossy()
                    .to_string();
                (
                    primary,
                    NameTemplateContext {
                        name: cap.base_name.clone(),
                        species: meta.bird_name.clone(),
                        date: meta.date_taken.clone(),
                        camera: meta.camera_model.clone(),
                        seq: 0,
                    },
                )
            })
            .collect();
        let capture_refs: Vec<&photo_domain::Capture> = ordered.iter().collect();
        // 4. 执行（序号/占位符在 ops 层按序递增渲染）
        let app_progress = app_work.clone();
        let results = photo_engine::ops::rename_captures_templated(
            &capture_refs,
            &template,
            start_seq,
            |cap| {
                let primary = cap
                    .source_files
                    .get(cap.primary_index)
                    .map(|f| f.path.to_string_lossy().to_string())
                    .unwrap_or_default();
                ctx_by_path.get(&primary).cloned().unwrap_or_else(|| {
                    NameTemplateContext {
                        name: cap.base_name.clone(),
                        ..Default::default()
                    }
                })
            },
        );
        // 5. 同步 folder_db 四表键（recognition/adjustments 相对路径、xmp_meta/keywords
        //    完整路径）；失败仅记日志，不中止整体（键错位会在下次扫描孤儿清理时收敛）。
        //    old_paths 与 results 同序（source_files 扁平序），同时供撤销日志使用
        let old_paths: Vec<String> = ordered
            .iter()
            .flat_map(|c| {
                c.source_files
                    .iter()
                    .map(|f| f.path.to_string_lossy().to_string())
            })
            .collect();
        if let Some(fdb) = &db {
            for (old_full, (new_path, res)) in old_paths.iter().zip(results.iter()) {
                if res.is_err() {
                    continue;
                }
                let new_full = new_path.to_string_lossy().to_string();
                if let Err(e) = photo_engine::ops::sync_rename_xmp(fdb, old_full, &new_full) {
                    tracing::error!("重命名同步 xmp 行失败 {old_full}: {e}");
                }
                if let Err(e) = photo_engine::ops::sync_rename_keywords(fdb, old_full, &new_full) {
                    tracing::error!("重命名同步关键词行失败 {old_full}: {e}");
                }
                if let (Some(old_rel), Some(new_rel)) =
                    (rel_path_of(&dir, old_full), rel_path_of(&dir, &new_full))
                {
                    if let Err(e) =
                        photo_engine::ops::sync_rename_recognition(fdb, &old_rel, &new_rel)
                    {
                        tracing::error!("重命名同步识别行失败 {old_full}: {e}");
                    }
                    if let Err(e) =
                        photo_engine::ops::sync_rename_adjustment(fdb, &old_rel, &new_rel)
                    {
                        tracing::error!("重命名同步调整行失败 {old_full}: {e}");
                    }
                }
            }
        }
        // 6. 汇总 + 事件（进度按文件计，done = 已处理数，成功/失败都推进）
        let mut processed = 0u32;
        let mut success = 0u32;
        let mut failures = Vec::new();
        let file_total = results.len() as u32;
        for (new_path, res) in &results {
            processed += 1;
            match res {
                Ok(()) => success += 1,
                Err(e) => failures.push(BatchOpFailure {
                    path: new_path.to_string_lossy().to_string(),
                    error: e.to_string(),
                }),
            }
            let _ = app_progress.emit(
                "batch:progress",
                BatchProgress {
                    done: processed,
                    total: file_total,
                    current_path: new_path.to_string_lossy().to_string(),
                },
            );
        }
        let failed = failures.len() as u32;
        let _ = app_work.emit("batch:done", BatchDone { success, failed });
        // 撤销日志：成功项记 Rename 逆操作（old_paths 与 results 同序），失败项无法
        // 确定部分状态不记录（对齐 batch_op_execute 的日志粒度）
        let journal_ops: Vec<photo_engine::undo::UndoOp> = old_paths
            .iter()
            .zip(results.iter())
            .filter(|(_, (_, res))| res.is_ok())
            .map(|(old_full, (new_path, _))| photo_engine::undo::UndoOp::Rename {
                from: PathBuf::from(old_full),
                to: new_path.clone(),
            })
            .collect();
        Ok((
            BatchOpResult {
                success,
                failed,
                failures,
            },
            journal_ops,
        ))
    })
    .await
    .map_err(|e| format!("批量重命名任务中断: {e}"))??;
    let (result, journal_ops) = result;
    // 记录撤销日志（单槽；仅当有成功项，失败批次不覆盖旧日志，对齐 batch_op_execute）
    if result.success > 0 && !journal_ops.is_empty() {
        let st = app.state::<Mutex<AppState>>();
        st.lock()
            .expect("AppState 锁中毒")
            .op_journal
            .record(journal_ops);
    }
    Ok(result)
}

/// 批量导出（预设模式，T1 批次）：对 `paths`（顺序即序号顺序）逐张按命名模板渲染
/// 输出基名，`convert::export_with_preset` 渲染（长边限制 + JPEG 质量 + 色调调整）。
/// 占位符数据从内存 CaptureMeta 取（鸟种/日期/相机），序号从 `start_seq` 递增；
/// 调整参数从 folder_db 读取（与 export_adjusted 同源）。同名输出已存在时自动追加
/// `_1/_2` 序号（照 export_adjusted 防覆盖语义，不覆盖原文件）。
///
/// 逐张 emit `export:progress`，完成 emit `export:done`（独立通道，避免与批量操作
/// 进度互踩）。返回 `BatchOpResult` 形态汇总。
#[tauri::command]
#[specta::specta]
async fn export_captures(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    paths: Vec<String>,
    output_dir: String,
    long_edge: Option<u32>,
    quality: u8,
    template: String,
    start_seq: u32,
) -> Result<BatchOpResult, String> {
    let (dir, metas, db) = {
        let st = state.lock().expect("AppState 锁中毒");
        (
            st.current_dir.clone().ok_or_else(|| "尚未打开目录".to_string())?,
            st.captures.clone(),
            st.folder_db.clone(),
        )
    };
    let meta_by_path: HashMap<String, CaptureMeta> = metas
        .into_iter()
        .map(|m| (m.primary_path.clone(), m))
        .collect();
    let out_dir = PathBuf::from(output_dir);
    let app_work = app.clone();

    let result = tauri::async_runtime::spawn_blocking(move || -> Result<BatchOpResult, String> {
        let mut success = 0u32;
        let mut failures = Vec::new();
        let total = paths.len() as u32;
        for (i, path) in paths.iter().enumerate() {
            let Some(meta) = meta_by_path.get(path) else {
                failures.push(BatchOpFailure {
                    path: path.clone(),
                    error: "找不到该照片（路径可能已失效）".into(),
                });
                let _ = app_work.emit(
                    "export:progress",
                    ExportProgress {
                        done: i as u32 + 1,
                        total,
                        current_path: path.clone(),
                    },
                );
                continue;
            };
            // 源文件 + 调整参数（与 export_adjusted 同源）
            let source = SourceFile {
                path: PathBuf::from(&meta.primary_path),
                format: ImageFormat::from_extension(
                    Path::new(&meta.primary_path)
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or(""),
                )
                .unwrap_or(ImageFormat::Jpeg),
                file_size: meta.file_size,
            };
            let params = match (&db, rel_path_of(&dir, path)) {
                (Some(fdb), Some(rel)) => fdb
                    .get_adjustments(&rel)
                    .ok()
                    .flatten()
                    .unwrap_or_default(),
                _ => AdjustParams::default(),
            };
            // 命名模板渲染 → 防覆盖
            let base = photo_engine::template::render_name_template(
                &template,
                &NameTemplateContext {
                    name: meta.base_name.clone(),
                    species: meta.bird_name.clone(),
                    date: meta.date_taken.clone(),
                    camera: meta.camera_model.clone(),
                    seq: start_seq + i as u32,
                },
            );
            let mut out = out_dir.join(format!("{base}.jpg"));
            let mut n = 1;
            while out.exists() {
                out = out_dir.join(format!("{base}_{n}.jpg"));
                n += 1;
            }
            match photo_engine::convert::export_with_preset(
                &source,
                &params,
                long_edge,
                quality,
                &out,
            ) {
                Ok(_) => success += 1,
                Err(e) => failures.push(BatchOpFailure {
                    path: path.clone(),
                    error: format!("导出失败: {e}"),
                }),
            }
            let _ = app_work.emit(
                "export:progress",
                ExportProgress {
                    done: i as u32 + 1,
                    total,
                    current_path: path.clone(),
                },
            );
        }
        let failed = failures.len() as u32;
        let _ = app_work.emit("export:done", ExportDone { success, failed });
        Ok(BatchOpResult {
            success,
            failed,
            failures,
        })
    })
    .await
    .map_err(|e| format!("批量导出任务中断: {e}"))??;
    Ok(result)
}

// ============================================================================
// Phase 3.5 commands：全局鸟种索引（统计视图）
// ============================================================================

/// 全局鸟种统计（统计视图汇总条 + 左栏列表）：全库聚合、张数降序。
/// 数据源 = 启动时打开的 exe 同级 data/global.db；打开失败（降级 None）返回空概览。
#[tauri::command]
#[specta::specta]
fn get_species_stats(state: State<'_, Mutex<AppState>>) -> Result<SpeciesOverview, String> {
    let st = state.lock().expect("AppState 锁中毒");
    let Some(gdb) = &st.global_db else {
        return Ok(SpeciesOverview {
            stats: vec![],
            folder_count: 0,
        });
    };
    let stats = gdb
        .species_stats()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|s| SpeciesStat {
            bird_name: s.bird_name,
            photo_count: s.photo_count,
            first_date: s.first_date,
            last_date: s.last_date,
            avg_sharpness: s.avg_sharpness,
        })
        .collect();
    let folder_count = gdb.distinct_folder_count().map_err(|e| e.to_string())?;
    Ok(SpeciesOverview { stats, folder_count })
}

/// 某鸟种全部照片定位（统计视图右栏网格）：folder + rel_path，前端拼绝对路径后
/// 经 ptimgUrl('thumb', 绝对路径) 渲染缩略图。
#[tauri::command]
#[specta::specta]
fn get_species_photos(
    state: State<'_, Mutex<AppState>>,
    bird_name: String,
) -> Result<Vec<SpeciesPhoto>, String> {
    let st = state.lock().expect("AppState 锁中毒");
    let Some(gdb) = &st.global_db else {
        return Ok(vec![]);
    };
    let photos = gdb
        .photos_of_species(&bird_name)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|(folder, rel_path)| SpeciesPhoto { folder, rel_path })
        .collect();
    Ok(photos)
}

/// 全局识别命中率（统计视图「识别命中率」区块）：按鸟种聚合 predicted /
/// corrected_away / accuracy（accuracy = 1 - corrected_away/predicted）。
/// 数据源 = 启动时打开的 exe 同级 data/global.db；打开失败（降级 None）返回空。
#[tauri::command]
#[specta::specta]
fn get_correction_stats(state: State<'_, Mutex<AppState>>) -> Result<Vec<CorrectionStat>, String> {
    let st = state.lock().expect("AppState 锁中毒");
    let Some(gdb) = &st.global_db else {
        return Ok(vec![]);
    };
    let stats = gdb
        .correction_stats()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|s| CorrectionStat {
            bird_name: s.bird_name,
            predicted_count: s.predicted_count,
            corrected_away_count: s.corrected_away_count,
            accuracy: s.accuracy,
        })
        .collect();
    Ok(stats)
}

/// 高频鸟种（修正鸟种下拉「常用」分组）：species_index 按张数降序、去 NULL/空名。
/// 本机使用频次即区域相关性代理（离线替代区域名录）。
#[tauri::command]
#[specta::specta]
fn get_frequent_species(
    state: State<'_, Mutex<AppState>>,
    limit: u32,
) -> Result<Vec<String>, String> {
    let st = state.lock().expect("AppState 锁中毒");
    let Some(gdb) = &st.global_db else {
        return Ok(vec![]);
    };
    gdb.frequent_species(limit as usize).map_err(|e| e.to_string())
}

// ============================================================================
// Phase 4 commands：导入（ImportRebuild）——驱动器检测 / 源扫描 / 日期分组计划 / 复制或移动
// ============================================================================

/// 可移动驱动器（`list_import_drives` 返回）
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ImportDrive {
    /// 根路径（Windows 如 "E:\\"，Linux 为挂载点如 "/run/media/user/CANON"）
    pub path: String,
    /// 卷标（读取失败/空卷标为 None）
    pub label: Option<String>,
}

/// 导入候选（`scan_import_source` 返回 / `plan_import` 输入）
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ImportCandidate {
    /// 源文件完整路径
    pub path: String,
    /// 拍摄日期 YYYY-MM-DD（EXIF 优先，回退 mtime）
    pub date: String,
    /// 文件大小（字节，去重用）
    pub size: u64,
}

/// 导入计划组（目标目录 = destRoot/dateDir/）
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ImportGroup {
    /// 日期目录名（YYYY-MM-DD，相对 destRoot）
    pub date_dir: String,
    /// 组内源文件完整路径
    pub files: Vec<String>,
}

/// 计划阶段被跳过的文件
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ImportSkipped {
    /// 源文件完整路径
    pub path: String,
    /// 跳过原因（目标已存在且大小相同 / 同名冲突等）
    pub reason: String,
}

/// 导入计划（`plan_import` 返回 / `execute_import` 输入；干跑不碰文件）
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ImportPlan {
    /// 按日期分组（YYYY-MM-DD）
    pub groups: Vec<ImportGroup>,
    /// 跳过清单
    pub skipped: Vec<ImportSkipped>,
}

/// 导入执行模式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub enum ImportMode {
    /// 复制（源保留）
    Copy,
    /// 移动（源删除；跨文件系统走 copy + delete 回退）
    Move,
}

/// `import:progress` 事件负载：逐文件进度
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ImportProgress {
    pub done: u32,
    pub total: u32,
    /// 当前处理的源文件路径
    pub current: String,
}

/// `import:done` 事件负载：导入汇总
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ImportDone {
    pub imported: u32,
    pub skipped: u32,
    pub failed: u32,
}

/// `execute_import` 返回（与 import:done 同内容，供 invoke 调用方直接使用）
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub imported: u32,
    pub skipped: u32,
    pub failed: u32,
}

/// 检测可移动驱动器（Windows 原生 API：GetLogicalDrives + GetDriveTypeW +
/// GetVolumeInformationW；非 Windows 返回空）。SD 卡/U 盘即 DRIVE_REMOVABLE。
#[tauri::command]
#[specta::specta]
fn list_import_drives() -> Vec<ImportDrive> {
    photo_engine::import::detect_removable_drives()
        .into_iter()
        .map(|d| ImportDrive {
            path: d.path,
            label: d.label,
        })
        .collect()
}

/// 递归扫描导入源（整棵子树，不限 DCIM）：EXIF 拍摄日期优先，回退 mtime。
/// 返回候选列表（前端据此展示「N 张」并送入 plan_import 干跑）。
#[tauri::command]
#[specta::specta]
async fn scan_import_source(path: String) -> Result<Vec<ImportCandidate>, String> {
    let src = PathBuf::from(path);
    tauri::async_runtime::spawn_blocking(move || {
        photo_engine::import::scan_import_source(&src)
            .map(|cands| {
                cands
                    .into_iter()
                    .map(|c| ImportCandidate {
                        path: c.path.to_string_lossy().to_string(),
                        date: c.date,
                        size: c.size,
                    })
                    .collect()
            })
            .map_err(|e| format!("扫描导入源失败: {e}"))
    })
    .await
    .map_err(|e| format!("扫描导入源任务中断: {e}"))?
}

/// 生成导入计划（干跑）：按日期分组建 destRoot/YYYY-MM-DD/ + 目标去重
/// （目标已存在同名同大小 = 已完成导入跳过；同名不同大小 = 防覆盖跳过）。
/// 不碰任何文件；跳过清单供前端预览前 20 条。
#[tauri::command]
#[specta::specta]
async fn plan_import(candidates: Vec<ImportCandidate>, dest_root: String) -> Result<ImportPlan, String> {
    let dest = PathBuf::from(dest_root);
    tauri::async_runtime::spawn_blocking(move || {
        let cands: Vec<photo_engine::import::ImportCandidate> = candidates
            .into_iter()
            .map(|c| photo_engine::import::ImportCandidate {
                path: PathBuf::from(c.path),
                date: c.date,
                size: c.size,
            })
            .collect();
        let plan = photo_engine::import::plan_import(&cands, &dest);
        Ok(ImportPlan {
            groups: plan
                .groups
                .into_iter()
                .map(|g| ImportGroup {
                    date_dir: g.date_dir,
                    files: g.files.into_iter().map(|p| p.to_string_lossy().to_string()).collect(),
                })
                .collect(),
            skipped: plan
                .skipped
                .into_iter()
                .map(|s| ImportSkipped {
                    path: s.path.to_string_lossy().to_string(),
                    reason: s.reason,
                })
                .collect(),
        })
    })
    .await
    .map_err(|e| format!("生成导入计划任务中断: {e}"))?
}

/// 执行导入：spawn_blocking 内逐文件委托 engine 复制/移动，逐文件 emit
/// import:progress，完成 emit import:done。返回 ImportResult（= done 负载）。
/// 完成后若目标根目录处于当前打开目录之内 → 触发重扫（scan_impl），
/// 新导入的照片立即出现在网格；否则不动当前目录。
#[tauri::command]
#[specta::specta]
async fn execute_import(
    app: AppHandle,
    plan: ImportPlan,
    dest_root: String,
    mode: ImportMode,
) -> Result<ImportResult, String> {
    if dest_root.trim().is_empty() {
        return Err("目标目录未指定".to_string());
    }
    // 执行阶段只需分组（跳过清单在计划期已决定）
    let engine_plan = photo_engine::import::ImportPlan {
        groups: plan
            .groups
            .iter()
            .map(|g| photo_engine::import::ImportGroup {
                date_dir: g.date_dir.clone(),
                files: g.files.iter().map(PathBuf::from).collect(),
            })
            .collect(),
        skipped: Vec::new(),
    };
    let skipped_count = plan.skipped.len() as u32;
    let dest = PathBuf::from(dest_root);
    let dest_for_exec = dest.clone();
    let engine_mode = match mode {
        ImportMode::Copy => photo_engine::import::ImportMode::Copy,
        ImportMode::Move => photo_engine::import::ImportMode::Move,
    };
    let app_work = app.clone();

    let (imported, failed) = tauri::async_runtime::spawn_blocking(move || -> Result<(u32, u32), String> {
        let mut imported = 0u32;
        let mut failed = 0u32;
        // 进度回调闭包持有自己的 AppHandle clone（外层闭包在回调结束后还要 emit done）
        let app_progress = app_work.clone();
        let results = photo_engine::import::execute_import(
            &engine_plan,
            &dest_for_exec,
            engine_mode,
            Some(Box::new(move |p| {
                let _ = app_progress.emit(
                    "import:progress",
                    ImportProgress {
                        done: p.done,
                        total: p.total,
                        current: p.current.to_string_lossy().to_string(),
                    },
                );
            })),
        );
        for (_, r) in results {
            if r.is_ok() {
                imported += 1;
            } else {
                failed += 1;
            }
        }
        let _ = app_work.emit(
            "import:done",
            ImportDone {
                imported,
                skipped: skipped_count,
                failed,
            },
        );
        Ok((imported, failed))
    })
    .await
    .map_err(|e| format!("导入任务中断: {e}"))??;

    // 完成后若目标根目录处于当前打开目录内 → 重扫（新导入照片立即可见）
    let current_dir = {
        let st = app.state::<Mutex<AppState>>();
        st.lock().expect("AppState 锁中毒").current_dir.clone()
    };
    if let Some(cur) = current_dir {
        if path_under(&cur, &dest) {
            let _ = scan_impl(app, cur).await;
        }
    }

    Ok(ImportResult {
        imported,
        skipped: skipped_count,
        failed,
    })
}

/// dest 是否位于 base 之内（规范化比较：防大小写/分隔符差异误判；
/// 目录不存在时回退原路径比较）
fn path_under(base: &Path, dest: &Path) -> bool {
    #[cfg(windows)]
    {
        // Windows 路径大小写不敏感：统一转小写再比较
        let norm = |p: &Path| {
            std::fs::canonicalize(p)
                .unwrap_or_else(|_| p.to_path_buf())
                .to_string_lossy()
                .to_lowercase()
        };
        dest_normalized_starts_with(norm(base), norm(dest))
    }
    #[cfg(not(windows))]
    {
        let norm = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        dest_normalized_starts_with(norm(base), norm(dest))
    }
}

/// 规范化后的 dest 字符串是否以 base 字符串开头（目录边界：base 末尾补分隔符防前缀误判）
fn dest_normalized_starts_with(base: String, dest: String) -> bool {
    if dest == base {
        return true;
    }
    dest.starts_with(&format!("{}\\", base)) || dest.starts_with(&format!("{}/", base))
}

// ============================================================================
// 扫描编排（照 state/scan.rs 移植）
// ============================================================================

/// 扫描 + DB 同步 + 缓存回填（阻塞，spawn_blocking 内执行）。
/// 只查缓存保证快返：EXIF 未命中项由后台 enrich 任务补齐。
/// `recursive` = true 时按配置递归扫全部子层（scanner::scan_directory_recursive），
/// 子目录照片的 rel 键（sub/bird.jpg）与 folder_db 单表键约定兼容（见 do_scan 注释）。
async fn do_scan(
    app: &AppHandle,
    dir: &Path,
    recursive: bool,
) -> Result<(Vec<CaptureMeta>, Option<FolderDb>), String> {
    // 克隆全局索引库句柄（spawn_blocking 内 replace_folder 同步用；None = 降级跳过同步）
    let global_db = {
        let state = app.state::<Mutex<AppState>>();
        state.lock().expect("AppState 锁中毒").global_db.clone()
    };
    let app_scan = app.clone();
    let dir_owned = dir.to_path_buf();
    tauri::async_runtime::spawn_blocking(move || {
        // scanner 进度回调要求 'static + Send，独立 clone AppHandle
        let app_progress = app_scan.clone();

        // 在照片目录中打开中心数据库（.pt/data.db）
        let folder_db = FolderDb::open_in_dir(&dir_owned).ok();

        // 扫描深度按配置选择：默认单层（scan_directory），include_subdirectories
        // 开启时递归（scan_directory_recursive，walkdir 不限深度，其余逻辑一致）
        let progress = Some(Box::new(move |pct: u32| {
            let _ = app_progress.emit(
                "scan:progress",
                ScanProgress {
                    stage: ScanStage::Scan,
                    done: pct,
                    total: 100,
                },
            );
        }) as Box<dyn Fn(u32) + Send>);
        let captures = if recursive {
            scanner::scan_directory_recursive(&dir_owned, &FilterCriteria::default(), progress)
        } else {
            scanner::scan_directory(&dir_owned, &FilterCriteria::default(), progress)
        }
        .map_err(|e| e.to_string())?;

        // 供 DB 三表同步的文件清单；顺带复用同一次 stat 构建 (size, mtime) 指纹表，
        // 供 exif 缓存行做内存指纹校验（避免 N 次 SQLite 点查询 + N 次 fs::metadata）。
        // rel_path 键约定：相对目录的正斜杠路径（strip_prefix + `\`→`/`）。递归扫描时
        // 子目录文件生成 `sub/bird.jpg` 这类含分隔符键——folder_db 的 recognition/
        // adjustments 表 rel_path 主键、upsert/get/sync 全部做 `\`→`/` 归一化比较，
        // 单表即可承载任意深度，无需按目录拆分（已验证，见 folder_db.rs FileEntry 注释）。
        let mut entries: Vec<FileEntry> = Vec::new();
        let mut fingerprints: HashMap<String, (u64, i64)> = HashMap::new();
        for f in captures.iter().flat_map(|c| c.source_files.iter()) {
            let Ok(rel) = f.path.strip_prefix(&dir_owned) else {
                continue;
            };
            let Some(m) = std::fs::metadata(&f.path).ok() else {
                continue;
            };
            let mtime_ns = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos() as i64)
                .unwrap_or(0);
            fingerprints.insert(f.path.to_string_lossy().to_string(), (m.len(), mtime_ns));
            entries.push(FileEntry {
                full_path: f.path.clone(),
                rel_path: rel.to_string_lossy().replace('\\', "/"),
                file_size: m.len(),
                mtime_ns,
                format: f.format.clone(),
            });
        }

        // 三表同步：删多余行、清识别/调整孤儿行（EXIF 提取不在此做，由后台 enrich 并发完成）。
        // 空目录也必须同步：外部直接删除全部文件后重扫，entries 为空 → 全表行判为多余删除，
        // 否则 .pt/data.db 里残留的识别/调整/缓存行永远不会被清理。
        if let Some(db) = &folder_db {
            let app_sync = app_scan.clone();
            let stats = db
                .sync_with_scan(&entries, &move |done, total| {
                    let _ = app_sync.emit(
                        "scan:progress",
                        ScanProgress {
                            stage: ScanStage::Scan,
                            done: done as u32,
                            total: total as u32,
                        },
                    );
                })
                .map_err(|e| e.to_string())?;
            tracing::info!(
                "DB 同步完成：清理缓存 {} / 识别 {} / 调整 {}",
                stats.cache_deleted,
                stats.recognition_deleted,
                stats.adjustments_deleted
            );
        }

        // 一次性载入 xmp/EXIF 缓存（2 条全表查询替代 2N 条点查询），
        // 键与 get_xmp/get_exif 一致：完整路径字符串（Windows 反斜杠）
        let xmp_rows: HashMap<String, photo_domain::XmpMetadata> = folder_db
            .as_ref()
            .and_then(|db| db.all_xmp_meta().ok())
            .unwrap_or_default();
        let keyword_rows: HashMap<String, Vec<String>> = folder_db
            .as_ref()
            .and_then(|db| db.all_keywords().ok())
            .unwrap_or_default();
        let exif_rows: HashMap<String, photo_engine::folder_db::ExifCacheRow> = folder_db
            .as_ref()
            .and_then(|db| db.all_exif().ok())
            .unwrap_or_default();
        let mut metas: Vec<CaptureMeta> = captures
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let mut meta = CaptureMeta::from_capture(c, i);
                let primary = &c.source_files[c.primary_index];
                let key = primary.path.to_string_lossy().to_string();
                if let Some(xmp) = xmp_rows.get(&key) {
                    meta.enrich_with_xmp(xmp);
                }
                if let Some(kws) = keyword_rows.get(&key) {
                    meta.enrich_with_keywords(kws);
                }
                // 只查缓存且校验指纹（与 get_exif 的 file_fingerprint 同源）：
                // 未命中/失效的 EXIF 由后台 enrich 任务并发提取
                if let Some(&(size, mtime_ns)) = fingerprints.get(&key)
                    && let Some(row) = exif_rows.get(&key)
                    && row.file_size == size as i64
                    && row.mtime_ns == mtime_ns
                {
                    meta.enrich_with_exif(&row.exif);
                }
                meta
            })
            .collect();

        // 用 folder_db 中已有的识别记录 enrich CaptureMeta（O(N) 哈希索引查表）
        if let Some(db) = &folder_db
            && let Ok(recs) = db.all_recognitions()
        {
            let rec_map: HashMap<&str, &photo_domain::Recognition> =
                recs.iter().map(|(p, r)| (p.as_str(), r)).collect();
            for meta in metas.iter_mut() {
                let primary_path = Path::new(&meta.primary_path);
                if let Ok(rel) = primary_path.strip_prefix(&dir_owned) {
                    let rel_str = rel.to_string_lossy().replace('\\', "/");
                    if let Some(rec) = rec_map.get(rel_str.as_str()) {
                        meta.enrich_with_recognition(rec);
                    }
                }
            }
        }

        // 全局鸟种索引同步：replace_folder 当前目录全部识别行（幂等；目录无识别
        // 记录即清空该文件夹行，保证外部删除/移动文件后索引不残留）
        if let Some(gdb) = &global_db {
            let folder_str = dir_owned.to_string_lossy().to_string();
            // rel → date_taken（EXIF 拍摄时间，统计首末见日期用）
            let date_by_rel: HashMap<String, Option<String>> = metas
                .iter()
                .filter_map(|m| {
                    let rel = Path::new(&m.primary_path).strip_prefix(&dir_owned).ok()?;
                    Some((rel.to_string_lossy().replace('\\', "/"), m.date_taken.clone()))
                })
                .collect();
            let mut rows: Vec<SpeciesRow> = Vec::new();
            if let Some(db) = &folder_db
                && let Ok(recs) = db.all_recognitions()
            {
                for (rel, rec) in recs {
                    if let Some(row) = species_row(
                        &folder_str,
                        &rel,
                        &rec,
                        date_by_rel.get(&rel).cloned().flatten(),
                    ) {
                        rows.push(row);
                    }
                }
            }
            if let Err(e) = gdb.replace_folder(&folder_str, &rows) {
                tracing::error!("全局鸟种索引同步失败 {folder_str}: {e}");
            }
        }

        tracing::info!("扫描完成：{} 共 {} 个 capture", dir_owned.display(), metas.len());
        Ok((metas, folder_db))
    })
    .await
    .map_err(|e| format!("扫描任务中断: {e}"))?
}

/// 后台 EXIF 增量提取 + 缩略图预生成（一次任务两产物，照 spawn_enrich_tasks 移植）。
/// 窗口化并发（4 张一组）替代 GPUI 的逐张 worker spawn；逐张 emit thumb:ready，
/// 完成后 emit capture:enriched（携带需重排的索引，前端据此重排）。
async fn enrich_and_pregen_thumbs(app: AppHandle, generation: u64) {
    // 快照：需要提取 EXIF 的 capture（扫描闭包只查缓存，未命中的字段为空）
    let (paths, folder_db, thumb_cache, thumbnail_size) = {
        let st = app.state::<Mutex<AppState>>();
        let st = st.lock().expect("AppState 锁中毒");
        let paths: Vec<(u32, PathBuf)> = st
            .captures
            .iter()
            .filter_map(|meta| {
                if meta.camera_make.is_some()
                    || meta.iso.is_some()
                    || meta.image_width.is_some()
                {
                    return None;
                }
                Some((meta.index as u32, PathBuf::from(&meta.primary_path)))
            })
            .collect();
        (
            paths,
            st.folder_db.clone(),
            st.thumb_cache.clone(),
            st.config.thumbnail_size,
        )
    };
    if paths.is_empty() {
        return;
    }
    let total = paths.len() as u32;
    let mut done: u32 = 0;
    let mut enriched: Vec<u32> = Vec::new();

    const CONCURRENCY: usize = 4;
    for chunk in paths.chunks(CONCURRENCY) {
        // 换目录后中止：旧索引对新 captures 无意义
        {
            let st = app.state::<Mutex<AppState>>();
            if st.lock().expect("AppState 锁中毒").scan_generation != generation {
                return;
            }
        }
        let mut handles = Vec::with_capacity(chunk.len());
        for &(idx, ref path) in chunk {
            let db = folder_db.clone();
            let cache = thumb_cache.clone();
            let path = path.clone();
            handles.push((
                idx,
                tauri::async_runtime::spawn_blocking(move || {
                    let ext = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_lowercase();
                    let Some(format) = ImageFormat::from_extension(&ext) else {
                        return (path, None, false);
                    };
                    // 经 SQLite 缓存提取并写回：下次扫描命中缓存，不再重复 LibRaw open
                    let exif = if let Some(db) = &db {
                        db.get_or_extract_exif(&path, &format).ok()
                    } else {
                        exif::extract_exif(&path, &format).ok()
                    };
                    // 顺带预生成缩略图缓存（RAW 内嵌提取 / JPG DCT 缩放；视频无缩略图跳过，
                    // file_size 用真实 stat 与浏览时 ptimg 的键一致）
                    let mut thumb_ok = false;
                    if let Some(cache) = &cache
                        && !format.is_other()
                    {
                        let source = SourceFile {
                            path: path.clone(),
                            format: format.clone(),
                            file_size: std::fs::metadata(&path).ok().map(|m| m.len()),
                        };
                        let result = if matches!(format, ImageFormat::Raw(_)) {
                            cache.get_or_generate_embedded(&source, thumbnail_size * 2, None)
                        } else {
                            cache.get_or_generate(&source, thumbnail_size * 2, None)
                        };
                        thumb_ok = result.is_ok();
                    }
                    (path, exif, thumb_ok)
                }),
            ));
        }
        for (idx, handle) in handles {
            let Ok((path, exif, thumb_ok)) = handle.await else {
                continue;
            };
            // 过期目录/列表：丢弃，防按新索引错绑 EXIF
            {
                let st = app.state::<Mutex<AppState>>();
                let mut st = st.lock().expect("AppState 锁中毒");
                if st.scan_generation != generation {
                    return;
                }
                if let Some(exif) = &exif
                    && let Some(meta) = st.captures.get_mut(idx as usize)
                    && meta.primary_path == path.to_string_lossy()
                {
                    meta.enrich_with_exif(exif);
                    enriched.push(idx);
                }
            }
            if thumb_ok {
                let _ = app.emit(
                    "thumb:ready",
                    ThumbReady {
                        path: path.to_string_lossy().to_string(),
                    },
                );
            }
            done += 1;
            // 节流：每 10 个或全部完成时推一次进度（逐张推会刷爆前端）
            if done == total || done.is_multiple_of(10) {
                let _ = app.emit(
                    "scan:progress",
                    ScanProgress {
                        stage: ScanStage::Exif,
                        done,
                        total,
                    },
                );
            }
        }
    }
    // 全部提取完成后通知重排（EXIF 日期/尺寸影响 DateTaken 排序与预览 fit 尺寸）
    let _ = app.emit("capture:enriched", CaptureEnriched { indices: enriched });
}

// ============================================================================
// ptimg:// 自定义协议（缩略图 / 预览母版 / 1:1 全尺寸三路流式 serve）
// ============================================================================

/// URL 形态（B 侧 ptimgUrl 封装）：
/// - Windows webview：`http://ptimg.localhost/<kind>/<urlencoded绝对路径>?v=<n>`
/// - Linux webkitgtk：`ptimg://<kind>/<urlencoded绝对路径>?v=<n>`（host 段为 kind）
/// `?v=` 为前端缓存失效参数，本 handler 忽略。
///
/// **异步协议**：tauri 同步版 uri scheme 回调在 WebView2 UI 线程执行，
/// 而缩略图生成/RAW 母版解码是耗时 IO（全尺寸解码 3-5s）——同步解码会
/// 卡死 UI（滚动/交互冻结，解码完才恢复）。这里只做 URL 解析（快），
/// 实际解码/IO 经 spawn_blocking 后台执行，完成后 responder 异步响应。
fn ptimg_handler(
    ctx: tauri::UriSchemeContext<'_, tauri::Wry>,
    request: tauri::http::Request<Vec<u8>>,
    responder: tauri::UriSchemeResponder,
) {
    use tauri::http::Response;
    let not_found = || {
        Response::builder()
            .status(404)
            .body(std::borrow::Cow::Borrowed(&[][..]))
            .unwrap()
    };

    let uri = request.uri();
    let host = uri.host().unwrap_or("");
    let raw_path = uri.path();
    // 两种形态择一：path 首段为 kind（Windows），或 host 为 kind（Linux）
    let (kind, encoded_path) = match raw_path.strip_prefix('/').and_then(|p| p.split_once('/')) {
        Some((k, rest)) if matches!(k, "thumb" | "master" | "full") => (k.to_string(), rest),
        _ if matches!(host, "thumb" | "master" | "full") => {
            (host.to_string(), raw_path.trim_start_matches('/'))
        }
        _ => return responder.respond(not_found()),
    };
    let Ok(decoded) = percent_encoding::percent_decode_str(encoded_path).decode_utf8() else {
        return responder.respond(not_found());
    };
    let path = PathBuf::from(decoded.as_ref());
    if !path.is_absolute() {
        return responder.respond(not_found());
    }

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let Some(format) = ImageFormat::from_extension(&ext) else {
        return responder.respond(not_found());
    };
    // 视频/OTHER 格式不生成缩略图（网格统一徽标）
    if format.is_other() {
        return responder.respond(not_found());
    }

    // 解码/IO 全部后台执行（spawn_blocking），响应经 responder 异步返回，
    // 不阻塞 WebView2 请求线程。ctx 是借用，先克隆 'static 的 AppHandle。
    let app_handle = ctx.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        // 克隆缓存句柄后即释放锁：生成是耗时 IO/解码，不能持锁阻塞 command
        let (cache, thumbnail_size) = {
            let state = app_handle.state::<Mutex<AppState>>();
            let st = state.lock().expect("AppState 锁中毒");
            (st.thumb_cache.clone(), st.config.thumbnail_size)
        };
        let file_size = std::fs::metadata(&path).ok().map(|m| m.len());
        let source = SourceFile {
            path: path.clone(),
            format: format.clone(),
            file_size,
        };

        // 调整参数渲染（尽力而为，ADR 0007）：master 预览且该图存在非中性调整参数时，
        // 经 engine 色调调整渲染输出（RAW 16-bit / 常规图 8-bit），供前端按 ?v= 刷新。
        // 参数持久化在 folder_db adjustments 表（键为相对路径），get/set_adjustments 即读写该表。
        const PREVIEW_LOAD_SIZE: u32 = 1600;
        if kind == "master"
            && let Some(params) = query_adjustments(&app_handle, &path)
            && !params.is_neutral()
        {
            match photo_engine::convert::render_adjusted(&source, &params, PREVIEW_LOAD_SIZE) {
                Ok(bytes) => {
                    return responder.respond(
                        Response::builder()
                            .status(200)
                            .header("Content-Type", "image/jpeg")
                            .header("Cache-Control", "no-cache")
                            .body(std::borrow::Cow::Owned(bytes))
                            .unwrap(),
                    );
                }
                Err(e) => {
                    // 渲染失败（解码/编码异常）不阻塞预览：记录后回退现有路径
                    tracing::warn!("调整渲染失败，回退原图: {e}");
                }
            }
        }

        // 常规图 webview 可直接解码的格式：master/full 直通原文件字节（零重编码，与 GPUI 一致）；
        // tiff/heif webview 解不了，走缓存重编码为 JPEG
        let passthrough_mime = match format {
            ImageFormat::Jpeg => Some("image/jpeg"),
            ImageFormat::Png => Some("image/png"),
            ImageFormat::WebP => Some("image/webp"),
            ImageFormat::Bmp => Some("image/bmp"),
            ImageFormat::Gif => Some("image/gif"),
            _ => None,
        };

        let served: Option<(Vec<u8>, &'static str)> = match kind.as_str() {
            "thumb" => {
                // 滚动缩略图：RAW 优先内嵌 JPEG（~50ms 提取，不触发全尺寸解码）。
                // 扫描时已用同键（std@size）预生成，命中直接读小文件；
                // 未命中时这里回退到常规 get_or_generate（会建母版）——由异步协议保证不卡 UI。
                match &format {
                    ImageFormat::Raw(_) => cache
                        .and_then(|c| {
                            c.get_or_generate_embedded(&source, thumbnail_size * 2, None)
                                .ok()
                        })
                        .map(|b| (b, "image/jpeg")),
                    _ => cache
                        .and_then(|c| c.get_or_generate(&source, thumbnail_size * 2, None).ok())
                        .map(|b| (b, "image/jpeg")),
                }
            }
            "master" => match (&format, passthrough_mime) {
                (ImageFormat::Raw(_), _) => cache
                    .and_then(|c| c.get_or_generate(&source, u32::MAX, None).ok())
                    .map(|b| (b, "image/jpeg")),
                (_, Some(mime)) => std::fs::read(&path).ok().map(|b| (b, mime)),
                _ => cache
                    .and_then(|c| c.get_or_generate(&source, u32::MAX, None).ok())
                    .map(|b| (b, "image/jpeg")),
            },
            "full" => match (&format, passthrough_mime) {
                (ImageFormat::Raw(_), _) => cache
                    .and_then(|c| c.get_or_generate_full(&source, None).ok())
                    .map(|b| (b, "image/jpeg")),
                (_, Some(mime)) => std::fs::read(&path).ok().map(|b| (b, mime)),
                _ => cache
                    .and_then(|c| c.get_or_generate(&source, u32::MAX, None).ok())
                    .map(|b| (b, "image/jpeg")),
            },
            _ => None,
        };

        let response = match served {
            Some((bytes, mime)) => Response::builder()
                .status(200)
                .header("Content-Type", mime)
                .header("Cache-Control", "no-cache")
                .body(std::borrow::Cow::Owned(bytes))
                .unwrap(),
            None => not_found(),
        };
        responder.respond(response);
    });
}

// ============================================================================
// 启动
// ============================================================================

/// 构建 specta Builder（commands + 事件负载类型），run 与导出 bin 共用
pub fn specta_builder() -> tauri_specta::Builder<tauri::Wry> {
    tauri_specta::Builder::<tauri::Wry>::new()
        .commands(tauri_specta::collect_commands![
            pick_directory,
            scan_directory,
            get_captures,
            set_rating,
            set_flag,
            set_color_label,
            set_keywords,
            list_favorites,
            add_favorite,
            remove_favorite,
            list_recent,
            list_subdirs,
            list_bird_species,
            get_histogram,
            get_clipping_mask,
            recognize_captures,
            cancel_recognition,
            batch_op_preview,
            batch_op_execute,
            undo_batch_operation,
            get_adjustments,
            set_adjustments,
            get_app_config,
            get_recognition,
            correct_bird,
            delete_captures,
            export_adjusted,
            list_system_fonts,
            set_app_config,
            get_species_stats,
            get_species_photos,
            get_correction_stats,
            get_frequent_species,
            list_import_drives,
            scan_import_source,
            plan_import,
            execute_import,
            batch_rename,
            export_captures,
        ])
        // 事件走 app.emit 明文通道（契约事件名含冒号，非 specta Event 命名），
        // 负载类型在此登记以便导出到 bindings.ts 供前端 listen 使用
        .typ::<ScanStage>()
        .typ::<ScanProgress>()
        .typ::<ScanDone>()
        .typ::<CaptureEnriched>()
        .typ::<ThumbReady>()
        .typ::<RecognizeProgress>()
        .typ::<RecognizeDone>()
        .typ::<BatchProgress>()
        .typ::<BatchDone>()
        .typ::<ExportProgress>()
        .typ::<ExportDone>()
        .typ::<ImportProgress>()
        .typ::<ImportDone>()
}

pub fn run() {
    // 配置：便携优先（exe 同级 PT.db / PT.toml），系统安装位置回落到 config_dir
    let config_path = photo_config::determine_config_path()
        .unwrap_or_else(|_| PathBuf::from("PT.db"));
    let config = photo_config::load_config(&config_path).unwrap_or_else(|e| {
        tracing::error!("加载配置失败，沿用默认配置: {e}");
        AppConfig::default()
    });

    let builder = specta_builder();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(builder.invoke_handler())
        .register_asynchronous_uri_scheme_protocol("ptimg", ptimg_handler)
        .manage(Mutex::new(AppState::new(config, config_path)))
        .setup(|app| {
            let app_handle = app.handle().clone();

            // 后台线程预热识别模型（DirectML 初始化 ~2-5s，首次识别不再等待）；
            // 失败仅记日志不阻塞启动。models/ 与 data/pica_ref.db 按便携约定在 exe 同级
            tauri::async_runtime::spawn_blocking(|| {
                let Some(exe_dir) = std::env::current_exe()
                    .ok()
                    .and_then(|p| p.parent().map(|d| d.to_path_buf()))
                else {
                    return;
                };
                let models_dir = exe_dir.join("models");
                let catalog_db = exe_dir.join("data").join("pica_ref.db");
                match photo_recognize::Recognizer::new(&models_dir, &catalog_db) {
                    Ok(_) => tracing::info!("识别模型预热完成"),
                    Err(e) => tracing::warn!("识别模型预热失败（不阻塞启动）: {e}"),
                }
            });

            // 恢复上次打开的目录（自动扫描）
            let last_dir = {
                let state = app_handle.state::<Mutex<AppState>>();
                state
                    .lock()
                    .expect("AppState 锁中毒")
                    .config
                    .last_directory
                    .clone()
            };
            if let Some(dir) = last_dir {
                let app_scan = app_handle.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = scan_impl(app_scan, PathBuf::from(dir)).await;
                });
            }

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // 退出时关闭 exiftool 长驻进程（避免残留子进程）
            if let tauri::RunEvent::Exit = event {
                photo_engine::exif::shutdown_provider();
                app.exit(0);
            }
        });
}

// ============================================================================
// specta 导出：生成 crates/photo-tauri/src/lib/bindings.ts（契约类型 + typed commands）
// ============================================================================

#[cfg(test)]
mod tests {
    /// 导出 TS 绑定到前端 lib 目录（集成时以本测试的生成物为准，覆盖手写 stub）
    #[test]
    fn export_bindings() {
        let builder = super::specta_builder();
        builder
            .export(
                specta_typescript::Typescript::default(),
                "../src/lib/bindings.ts",
            )
            .expect("导出 TS 绑定失败");
    }
}
