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
    /// 缩略图缓存（.pt/thumbs，跟随目录）
    thumb_cache: Option<ThumbnailCache>,
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
}

impl AppState {
    fn new(config: AppConfig, config_path: PathBuf) -> Self {
        Self {
            current_dir: None,
            captures: Vec::new(),
            folder_db: None,
            thumb_cache: None,
            config,
            config_path,
            scan_generation: 0,
            recognition_cancel: Arc::new(AtomicBool::new(false)),
            recognition_running: false,
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
    let generation = {
        let state = app.state::<Mutex<AppState>>();
        let mut st = state.lock().expect("AppState 锁中毒");
        st.scan_generation += 1;
        st.scan_generation
    };
    match do_scan(&app, &dir).await {
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
    let (cancel, folder_db, thumb_cache, thread_count, dir, generation) = {
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
            dir,
            st.scan_generation,
        )
    };

    // 构建目标列表：path（主文件绝对路径）→ (rel_path, Capture)；找不到对应
    // CaptureMeta 的路径跳过（文件可能已被移动/删除）
    let targets: Vec<(String, String, photo_domain::Capture)> = {
        let st = app.state::<Mutex<AppState>>();
        let st = st.lock().expect("AppState 锁中毒");
        paths
            .iter()
            .filter_map(|path| {
                let meta = st.captures.iter().find(|m| m.primary_path == *path)?;
                let rel = rel_path_of(&dir, path)?;
                Some((path.clone(), rel, build_capture_from_meta(meta)))
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
            let cache = &thumb_cache;
            let generation = &generation_work;
            let targets = &targets;

            let handles: Vec<_> = targets
                .chunks(chunk_size)
                .map(|chunk| {
                    s.spawn(move || {
                        // 每线程懒加载自己的 Recognizer（DirectML 初始化 2-5s 一次性）
                        let mut recognizer = build_recognizer();
                        for (path, rel, cap) in chunk {
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
                            let rec_result = match &mut recognizer {
                                Ok(rec) => rec
                                    .recognize_with_thumbnail(cap, thumb_bytes.as_deref(), None)
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
    let (dir, _generation) = {
        let st = app.state::<Mutex<AppState>>();
        let st = st.lock().expect("AppState 锁中毒");
        (
            st.current_dir
                .clone()
                .ok_or_else(|| "尚未打开目录".to_string())?,
            st.scan_generation,
        )
    };
    let target_dir = options.target_dir.map(PathBuf::from);
    let formats = formats_to_set(&options.formats);
    let sync_siblings = options.sync_siblings;
    let app_work = app.clone();

    let result = tauri::async_runtime::spawn_blocking(move || -> Result<BatchOpResult, String> {
        // 1. 重扫源目录，取完整 Capture（批量操作需要 source_files 全列表）
        let caps = scanner::scan_directory(&dir, &FilterCriteria::default(), None)
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
        // 3. 执行；进度回调逐文件 emit（闭包借用 paths 与 app 的 clone，仅在线程闭包体内引用）
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
        let failed = failures.len() as u32;
        let _ = app_work.emit("batch:done", BatchDone { success, failed });
        Ok(BatchOpResult {
            success,
            failed,
            failures,
        })
    })
    .await
    .map_err(|e| format!("批量操作任务中断: {e}"))??;
    Ok(result)
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

    // 同步内存 CaptureMeta
    {
        let mut st = state.lock().expect("AppState 锁中毒");
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
    let (dir, db) = {
        let mut st = state.lock().expect("AppState 锁中毒");
        st.scan_generation += 1;
        (
            st.current_dir.clone().ok_or("尚未打开目录")?,
            st.folder_db.clone(),
        )
    };
    let target: HashSet<String> = paths.into_iter().collect();
    let app_work = app.clone();
    let dir_str = dir.to_string_lossy().to_string();

    let result = tauri::async_runtime::spawn_blocking(move || -> Result<u32, String> {
        // 1. 重扫源目录取完整 Capture（同 batch_op_execute）
        let caps = scanner::scan_directory(&dir, &FilterCriteria::default(), None)
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
    };
    save_config(&st);
    Ok(())
}

// ============================================================================
// 扫描编排（照 state/scan.rs 移植）
// ============================================================================

/// 扫描 + DB 同步 + 缓存回填（阻塞，spawn_blocking 内执行）。
/// 只查缓存保证快返：EXIF 未命中项由后台 enrich 任务补齐。
async fn do_scan(app: &AppHandle, dir: &Path) -> Result<(Vec<CaptureMeta>, Option<FolderDb>), String> {
    let app_scan = app.clone();
    let dir_owned = dir.to_path_buf();
    tauri::async_runtime::spawn_blocking(move || {
        // scanner 进度回调要求 'static + Send，独立 clone AppHandle
        let app_progress = app_scan.clone();

        // 在照片目录中打开中心数据库（.pt/data.db）
        let folder_db = FolderDb::open_in_dir(&dir_owned).ok();

        let captures = scanner::scan_directory(
            &dir_owned,
            &FilterCriteria::default(),
            Some(Box::new(move |pct| {
                let _ = app_progress.emit(
                    "scan:progress",
                    ScanProgress {
                        stage: ScanStage::Scan,
                        done: pct,
                        total: 100,
                    },
                );
            })),
        )
        .map_err(|e| e.to_string())?;

        // 供 DB 三表同步的文件清单；顺带复用同一次 stat 构建 (size, mtime) 指纹表，
        // 供 exif 缓存行做内存指纹校验（避免 N 次 SQLite 点查询 + N 次 fs::metadata）
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
fn ptimg_handler(
    ctx: tauri::UriSchemeContext<'_, tauri::Wry>,
    request: tauri::http::Request<Vec<u8>>,
) -> tauri::http::Response<std::borrow::Cow<'static, [u8]>> {
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
        _ => return not_found(),
    };
    let Ok(decoded) = percent_encoding::percent_decode_str(encoded_path).decode_utf8() else {
        return not_found();
    };
    let path = PathBuf::from(decoded.as_ref());
    if !path.is_absolute() {
        return not_found();
    }

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let Some(format) = ImageFormat::from_extension(&ext) else {
        return not_found();
    };
    // 视频/OTHER 格式不生成缩略图（网格统一徽标）
    if format.is_other() {
        return not_found();
    }

    // 克隆缓存句柄后即释放锁：生成是耗时 IO/解码，不能持锁阻塞 command
    let (cache, thumbnail_size) = {
        let state = ctx.app_handle().state::<Mutex<AppState>>();
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
        && let Some(params) = query_adjustments(ctx.app_handle(), &path)
        && !params.is_neutral()
    {
        match photo_engine::convert::render_adjusted(&source, &params, PREVIEW_LOAD_SIZE) {
            Ok(bytes) => {
                return Response::builder()
                    .status(200)
                    .header("Content-Type", "image/jpeg")
                    .header("Cache-Control", "no-cache")
                    .body(std::borrow::Cow::Owned(bytes))
                    .unwrap();
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
        "thumb" => cache
            .and_then(|c| c.get_or_generate(&source, thumbnail_size * 2, None).ok())
            .map(|b| (b, "image/jpeg")),
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

    match served {
        Some((bytes, mime)) => Response::builder()
            .status(200)
            .header("Content-Type", mime)
            .header("Cache-Control", "no-cache")
            .body(std::borrow::Cow::Owned(bytes))
            .unwrap(),
        None => not_found(),
    }
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
            list_favorites,
            add_favorite,
            remove_favorite,
            list_recent,
            list_bird_species,
            recognize_captures,
            cancel_recognition,
            batch_op_preview,
            batch_op_execute,
            get_adjustments,
            set_adjustments,
            get_app_config,
            get_recognition,
            correct_bird,
            delete_captures,
            export_adjusted,
            list_system_fonts,
            set_app_config,
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
        .register_uri_scheme_protocol("ptimg", ptimg_handler)
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
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
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
