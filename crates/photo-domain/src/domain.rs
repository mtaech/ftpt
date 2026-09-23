use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 图片格式枚举
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ImageFormat {
    Jpeg,
    Png,
    Tiff,
    Heif,
    WebP,
    Bmp,
    Gif,
    Raw(String), // RAW 扩展名（如 "NEF"、"CR2"）
    Other,       // 除图片外的其他可查看格式（视频等）——网格统一显示徽标，不生成缩略图
}

impl std::fmt::Display for ImageFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Jpeg => write!(f, "JPEG"),
            Self::Png => write!(f, "PNG"),
            Self::Tiff => write!(f, "TIFF"),
            Self::Heif => write!(f, "HEIF"),
            Self::WebP => write!(f, "WebP"),
            Self::Bmp => write!(f, "BMP"),
            Self::Gif => write!(f, "GIF"),
            Self::Raw(r) => write!(f, "{}", r),
            Self::Other => write!(f, "OTHER"),
        }
    }
}

impl ImageFormat {
    /// 从文件扩展名推断格式
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_lowercase().as_str() {
            "jpg" | "jpeg" => Some(Self::Jpeg),
            "png" => Some(Self::Png),
            "tif" | "tiff" => Some(Self::Tiff),
            "heif" | "heic" => Some(Self::Heif),
            "webp" => Some(Self::WebP),
            "bmp" => Some(Self::Bmp),
            "gif" => Some(Self::Gif),
            "mp4" | "mov" | "m4v" | "avi" => Some(Self::Other),
            raw if Self::is_raw_extension(raw) => Some(Self::Raw(raw.to_uppercase())),
            _ => None,
        }
    }

    /// 是否为非图片格式（视频等，网格统一徽标、不生成缩略图）
    pub fn is_other(&self) -> bool {
        matches!(self, Self::Other)
    }

    /// 判断是否是可查看文件格式（图片 + RAW + 视频）
    pub fn is_viewable(ext: &str) -> bool {
        Self::from_extension(ext).is_some()
    }

    /// RAW 扩展名白名单
    pub fn is_raw_extension(ext: &str) -> bool {
        matches!(
            ext.to_lowercase().as_str(),
            "nef"
                | "nrw"
                | "cr2"
                | "cr3"
                | "arw"
                | "srf"
                | "sr2"
                | "dng"
                | "orf"
                | "raf"
                | "pef"
                | "rw2"
                | "raw"
                | "3fr"
                | "ari"
                | "bay"
                | "cap"
                | "dcr"
                | "drf"
                | "eip"
                | "erf"
                | "fff"
                | "iiq"
                | "k25"
                | "kdc"
                | "mdc"
                | "mef"
                | "mos"
                | "mrw"
                | "ndf"
                | "obm"
                | "ori"
                | "ptx"
                | "pxn"
                | "r3d"
                | "rwl"
                | "rwz"
                | "srw"
                | "x3f"
        )
    }
}

/// 组成一次拍摄的单个源文件
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceFile {
    pub path: PathBuf,
    pub format: ImageFormat,
    pub file_size: Option<u64>,
}

/// 一次快门产生的拍摄
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capture {
    pub base_name: String,
    pub source_files: Vec<SourceFile>,
    pub primary_index: usize,
}

/// 多主体识别摘要：一个识别主体的轻量展示信息（UI 用，与 Recognition.subjects 对应）。
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubjectSummary {
    /// 主体展示名（有中文名用中文名，否则学名）
    pub display_name: String,
    /// 主体置信度（0–100）
    pub confidence: Option<f32>,
}

/// 发送到前端的拍摄摘要（轻量，不含完整 SourceFile）
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureMeta {
    pub index: usize,
    pub base_name: String,
    pub primary_path: String,
    pub primary_format: String,
    pub file_size: Option<u64>,
    pub date_taken: Option<String>,
    pub extensions: Vec<String>,
    // --- EXIF 摘要字段（可延迟填充） ---
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub lens: Option<String>,
    pub exposure_time: Option<String>,
    pub f_number: Option<String>,
    pub iso: Option<u32>,
    pub focal_length: Option<String>,
    pub image_width: Option<u32>,
    pub image_height: Option<u32>,
    /// GPS 纬度（十进制度；南纬为负；EXIF 无 GPS 为 None）
    pub gps_lat: Option<f64>,
    /// GPS 经度（十进制度；西经为负；EXIF 无 GPS 为 None）
    pub gps_lon: Option<f64>,
    /// 对焦点（归一化坐标；相机未记录为 None）
    pub focus_point: Option<FocusPoint>,
    // --- 评分/色标/旗标字段（从文件夹数据库 xmp_meta 表填充） ---
    pub rating: Rating,
    pub color_label: ColorLabel,
    pub flag: Option<Flag>,
    // --- 关键词标签字段（从文件夹数据库 keywords 真相表填充，空 = 无标签） ---
    pub keywords: Vec<String>,
    // --- 识别摘要字段（从文件夹数据目录的 recognition 表填充，None = 未识别） ---
    pub taxon_name: Option<String>,
    pub taxon_confidence: Option<f32>,
    pub recognition_status: Option<RecognitionStatus>,
    pub taxon_bbox: Option<BBox>,
    /// 多主体识别摘要（与 Recognition.subjects 对应；主主体在前，旧数据退化为单主体）
    pub subjects: Vec<SubjectSummary>,
}

/// 从 Capture 构造 CaptureMeta。注意 index 固定为 0（历史调用约定，
/// 预览/EXIF 缓存会按 index 错绑），新代码请改用 [`CaptureMeta::from_capture`]。
impl From<&Capture> for CaptureMeta {
    fn from(c: &Capture) -> Self {
        Self::from_capture(c, 0)
    }
}

impl CaptureMeta {
    /// 显示用文件名：base_name（无扩展名）+ 主路径的**真实**扩展名。
    /// 不要用 primary_format 拼后缀——它是规范化格式名（如 JPEG→jpeg、TIFF→tiff），
    /// 会让 .jpg 显示成 .jpeg、.tif 显示成 .tiff（“拼接”假后缀）。
    pub fn display_name(&self) -> String {
        match std::path::Path::new(&self.primary_path)
            .extension()
            .and_then(|e| e.to_str())
        {
            Some(ext) if !ext.is_empty() => format!("{}.{}", self.base_name, ext),
            _ => self.base_name.clone(),
        }
    }

    /// 从 Capture 构造 CaptureMeta，显式指定 index（预览图/EXIF 回填/缩略图缓存的键）。
    /// source_files 为空或 primary_index 越界时退化到首个源文件/空主路径，绝不 panic。
    pub fn from_capture(c: &Capture, index: usize) -> Self {
        let primary = c
            .source_files
            .get(c.primary_index)
            .or_else(|| c.source_files.first());
        let ext_list: Vec<String> = c
            .source_files
            .iter()
            .map(|f| {
                f.path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|s| s.to_uppercase())
                    .unwrap_or_default()
            })
            .collect();
        Self {
            index,
            base_name: c.base_name.clone(),
            primary_path: primary
                .map(|f| f.path.to_string_lossy().to_string())
                .unwrap_or_default(),
            primary_format: primary.map(|f| f.format.to_string()).unwrap_or_default(),
            file_size: primary.and_then(|f| f.file_size),
            date_taken: None,
            extensions: ext_list,
            camera_make: None,
            camera_model: None,
            lens: None,
            exposure_time: None,
            f_number: None,
            iso: None,
            focal_length: None,
            image_width: None,
            image_height: None,
            gps_lat: None,
            gps_lon: None,
            focus_point: None,
            rating: Rating::None,
            color_label: ColorLabel::None,
            flag: None,
            keywords: Vec::new(),
            taxon_name: None,
            taxon_confidence: None,
            recognition_status: None,
            taxon_bbox: None,
            subjects: Vec::new(),
        }
    }

    /// 填充 EXIF 摘要字段（由调用方负责提取 ExifMetadata，本方法只做字段拷贝）
    pub fn enrich_with_exif(&mut self, exif: &ExifMetadata) {
        self.camera_make = exif.camera.make.clone();
        self.camera_model = exif.camera.model.clone();
        self.lens = exif.camera.lens.clone();
        self.exposure_time = exif.shooting.exposure_time.clone();
        self.f_number = exif.shooting.f_number.clone();
        self.iso = exif.shooting.iso;
        self.focal_length = exif.shooting.focal_length.clone();
        self.image_width = exif.image_width;
        self.image_height = exif.image_height;
        self.date_taken = exif.date_time_original.clone();
        self.focus_point = exif.focus_point;
        // GPS：EXIF 存 DMS 元组（南纬/西经符号已由提取层施加在度分量上），
        // 此处统一转十进制度供前端展示/地图链接
        self.gps_lat = exif.gps.latitude.map(|(d, m, s)| dms_to_decimal(d, m, s));
        self.gps_lon = exif.gps.longitude.map(|(d, m, s)| dms_to_decimal(d, m, s));
        // 如果 EXIF 没有文件大小，回退到 fs::metadata
        if self.file_size.is_none() {
            self.file_size = std::fs::metadata(std::path::Path::new(&self.primary_path))
                .ok()
                .map(|m| m.len());
        }
    }

    /// 填充评分/颜色标签/旗标（由调用方负责从文件夹数据库读取 XmpMetadata）
    pub fn enrich_with_xmp(&mut self, xmp: &XmpMetadata) {
        self.rating = xmp.rating();
        self.color_label = xmp.color_label();
        self.flag = xmp.flag();
    }

    /// 填充关键词标签（由调用方负责从文件夹数据库 keywords 真相表读取）
    pub fn enrich_with_keywords(&mut self, keywords: &[String]) {
        self.keywords = keywords.to_vec();
    }

    /// 填充识别摘要字段（由调用方负责从文件夹数据目录读取 Recognition）
    pub fn enrich_with_recognition(&mut self, recognition: &Recognition) {
        self.taxon_name = recognition.taxon.as_ref().map(|t| t.cn_name.clone());
        self.taxon_confidence = recognition.confidence;
        self.recognition_status = Some(recognition.status);
        self.taxon_bbox = recognition.bbox;
        // 多主体：有 subjects 用 subjects；旧数据（空）以顶层字段退化为单主体
        self.subjects = if recognition.subjects.is_empty() {
            recognition.taxon.as_ref().map(|t| {
                vec![SubjectSummary {
                    display_name: t.display_name().to_string(),
                    confidence: recognition.confidence,
                }]
            }).unwrap_or_default()
        } else {
            recognition.subjects.iter().map(|s| SubjectSummary {
                display_name: s.taxon.as_ref().map(|t| t.display_name().to_string())
                    .unwrap_or_else(|| "<未识别>".to_string()),
                confidence: s.confidence,
            }).collect()
        };
    }
}

/// 评分
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Rating {
    None = 0,
    One = 1,
    Two = 2,
    Three = 3,
    Four = 4,
    Five = 5,
}

/// 颜色标签
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColorLabel {
    None,
    Red,
    Yellow,
    Green,
    Blue,
    Purple,
}

/// Pick/Reject 旗标
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Flag {
    Pick,
    Reject,
}

/// 筛选条件
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterCriteria {
    /// 按文件类型过滤
    pub format_filter: Option<ImageFormat>,
    /// 按物种中文名过滤（多选，空 = 不过滤）
    #[serde(default)]
    pub taxon_names: Vec<String>,
    /// 按日期范围过滤
    pub date_from: Option<chrono::NaiveDate>,
    pub date_to: Option<chrono::NaiveDate>,
    /// 按评分过滤
    pub min_rating: Option<Rating>,
    /// 按颜色标签过滤
    pub color_label: Option<ColorLabel>,
    /// 按旗标过滤（Pick/Reject/None）
    pub flag_filter: Option<Flag>,
    /// 如果为 true，只显示没有标记旗标的照片
    pub unflagged_filter: bool,
    /// 按识别状态过滤（含"未识别"= 无识别记录）
    #[serde(default)]
    pub recognition_filter: RecognitionFilter,
}

impl FilterCriteria {
    /// 是否有激活的筛选条件（任一字段非默认）。
    /// 无筛选时操作集 = 全部文件，批量文件操作应拒绝执行（防误操作）。
    pub fn has_active_filter(&self) -> bool {
        self.format_filter.is_some()
            || !self.taxon_names.is_empty()
            || self.date_from.is_some()
            || self.date_to.is_some()
            || self.min_rating.is_some()
            || self.color_label.is_some()
            || self.flag_filter.is_some()
            || self.unflagged_filter
            || self.recognition_filter != RecognitionFilter::All
    }
}

/// 排序方式
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortBy {
    FileName,
    DateTaken,
    FileSize,
    Rating,
    Modified,
}


/// 排序方向
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortDirection {
    Ascending,
    Descending,
}

/// 批量文件操作类型（作用于当前筛选结果，ADR 0006）
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BatchOpType {
    /// 复制到目标目录
    Copy,
    /// 删除（回收站）
    Delete,
    /// 移动到目标目录
    Move,
}

impl BatchOpType {
    /// 是否需要目标目录（删除操作不需要）
    pub fn needs_target_dir(&self) -> bool {
        !matches!(self, Self::Delete)
    }

    /// 执行的动作标签
    pub fn action_label(&self) -> &'static str {
        match self {
            Self::Copy => "复制",
            Self::Delete => "删除",
            Self::Move => "移动",
        }
    }
}

impl std::fmt::Display for BatchOpType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.action_label())
    }
}

// ============================================================================
// Exif 数据类型（纯结构体，提取/读写机械在 photo-engine 中）
// ============================================================================

/// 相机制造商信息
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraInfo {
    pub make: Option<String>,
    pub model: Option<String>,
    pub lens: Option<String>,
}

/// 拍摄参数
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShootingParams {
    pub exposure_time: Option<String>,
    pub f_number: Option<String>,
    pub iso: Option<u32>,
    pub focal_length: Option<String>,
    pub exposure_compensation: Option<String>,
    pub white_balance: Option<String>,
}

/// GPS 信息
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GpsInfo {
    pub latitude: Option<(f64, f64, f64)>,
    pub longitude: Option<(f64, f64, f64)>,
    pub altitude: Option<f64>,
}

/// 度分秒 → 十进制度。
/// 南纬/西经的负号约定由 EXIF 提取层施加在度分量上（deg 为负）；分/秒恒为正，
/// 因此符号施加在整体结果上：-116°23'29" → -(116 + 23/60 + 29/3600) ≈ -116.3914
/// （直接 deg + min/60 + sec/3600 会把 -116°23' 算成 -115.6°，西半球严重偏移）。
pub fn dms_to_decimal(deg: f64, min: f64, sec: f64) -> f64 {
    let magnitude = deg.abs() + min / 60.0 + sec / 3600.0;
    // 用 is_sign_negative 而非 deg < 0.0：0°xx′S/W 的度分量是 -0.0，
    // 比较运算会把它当成非负而翻到另一个半球
    if deg.is_sign_negative() {
        -magnitude
    } else {
        magnitude
    }
}

/// 对焦点形状（EXIF SubjectArea 三形态 / RAW makernotes 映射）
/// 纯枚举不加 rename（对齐 Rating/ColorLabel 约定）：变体按原名序列化。
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum FocusShape {
    /// 仅中心点（SubjectArea 1 值 / Fuji FocusPixel / Nikon·Panasonic AFPointSelected）
    Point,
    /// 圆形区域（SubjectArea 2 值：圆心 + 直径）
    Circle,
    /// 矩形区域（SubjectArea 3 值：左上角 + 宽高）
    Rectangle,
}

/// 对焦点：归一化坐标（0–1，相对 orientation 修正后的显示方向），形状决定渲染形态。
///
/// 字段语义随形状变化：`Point` 时 (x, y) 为点位置、width/height 为 0；`Circle` 时
/// (x, y) 为圆心、width = height = 直径；`Rectangle` 时 (x, y) 为左上角、width/height 为宽高。
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FocusPoint {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub shape: FocusShape,
}

impl FocusPoint {
    /// 构造点形状焦点并夹紧到 0–1
    pub fn point(x: f32, y: f32) -> Self {
        Self {
            x: x.clamp(0.0, 1.0),
            y: y.clamp(0.0, 1.0),
            width: 0.0,
            height: 0.0,
            shape: FocusShape::Point,
        }
    }

    /// 构造圆形区域焦点（width/height 为直径）并夹紧到 0–1
    pub fn circle(x: f32, y: f32, d: f32) -> Self {
        Self {
            x: x.clamp(0.0, 1.0),
            y: y.clamp(0.0, 1.0),
            width: d.clamp(0.0, 1.0),
            height: d.clamp(0.0, 1.0),
            shape: FocusShape::Circle,
        }
    }

    /// 构造矩形区域焦点（(x, y) 为左上角）并夹紧到 0–1
    pub fn rectangle(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self {
            x: x.clamp(0.0, 1.0),
            y: y.clamp(0.0, 1.0),
            width: w.clamp(0.0, 1.0),
            height: h.clamp(0.0, 1.0),
            shape: FocusShape::Rectangle,
        }
    }
}

/// 完整的 EXIF 元数据
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExifMetadata {
    pub camera: CameraInfo,
    pub shooting: ShootingParams,
    pub gps: GpsInfo,
    pub date_time_original: Option<String>,
    pub image_width: Option<u32>,
    pub image_height: Option<u32>,
    pub file_size: Option<u64>,
    pub color_space: Option<String>,
    pub orientation: Option<u16>,
    /// 对焦点（EXIF SubjectArea / RAW makernotes；相机未记录为 None）
    pub focus_point: Option<FocusPoint>,
}

// ============================================================================
// 识别数据类型（纯结构体，推理机械在 photo-recognize 中，持久化机械在 photo-engine 中）
// ============================================================================

/// 检测框：归一化坐标 [x1, y1, x2, y2]（0–1，相对图像宽高）
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BBox {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
}

impl BBox {
    /// 构造并夹紧到 0–1 范围
    pub fn new(x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        // NaN/Inf 归零：f32::clamp 对 NaN 返回 NaN，会一路流进推理与持久化
        let clamp01 = |v: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
        Self {
            x1: clamp01(x1),
            y1: clamp01(y1),
            x2: clamp01(x2),
            y2: clamp01(y2),
        }
    }

    /// 解析数据库文本格式 "x1,y1,x2,y2"
    pub fn parse(s: &str) -> Option<Self> {
        // 严格四段：filter_map 会把畸形串（多段/含垃圾段）悄悄拼成 4 个值；
        // NaN/Inf 也必须拒绝（clamp 对 NaN 返回 NaN，会污染推理与持久化）
        let parts: Vec<&str> = s.split(',').collect();
        if parts.len() != 4 {
            return None;
        }
        let mut v = [0.0f32; 4];
        for (i, p) in parts.iter().enumerate() {
            let x: f32 = p.trim().parse().ok()?;
            if !x.is_finite() {
                return None;
            }
            v[i] = x;
        }
        Some(Self::new(v[0], v[1], v[2], v[3]))
    }

    /// 序列化为数据库文本格式 "x1,y1,x2,y2"
    pub fn to_db_string(&self) -> String {
        format!("{},{},{},{}", self.x1, self.y1, self.x2, self.y2)
    }
}

/// 调整参数（参数化非破坏，ADR 0007）：per-capture，全零 = 无调整（短路现有渲染路径）。
/// 曝光/对比度/饱和度均为像素值变换（裁切已移除，见 ADR 0007 修订）。
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdjustParams {
    /// 曝光（EV，±2.0，步进 0.05；+1.0 EV = 曝光量翻倍）
    pub exposure: f32,
    /// 对比度（-100 ~ +100，0 中性）
    pub contrast: i32,
    /// 饱和度（-100 ~ +100，0 中性；-100 = 去饱和）
    pub saturation: i32,
}

impl Default for AdjustParams {
    fn default() -> Self {
        Self {
            exposure: 0.0,
            contrast: 0,
            saturation: 0,
        }
    }
}

impl AdjustParams {
    /// 是否为无调整（全零参数）——渲染路径短路到现有 8-bit 链路
    pub fn is_neutral(&self) -> bool {
        self.exposure == 0.0 && self.contrast == 0 && self.saturation == 0
    }
}

/// 识别状态（三态；无识别记录 = 未识别，不占枚举值）
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecognitionStatus {
    /// 检测、分类、名录映射全部成功
    Confirmed,
    /// 管线中途失败（分类异常 / 名录映射失败 / 源图不可用），需人工复核
    NeedsReview,
    /// 检测阶段未发现鸟
    Unrecognized,
}

impl RecognitionStatus {
    /// 数据库存储文本
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::NeedsReview => "needs_review",
            Self::Unrecognized => "unrecognized",
        }
    }

    /// 从数据库文本解析
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "confirmed" => Some(Self::Confirmed),
            "needs_review" => Some(Self::NeedsReview),
            "unrecognized" => Some(Self::Unrecognized),
            _ => None,
        }
    }
}

/// 识别失败阶段
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecognitionFailureStage {
    None,
    Detection,
    Classification,
    Mapping,
    Assets,
}

impl RecognitionFailureStage {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Detection => "detection",
            Self::Classification => "classification",
            Self::Mapping => "mapping",
            Self::Assets => "assets",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "none" => Some(Self::None),
            "detection" => Some(Self::Detection),
            "classification" => Some(Self::Classification),
            "mapping" => Some(Self::Mapping),
            "assets" => Some(Self::Assets),
            _ => None,
        }
    }

    /// 面向用户的失败原因说明
    pub fn user_message(&self) -> &'static str {
        match self {
            Self::None => "",
            Self::Detection => "检测异常",
            Self::Classification => "分类异常",
            Self::Mapping => "名录映射失败",
            Self::Assets => "源图不可用",
        }
    }
}

/// 物种匹配：识别器给出的一个物种结论（见 docs/adr/0008）。
///
/// BioCLIP 直接以标签的学名为答案（名录里查得到才有 taxon_id）。所以
/// **模型的结论不依赖本地名录是否有这个物种** —— 名录只提供中文名与主键。
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaxonMatch {
    /// 名录库 animal_info 主键（名录里没有这个物种时为 None）
    pub taxon_id: Option<i64>,
    /// 中文名（空串 = 三个中文名源都没收这个名字）
    pub cn_name: String,
    /// 学名（拉丁名，双名）
    pub latin_name: String,
    /// 中文名来自哪一级（种 / 属 / 科 / 无）
    pub cn_level: CnLevel,
    /// 七级分类 [界, 门, 纲, 目, 科, 属, 种]，空串 = 该级缺
    pub ranks: Vec<String>,
}

impl TaxonMatch {
    /// 展示名：有中文名就用中文名，没有就退到学名。
    /// 全量标签空间里 26% 的类群三个中文名源都收不到，这类只能显示学名。
    pub fn display_name(&self) -> &str {
        if self.cn_name.is_empty() { &self.latin_name } else { &self.cn_name }
    }
}

/// 中文名的级别：识别器给的是学名，中文名按 种 → 属 → 科 三级回落
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CnLevel {
    /// 种级中文名
    Species,
    /// 回落到属级中文名
    Genus,
    /// 回落到科级中文名
    Family,
    /// 三级都没有（只能显示学名）
    Missing,
}

impl CnLevel {
    /// 数据库存储文本
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Species => "species",
            Self::Genus => "genus",
            Self::Family => "family",
            Self::Missing => "missing",
        }
    }

    /// 从数据库文本解析
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "species" => Some(Self::Species),
            "genus" => Some(Self::Genus),
            "family" => Some(Self::Family),
            "missing" => Some(Self::Missing),
            _ => None,
        }
    }
}

/// Top-N 候选（含未映射项：taxon 为 None 表示该候选没落到任何物种上）
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaxonCandidate {
    /// 识别器原始类别号（BioCLIP 标签下标）
    pub class_index: u32,
    /// 置信度（0–100）
    pub confidence: f32,
    /// 该候选对应的物种（未命中为 None）
    pub taxon: Option<TaxonMatch>,
}

/// 一次识别的完整结果（不含路径；路径是持久化层的键）
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recognition {
    pub status: RecognitionStatus,
    /// Top-1 物种匹配（未检出 / 分类失败时为 None）
    pub taxon: Option<TaxonMatch>,
    /// Top-1 原始类别号（诊断用）
    pub class_index: Option<u32>,
    /// Top-1 置信度（0–100）
    pub confidence: Option<f32>,
    /// 检测框（检测失败为 None）
    pub bbox: Option<BBox>,
    /// Top-5 候选（含 Top-1 自身除外的备选；分类失败为空）
    pub candidates: Vec<TaxonCandidate>,
    pub failure_stage: RecognitionFailureStage,
    /// ISO8601 时间戳
    pub recognized_at: String,
    /// 多主体：每个检测到的主体的识别结论（主主体在 0 位）。
    /// 单主体 = 1 个元素；顶层 taxon/confidence/bbox/candidates 恒 = 主主体（subjects[0]），
    /// 兼容既有展示；旧数据（无 subjects 列）subjects 为空，读侧以顶层字段兜底。
    pub subjects: Vec<SubjectRecognition>,
}

/// 一张照片里的一个识别主体（多主体数组元素）。
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubjectRecognition {
    /// 主体序号（检测顺序；0 = 主主体）
    pub index: u32,
    /// 主体检测框（归一化 0-1，相对原图）
    pub bbox: BBox,
    /// 该主体的物种结论
    pub taxon: Option<TaxonMatch>,
    /// 该主体 Top-1 原始类别号（诊断用）
    pub class_index: Option<u32>,
    /// 该主体 Top-1 置信度（0–100）
    pub confidence: Option<f32>,
    /// 该主体 Top-5 候选
    pub candidates: Vec<TaxonCandidate>,
    /// 该主体失败阶段（None = 有结论）
    pub failure: RecognitionFailureStage,
}

impl Recognition {
    /// 手动框选补充主体：把框选识别出的新主体追加进 subjects（漏检补充语义）。
    ///
    /// - 旧数据（subjects 空）先把顶层字段物化为「主主体」subject 0，再追加——
    ///   这样旧的识别结论不被覆盖，手动框选的成为补充主体
    /// - 新主体 index = 现有最大 index + 1；顶层字段保持 = 主主体（subjects[0]）
    /// - 状态重新聚合：任一主体有结论（failure == None）→ Confirmed；全失败 → NeedsReview
    /// - recognized_at 刷新为当前时间
    pub fn append_subject(mut self, mut subject: SubjectRecognition) -> Recognition {
        // 旧数据兜底：subjects 空时顶层字段就是唯一主体，先物化为 subject 0
        if self.subjects.is_empty() {
            if let Some(t) = self.taxon.clone() {
                self.subjects.push(SubjectRecognition {
                    index: 0,
                    bbox: self.bbox.unwrap_or(BBox::new(0.0, 0.0, 1.0, 1.0)),
                    taxon: Some(t),
                    class_index: self.class_index,
                    confidence: self.confidence,
                    candidates: self.candidates.clone(),
                    failure: RecognitionFailureStage::None,
                });
            }
        }
        let next_index = self
            .subjects
            .iter()
            .map(|s| s.index)
            .max()
            .map(|m| m + 1)
            .unwrap_or(0);
        subject.index = next_index;
        self.subjects.push(subject);
        // 顶层字段恒 = 主主体（subjects[0]）：旧记录无结论且 subjects 为空时，subjects[0]
        // 就是刚追加的这个主体——不同步的话顶层 taxon 永远是 None，而读侧（信息栏/网格）
        // 只看顶层，于是状态已 Confirmed 却显示「无记录」（用户报过「框选识别出来了却不显示」）。
        if let Some(primary) = self.subjects.first().cloned() {
            self.taxon = primary.taxon;
            self.class_index = primary.class_index;
            self.confidence = primary.confidence;
            self.bbox = Some(primary.bbox);
            self.candidates = primary.candidates;
        }
        // 状态重新聚合：任一主体有结论 → Confirmed；否则 NeedsReview（首个失败阶段）
        if self
            .subjects
            .iter()
            .any(|s| s.failure == RecognitionFailureStage::None)
        {
            self.status = RecognitionStatus::Confirmed;
            self.failure_stage = RecognitionFailureStage::None;
        } else {
            self.status = RecognitionStatus::NeedsReview;
            self.failure_stage = self
                .subjects
                .iter()
                .find(|s| s.failure != RecognitionFailureStage::None)
                .map(|s| s.failure)
                .unwrap_or(RecognitionFailureStage::Detection);
        }
        self.recognized_at = chrono::Utc::now().to_rfc3339();
        self
    }
}

/// 识别状态筛选条件
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecognitionFilter {
    /// 全部（不筛选）
    #[default]
    All,
    Confirmed,
    NeedsReview,
    Unrecognized,
    /// 未识别（无识别记录）
    NotRecognized,
}

// ============================================================================
// XMP 数据类型（纯结构体 + 枚举转换，读写机械在 photo-engine 中）
// ============================================================================

/// XMP 中存储的 PT 元数据
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct XmpMetadata {
    /// 评分 (0-5)
    pub rating: u8,
    /// 颜色标签 (空字符串表示无)
    pub color_label: String,
    /// 旗标: "" | "pick" | "reject"
    pub flag: String,
}

impl XmpMetadata {
    pub fn rating(&self) -> Rating {
        match self.rating {
            1 => Rating::One,
            2 => Rating::Two,
            3 => Rating::Three,
            4 => Rating::Four,
            5 => Rating::Five,
            _ => Rating::None,
        }
    }

    pub fn set_rating(&mut self, rating: Rating) {
        self.rating = rating as u8;
    }

    pub fn color_label(&self) -> ColorLabel {
        match self.color_label.as_str() {
            "red" => ColorLabel::Red,
            "yellow" => ColorLabel::Yellow,
            "green" => ColorLabel::Green,
            "blue" => ColorLabel::Blue,
            "purple" => ColorLabel::Purple,
            _ => ColorLabel::None,
        }
    }

    pub fn set_color_label(&mut self, label: ColorLabel) {
        self.color_label = match label {
            ColorLabel::Red => "red".into(),
            ColorLabel::Yellow => "yellow".into(),
            ColorLabel::Green => "green".into(),
            ColorLabel::Blue => "blue".into(),
            ColorLabel::Purple => "purple".into(),
            ColorLabel::None => "".into(),
        };
    }

    pub fn flag(&self) -> Option<Flag> {
        match self.flag.as_str() {
            "pick" => Some(Flag::Pick),
            "reject" => Some(Flag::Reject),
            _ => None,
        }
    }

    pub fn set_flag(&mut self, flag: Option<Flag>) {
        self.flag = match flag {
            Some(Flag::Pick) => "pick".into(),
            Some(Flag::Reject) => "reject".into(),
            None => "".into(),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_from_jpg() {
        assert_eq!(ImageFormat::from_extension("jpg"), Some(ImageFormat::Jpeg));
        assert_eq!(ImageFormat::from_extension("JPEG"), Some(ImageFormat::Jpeg));
    }

    #[test]
    fn test_format_from_raw() {
        assert_eq!(
            ImageFormat::from_extension("NEF"),
            Some(ImageFormat::Raw("NEF".to_string()))
        );
    }

    #[test]
    fn test_raw_extension_whitelist() {
        assert!(ImageFormat::is_raw_extension("NEF"));
        assert!(ImageFormat::is_raw_extension("cr2"));
        assert!(ImageFormat::is_raw_extension("DNG"));
        assert!(!ImageFormat::is_raw_extension("jpg"));
        assert!(!ImageFormat::is_raw_extension("mp4"));
    }


    #[test]
    fn test_format_from_png() {
        assert_eq!(ImageFormat::from_extension("png"), Some(ImageFormat::Png));
    }

    #[test]
    fn test_format_from_tiff() {
        assert_eq!(ImageFormat::from_extension("tif"), Some(ImageFormat::Tiff));
        assert_eq!(ImageFormat::from_extension("TIFF"), Some(ImageFormat::Tiff));
    }

    #[test]
    fn test_format_gif() {
        assert_eq!(ImageFormat::from_extension("gif"), Some(ImageFormat::Gif));
    }

    #[test]
    fn test_invalid_extension() {
        assert_eq!(ImageFormat::from_extension("txt"), None);
        assert_eq!(ImageFormat::from_extension("exe"), None);
    }

    #[test]
    fn test_video_extension() {
        assert_eq!(ImageFormat::from_extension("mp4"), Some(ImageFormat::Other));
        assert_eq!(ImageFormat::from_extension("MOV"), Some(ImageFormat::Other));
        assert!(ImageFormat::Other.is_other());
        assert!(!ImageFormat::Jpeg.is_other());
    }

    #[test]
    fn test_is_viewable() {
        assert!(ImageFormat::is_viewable("jpg"));
        assert!(ImageFormat::is_viewable("nef"));
        assert!(ImageFormat::is_viewable("mp4"), "视频也应可查看（显示徽标）");
        assert!(!ImageFormat::is_viewable("txt"));
    }

    #[test]
    fn test_enrich_with_xmp_fills_fields() {
        let mut xmp = XmpMetadata::default();
        xmp.set_rating(Rating::Three);
        xmp.set_color_label(ColorLabel::Green);
        xmp.set_flag(Some(Flag::Pick));

        let mut cm = CaptureMeta {
            index: 0,
            base_name: "IMG_0001".into(),
            primary_path: "/tmp/IMG_0001.jpg".into(),
            primary_format: "JPEG".into(),
            file_size: None,
            date_taken: None,
            extensions: vec![],
            camera_make: None,
            camera_model: None,
            lens: None,
            exposure_time: None,
            f_number: None,
            iso: None,
            focal_length: None,
            image_width: None,
            image_height: None,
            gps_lat: None,
            gps_lon: None,
            focus_point: None,
            rating: Rating::None,
            color_label: ColorLabel::None,
            flag: None,
            keywords: Vec::new(),
            taxon_name: None,
            taxon_confidence: None,
            recognition_status: None,
            taxon_bbox: None,
            subjects: vec![],
        };
        cm.enrich_with_xmp(&xmp);
        assert_eq!(cm.rating, Rating::Three);
        assert_eq!(cm.color_label, ColorLabel::Green);
        assert_eq!(cm.flag, Some(Flag::Pick));
    }

    #[test]
    fn test_xmp_metadata_default() {
        let m = XmpMetadata::default();
        assert_eq!(m.rating, 0);
        assert_eq!(m.color_label, "");
        assert_eq!(m.flag, "");
    }

    #[test]
    fn test_xmp_rating_conversion() {
        let mut m = XmpMetadata::default();
        m.set_rating(Rating::Four);
        assert_eq!(m.rating, 4);
        assert_eq!(m.rating(), Rating::Four);
    }

    #[test]
    fn test_xmp_color_label_conversion() {
        let mut m = XmpMetadata::default();
        m.set_color_label(ColorLabel::Red);
        assert_eq!(m.color_label, "red");
        assert_eq!(m.color_label(), ColorLabel::Red);
    }

    #[test]
    fn test_xmp_flag_conversion() {
        let mut m = XmpMetadata::default();
        m.set_flag(Some(Flag::Pick));
        assert_eq!(m.flag, "pick");
        assert_eq!(m.flag(), Some(Flag::Pick));

        m.set_flag(None);
        assert_eq!(m.flag, "");
        assert_eq!(m.flag(), None);
    }

    #[test]
    fn test_exif_default_metadata_is_empty() {
        let meta = ExifMetadata::default();
        assert!(meta.camera.make.is_none());
        assert!(meta.camera.model.is_none());
        assert!(meta.shooting.iso.is_none());
        assert!(meta.image_width.is_none());
        assert!(meta.image_height.is_none());
        assert!(meta.file_size.is_none());
        assert!(meta.focus_point.is_none());
    }

    #[test]
    fn test_focus_point_constructors_clamp_and_shape() {
        // 点：width/height 为 0，坐标夹紧
        let p = FocusPoint::point(1.5, -0.2);
        assert_eq!(p.shape, FocusShape::Point);
        assert_eq!(p.x, 1.0);
        assert_eq!(p.y, 0.0);
        assert_eq!(p.width, 0.0);
        assert_eq!(p.height, 0.0);
        // 圆：width = height = 直径
        let c = FocusPoint::circle(0.3, 0.4, 0.2);
        assert_eq!(c.shape, FocusShape::Circle);
        assert_eq!(c.width, 0.2);
        assert_eq!(c.height, 0.2);
        // 矩形：左上角 + 宽高
        let r = FocusPoint::rectangle(0.1, 0.1, 0.5, 0.3);
        assert_eq!(r.shape, FocusShape::Rectangle);
        assert_eq!(r.x, 0.1);
        assert_eq!(r.width, 0.5);
        assert_eq!(r.height, 0.3);
    }

    #[test]
    fn test_focus_point_serde_roundtrip_camel_case() {
        let r = FocusPoint::rectangle(0.1, 0.2, 0.5, 0.3);
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("\"shape\":\"Rectangle\""), "got: {json}");
        let back: FocusPoint = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn test_enrich_with_exif_copies_focus_point() {
        let mk = || CaptureMeta::from_capture(
            &Capture {
                base_name: "G".into(),
                source_files: vec![],
                primary_index: 0,
            },
            0,
        );
        let mut exif = ExifMetadata::default();
        exif.image_width = Some(4000);
        exif.image_height = Some(3000);
        exif.focus_point = Some(FocusPoint::point(0.5, 0.5));
        let mut cm = mk();
        cm.enrich_with_exif(&exif);
        assert_eq!(cm.focus_point, exif.focus_point);
        // 无焦点记录时保持 None
        exif.focus_point = None;
        let mut cm2 = mk();
        cm2.enrich_with_exif(&exif);
        assert!(cm2.focus_point.is_none());
    }

    #[test]
    fn test_dms_to_decimal_conversion() {
        // 东经 116°23'45" ≈ 116.39583；南纬/西经负号由提取层施加在度分量上
        let d = dms_to_decimal(116.0, 23.0, 45.0);
        assert!((d - 116.39583).abs() < 1e-4, "实际 {d}");
        // 符号只来自度分量（模拟 S/W 已施加负号）
        let s = dms_to_decimal(-30.0, 30.0, 0.0);
        assert!((s - (-30.5)).abs() < 1e-9);
        // 整度/整分边界
        assert_eq!(dms_to_decimal(10.0, 0.0, 0.0), 10.0);
        assert!((dms_to_decimal(0.0, 30.0, 0.0) - 0.5).abs() < 1e-9);
        // -0.0（0°xx′S/W）必须判负：deg < 0.0 会漏判
        assert!(dms_to_decimal(-0.0, 30.0, 0.0) < 0.0);
    }

    #[test]
    fn test_enrich_with_exif_fills_gps_decimal() {
        let mut exif = ExifMetadata::default();
        // 北纬 39°54'26"、西经 116°23'29"（西经符号在度分量上，模拟 exif.rs 施加）
        exif.gps.latitude = Some((39.0, 54.0, 26.0));
        exif.gps.longitude = Some((-116.0, 23.0, 29.0));
        let mut cm = CaptureMeta::from_capture(
            &Capture {
                base_name: "G".into(),
                source_files: vec![],
                primary_index: 0,
            },
            0,
        );
        assert!(cm.gps_lat.is_none(), "默认无 GPS");
        cm.enrich_with_exif(&exif);
        let lat = cm.gps_lat.expect("回填后应有纬度");
        let lon = cm.gps_lon.expect("回填后应有经度");
        assert!((lat - (39.0 + 54.0 / 60.0 + 26.0 / 3600.0)).abs() < 1e-9);
        assert!(lon < 0.0, "西经应为负: {lon}");
        assert!((lon - (-(116.0 + 23.0 / 60.0 + 29.0 / 3600.0))).abs() < 1e-9);
    }

    #[test]
    fn test_bbox_db_string_roundtrip() {
        let b = BBox::new(0.1, 0.2, 0.8, 0.9);
        let parsed = BBox::parse(&b.to_db_string()).unwrap();
        assert_eq!(parsed, b);
    }

    #[test]
    fn test_bbox_new_clamps_to_unit_range() {
        let b = BBox::new(-0.5, 0.2, 1.3, 0.9);
        assert_eq!(b, BBox { x1: 0.0, y1: 0.2, x2: 1.0, y2: 0.9 });
    }

    #[test]
    fn test_bbox_parse_rejects_malformed() {
        assert!(BBox::parse("0.1,0.2,0.3").is_none());
        assert!(BBox::parse("not,a,box,here").is_none());
        assert!(BBox::parse("").is_none());
        // 多段/垃圾段不能被 filter_map 悄悄拼成合法四值
        assert!(BBox::parse("0.1,0.2,0.3,0.4,garbage").is_none());
        // NaN 必须拒绝（clamp 对 NaN 返回 NaN，会污染推理与持久化）
        assert!(BBox::parse("0.1,0.2,0.3,NaN").is_none());
        assert!(BBox::new(f32::NAN, 0.2, 0.3, 0.4).x1 == 0.0);
    }

    #[test]
    fn test_recognition_status_str_roundtrip() {
        for s in [
            RecognitionStatus::Confirmed,
            RecognitionStatus::NeedsReview,
            RecognitionStatus::Unrecognized,
        ] {
            assert_eq!(RecognitionStatus::from_str(s.as_str()), Some(s));
        }
        assert_eq!(RecognitionStatus::from_str("pending"), None);
    }

    // ── 手动框选补充：Recognition::append_subject ──

    fn subject(index: u32, latin: &str, cn: &str, failure: RecognitionFailureStage) -> SubjectRecognition {
        SubjectRecognition {
            index,
            bbox: BBox::new(0.1, 0.1, 0.4, 0.4),
            taxon: Some(TaxonMatch {
                taxon_id: None,
                cn_name: cn.to_string(),
                latin_name: latin.to_string(),
                cn_level: CnLevel::Species,
                ranks: vec![],
            }),
            class_index: Some(index),
            confidence: Some(80.0),
            candidates: vec![],
            failure,
        }
    }

    fn base_rec() -> Recognition {
        Recognition {
            status: RecognitionStatus::Confirmed,
            taxon: Some(TaxonMatch {
                taxon_id: None,
                cn_name: "长耳鸮".to_string(),
                latin_name: "Asio otus".to_string(),
                cn_level: CnLevel::Species,
                ranks: vec![],
            }),
            class_index: Some(0),
            confidence: Some(70.0),
            bbox: Some(BBox::new(0.0, 0.0, 0.5, 0.5)),
            candidates: vec![],
            failure_stage: RecognitionFailureStage::None,
            recognized_at: "2026-09-22T10:00:00Z".to_string(),
            subjects: vec![subject(0, "Asio otus", "长耳鸮", RecognitionFailureStage::None)],
        }
    }

    #[test]
    fn test_append_subject_appends_with_next_index() {
        let rec = base_rec().append_subject(subject(99, "Entoloma", "粉褶蕈属", RecognitionFailureStage::None));
        assert_eq!(rec.subjects.len(), 2);
        // 新主体 index = 现有最大 + 1（忽略入参里的 99，重新编号）
        assert_eq!(rec.subjects[1].index, 1);
        assert_eq!(rec.subjects[1].taxon.as_ref().unwrap().latin_name, "Entoloma");
        // 顶层仍 = 主主体（subjects[0]），不因追加而变
        assert_eq!(rec.taxon.as_ref().unwrap().latin_name, "Asio otus");
        assert_eq!(rec.status, RecognitionStatus::Confirmed);
    }

    #[test]
    fn test_append_subject_failure_moves_status_to_needs_review() {
        // 两个主体都有结论 → 追加一个失败主体 → 整张仍 Confirmed（任一有结论）
        let rec = base_rec().append_subject(subject(1, "x", "y", RecognitionFailureStage::Classification));
        assert_eq!(rec.status, RecognitionStatus::Confirmed);
        assert_eq!(rec.subjects[1].failure, RecognitionFailureStage::Classification);
        // 全部失败 → NeedsReview（首个失败阶段）
        let all_fail = Recognition {
            status: RecognitionStatus::NeedsReview,
            taxon: None,
            class_index: None,
            confidence: None,
            bbox: None,
            candidates: vec![],
            failure_stage: RecognitionFailureStage::Classification,
            recognized_at: "2026-09-22T10:00:00Z".to_string(),
            subjects: vec![subject(0, "a", "b", RecognitionFailureStage::Classification)],
        };
        let rec = all_fail.append_subject(subject(1, "c", "d", RecognitionFailureStage::Mapping));
        assert_eq!(rec.status, RecognitionStatus::NeedsReview);
        assert_eq!(rec.failure_stage, RecognitionFailureStage::Classification);
    }

    /// 回归（用户报「框选识别出来了，结果栏却显示『无记录』」）：照片此前是 Unrecognized
    /// （subjects 空、顶层无结论），框选补到一个有结论的主体 → 该主体成为 subjects[0]，
    /// **顶层字段必须跟着 subjects[0]**（文档承诺「顶层恒 = 主主体」）。不同步的话
    /// 展示层只读顶层 taxon 就会显示成「无记录」，而状态却是 Confirmed。
    #[test]
    fn test_append_subject_into_conclusionless_record_syncs_top_level() {
        let legacy = Recognition {
            status: RecognitionStatus::Unrecognized,
            taxon: None,
            class_index: None,
            confidence: None,
            bbox: None,
            candidates: vec![],
            failure_stage: RecognitionFailureStage::Detection,
            recognized_at: "2026-09-23T10:00:00Z".to_string(),
            subjects: vec![],
        };
        let rec = legacy.append_subject(subject(
            0,
            "Padda oryzivora",
            "爪哇禾雀",
            RecognitionFailureStage::None,
        ));
        // 无结论的旧记录不该凭空多出一个「空主体」
        assert_eq!(rec.subjects.len(), 1);
        assert_eq!(rec.status, RecognitionStatus::Confirmed);
        assert_eq!(
            rec.taxon.as_ref().map(|t| t.latin_name.as_str()),
            Some("Padda oryzivora"),
            "顶层必须同步为主主体（subjects[0]），否则展示层显示空"
        );
        assert_eq!(rec.confidence, Some(80.0));
        assert_eq!(rec.class_index, Some(0));
    }

    #[test]
    fn test_append_subject_legacy_empty_subjects_materializes_top_level() {
        // 旧数据：subjects 空但顶层有结论 → 先物化为 subject 0，新主体为 subject 1
        let legacy = Recognition {
            status: RecognitionStatus::Confirmed,
            taxon: Some(TaxonMatch {
                taxon_id: None,
                cn_name: "白鹭".to_string(),
                latin_name: "Egretta garzetta".to_string(),
                cn_level: CnLevel::Species,
                ranks: vec![],
            }),
            class_index: Some(3),
            confidence: Some(90.0),
            bbox: None,
            candidates: vec![],
            failure_stage: RecognitionFailureStage::None,
            recognized_at: "2026-08-01T10:00:00Z".to_string(),
            subjects: vec![],
        };
        let rec = legacy.append_subject(subject(0, "Entoloma", "粉褶蕈属", RecognitionFailureStage::None));
        assert_eq!(rec.subjects.len(), 2);
        assert_eq!(rec.subjects[0].taxon.as_ref().unwrap().latin_name, "Egretta garzetta");
        assert_eq!(rec.subjects[0].index, 0);
        assert_eq!(rec.subjects[1].index, 1);
        // 顶层保持 = 物化后的主主体（旧结论）
        assert_eq!(rec.taxon.as_ref().unwrap().latin_name, "Egretta garzetta");
        assert_eq!(rec.status, RecognitionStatus::Confirmed);
    }

    #[test]
    fn test_failure_stage_str_roundtrip_and_messages() {
        for s in [
            RecognitionFailureStage::None,
            RecognitionFailureStage::Detection,
            RecognitionFailureStage::Classification,
            RecognitionFailureStage::Mapping,
            RecognitionFailureStage::Assets,
        ] {
            assert_eq!(RecognitionFailureStage::from_str(s.as_str()), Some(s));
        }
        assert_eq!(RecognitionFailureStage::Mapping.user_message(), "名录映射失败");
    }

    #[test]
    fn test_display_name_uses_real_extension() {
        // .jpg 文件：规范化格式名是 JPEG，必须显示 .jpg 而非 .jpeg
        let cm = CaptureMeta::from(&Capture {
            base_name: "IMG_1234".into(),
            source_files: vec![SourceFile {
                path: std::path::PathBuf::from("/photos/IMG_1234.jpg"),
                format: ImageFormat::Jpeg,
                file_size: Some(1024),
            }],
            primary_index: 0,
        });
        assert_eq!(cm.display_name(), "IMG_1234.jpg");

        // .tif 文件：必须显示 .tif 而非 .tiff
        let cm = CaptureMeta::from(&Capture {
            base_name: "SCAN_01".into(),
            source_files: vec![SourceFile {
                path: std::path::PathBuf::from("/photos/SCAN_01.tif"),
                format: ImageFormat::Tiff,
                file_size: Some(1024),
            }],
            primary_index: 0,
        });
        assert_eq!(cm.display_name(), "SCAN_01.tif");

        // RAW：真实扩展名（.nef）优先
        let cm = CaptureMeta::from(&Capture {
            base_name: "DSC_0001".into(),
            source_files: vec![SourceFile {
                path: std::path::PathBuf::from("/photos/DSC_0001.nef"),
                format: ImageFormat::Raw("NEF".into()),
                file_size: Some(1024),
            }],
            primary_index: 0,
        });
        assert_eq!(cm.display_name(), "DSC_0001.nef");

        // 无扩展名：回退 base_name
        let cm = CaptureMeta::from(&Capture {
            base_name: "README".into(),
            source_files: vec![SourceFile {
                path: std::path::PathBuf::from("/photos/README"),
                format: ImageFormat::Other,
                file_size: Some(1024),
            }],
            primary_index: 0,
        });
        assert_eq!(cm.display_name(), "README");
    }

    #[test]
    fn test_enrich_with_recognition_confirmed() {
        let rec = Recognition {
            status: RecognitionStatus::Confirmed,
            taxon: Some(TaxonMatch {
                taxon_id: Some(42),
                cn_name: "大山雀".into(),
                latin_name: "Parus major".into(),
                cn_level: CnLevel::Species,
                ranks: vec![],
            }),
            class_index: Some(1066),
            confidence: Some(85.3),
            bbox: Some(BBox::new(0.1, 0.2, 0.8, 0.9)),
            candidates: vec![],
            failure_stage: RecognitionFailureStage::None,
            recognized_at: "2026-07-28T12:00:00Z".into(),
            subjects: vec![],
        };
        let mut cm = CaptureMeta::from(&Capture {
            base_name: "DSC_0001".into(),
            source_files: vec![SourceFile {
                path: std::path::PathBuf::from("/photos/DSC_0001.jpg"),
                format: ImageFormat::Jpeg,
                file_size: Some(1024),
            }],
            primary_index: 0,
        });
        cm.enrich_with_recognition(&rec);
        assert_eq!(cm.taxon_name.as_deref(), Some("大山雀"));
        assert_eq!(cm.taxon_confidence, Some(85.3));
        assert_eq!(cm.recognition_status, Some(RecognitionStatus::Confirmed));
        assert!(cm.taxon_bbox.is_some());
    }

    #[test]
    fn test_subjects_serde_roundtrip() {
        let subject = SubjectRecognition {
            index: 0,
            bbox: BBox::new(0.1, 0.2, 0.4, 0.5),
            taxon: Some(TaxonMatch {
                taxon_id: Some(1),
                cn_name: "乌鸫".into(),
                latin_name: "Turdus merula".into(),
                cn_level: CnLevel::Species,
                ranks: vec![],
            }),
            class_index: Some(7),
            confidence: Some(88.0),
            candidates: vec![],
            failure: RecognitionFailureStage::None,
        };
        let json = serde_json::to_string(&subject).unwrap();
        let back: SubjectRecognition = serde_json::from_str(&json).unwrap();
        assert_eq!(back, subject);
    }

    #[test]
    fn test_enrich_with_recognition_multi_subject_builds_summary() {
        let rec = Recognition {
            status: RecognitionStatus::Confirmed,
            taxon: Some(TaxonMatch {
                taxon_id: Some(1),
                cn_name: "乌鸫".into(),
                latin_name: "Turdus merula".into(),
                cn_level: CnLevel::Species,
                ranks: vec![],
            }),
            class_index: Some(7),
            confidence: Some(88.0),
            bbox: Some(BBox::new(0.1, 0.2, 0.3, 0.4)),
            candidates: vec![],
            failure_stage: RecognitionFailureStage::None,
            recognized_at: "2026-07-28T12:00:00Z".into(),
            subjects: vec![
                SubjectRecognition {
                    index: 0,
                    bbox: BBox::new(0.1, 0.2, 0.3, 0.4),
                    taxon: Some(TaxonMatch {
                        taxon_id: Some(1),
                        cn_name: "乌鸫".into(),
                        latin_name: "Turdus merula".into(),
                        cn_level: CnLevel::Species,
                        ranks: vec![],
                    }),
                    class_index: Some(7),
                    confidence: Some(88.0),
                    candidates: vec![],
                    failure: RecognitionFailureStage::None,
                },
                SubjectRecognition {
                    index: 1,
                    bbox: BBox::new(0.6, 0.2, 0.9, 0.6),
                    taxon: Some(TaxonMatch {
                        taxon_id: Some(2),
                        cn_name: "大山雀".into(),
                        latin_name: "Parus major".into(),
                        cn_level: CnLevel::Species,
                        ranks: vec![],
                    }),
                    class_index: Some(8),
                    confidence: Some(77.0),
                    candidates: vec![],
                    failure: RecognitionFailureStage::None,
                },
            ],
        };
        let mut cm = CaptureMeta::from(&Capture {
            base_name: "DSC_0001".into(),
            source_files: vec![SourceFile {
                path: std::path::PathBuf::from("/photos/DSC_0001.jpg"),
                format: ImageFormat::Jpeg,
                file_size: Some(1024),
            }],
            primary_index: 0,
        });
        cm.enrich_with_recognition(&rec);
        assert_eq!(cm.subjects.len(), 2);
        assert_eq!(cm.subjects[0].display_name, "乌鸫");
        assert_eq!(cm.subjects[1].display_name, "大山雀");
        assert_eq!(cm.subjects[1].confidence, Some(77.0));
        // 主主体字段仍是 subjects[0]（乌鸫）
        assert_eq!(cm.taxon_name.as_deref(), Some("乌鸫"));
    }

    #[test]
    fn test_enrich_with_recognition_unrecognized_has_no_bird_fields() {
        let rec = Recognition {
            status: RecognitionStatus::Unrecognized,
            taxon: None,
            class_index: None,
            confidence: None,
            bbox: None,
            candidates: vec![],
            failure_stage: RecognitionFailureStage::Detection,
            recognized_at: "2026-07-28T12:00:00Z".into(),
            subjects: vec![],
        };
        let mut cm = CaptureMeta::from(&Capture {
            base_name: "DSC_0002".into(),
            source_files: vec![SourceFile {
                path: std::path::PathBuf::from("/photos/DSC_0002.jpg"),
                format: ImageFormat::Jpeg,
                file_size: Some(1024),
            }],
            primary_index: 0,
        });
        cm.enrich_with_recognition(&rec);
        assert_eq!(cm.taxon_name, None);
        assert_eq!(cm.recognition_status, Some(RecognitionStatus::Unrecognized));
        assert_eq!(cm.taxon_bbox, None);
    }

    #[test]
    fn test_sort_by_modified_serde_roundtrip() {
        let json = serde_json::to_string(&SortBy::Modified).expect("序列化失败");
        assert_eq!(json, "\"Modified\"");
        assert_eq!(
            serde_json::from_str::<SortBy>("\"Modified\"").expect("反序列化失败"),
            SortBy::Modified
        );
    }

}
