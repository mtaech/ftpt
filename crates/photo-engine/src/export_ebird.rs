//! eBird/观鸟记录 CSV 导出：按文件夹汇总已确认鸟种 → CSV。
//!
//! 数据源 = 文件夹级 `.pt/data.db`（folder_db）：
//! - `recognition` 表：Confirmed 全量计入；NeedsReview 计入但备注列标注「待确认」；
//!   Unrecognized 与无鸟种名（bird None）的行不计入
//! - `exif_cache` 表：拍摄日期（date_time_original → YYYY-MM-DD），缺失/解析失败
//!   回退文件 mtime（照抄 import.rs 的 fallback 逻辑）；GPS 度分秒 → 十进制度
//!   （photo_domain::dms_to_decimal，南纬/西经为负）
//!
//! 聚合键 = (中文名, 学名, 日期)：同日多张合并计数（count 累加），跨日拆分。
//! 导出范围 = 单个文件夹（不读 global_db 跨文件夹索引；任务契约）。
//! CSV 输出 = UTF-8 BOM + RFC 4180 转义（逗号/引号/换行），手写转义不引入新依赖。

use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::path::Path;

use chrono::Datelike;
use photo_domain::{dms_to_decimal, ExifMetadata, RecognitionStatus};
use thiserror::Error;

use crate::folder_db::{FolderDb, FolderDbError};

/// eBird 导出错误
#[derive(Error, Debug)]
pub enum EbirdError {
    #[error("数据库错误: {0}")]
    Db(#[from] FolderDbError),
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
}

/// eBird CSV 单行（按（鸟种, 日期）聚合后的观测记录）。
///
/// `species_sci` 来自识别行的 BirdMatch::latin_name——注意 folder_db 持久化只存
/// 中文名（bird_name），重读时学名为空串；需学名的消费方可自行经名录库补全。
#[derive(Debug, Clone, PartialEq)]
pub struct EbirdRow {
    pub species_cn: String,
    pub species_sci: String,
    pub count: u32,
    pub date: String,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    /// 待确认标记：组内含 NeedsReview 记录时为 Some("待确认")，纯 Confirmed 为 None
    pub note: Option<String>,
}

/// 聚合中间行（build_rows 读取 DB 后展开，aggregate 做分组合并）
struct FileRow {
    /// 正斜杠相对路径（仅用于组内首张确定性排序，不进入 CSV）
    rel_path: String,
    cn: String,
    sci: String,
    date: String,
    lat: Option<f64>,
    lon: Option<f64>,
    /// 该行是否为 NeedsReview（组内任一待确认 → 备注列标注）
    needs_review: bool,
}

/// 按文件夹汇总 eBird 记录：读 recognition + exif_cache，按（鸟种, 日期）聚合。
///
/// - Confirmed / NeedsReview 且 bird 匹配成功（有鸟种名）的行参与聚合；
///   NeedsReview 组内计入并标注「待确认」
/// - 日期 = EXIF date_time_original（YYYY-MM-DD），回退文件 mtime（照 import.rs）
/// - GPS = EXIF 度分秒 → 十进制；组内取首个有 GPS 的坐标（按 rel_path 升序确定性）
/// - 输出按（日期, 中文名, 学名）升序（日期字典序即时间序），保证确定性
pub fn build_rows(dir: &Path) -> Result<Vec<EbirdRow>, EbirdError> {
    let db = FolderDb::open_in_dir(dir)?;
    let recs = db.all_recognitions()?;
    let exif_map = db.all_exif()?;
    // exif_cache 键为写入时的完整路径串（Windows 反斜杠/大小写），归一化后查表
    let exif_norm: HashMap<String, ExifMetadata> = exif_map
        .into_iter()
        .map(|(k, v)| (norm_key(&k), v.exif))
        .collect();

    let mut file_rows = Vec::new();
    for (rel_path, rec) in recs {
        // 三态：只取 Confirmed + NeedsReview；Unrecognized 不计
        let needs_review = match rec.status {
            RecognitionStatus::Confirmed => false,
            RecognitionStatus::NeedsReview => true,
            RecognitionStatus::Unrecognized => continue,
        };
        // 无鸟种名（映射失败/未检出）无法构成记录行，跳过
        let Some(bird) = rec.bird.as_ref() else { continue };

        let full_path = dir.join(&rel_path);
        let exif = exif_norm.get(&norm_key(&full_path.to_string_lossy()));
        let (lat, lon) = row_gps(exif);
        file_rows.push(FileRow {
            rel_path,
            cn: bird.cn_name.clone(),
            sci: bird.latin_name.clone(),
            date: row_date(&full_path, exif),
            lat,
            lon,
            needs_review,
        });
    }
    Ok(aggregate(file_rows))
}

/// 按（日期, 中文名, 学名）聚合：同键合并 count；组内任一 NeedsReview 标备注；
/// GPS 取组内首个有坐标的（独立取 lat/lon，任一有即填充）。
fn aggregate(rows: Vec<FileRow>) -> Vec<EbirdRow> {
    // 先按 rel_path 升序排序：DB 读取顺序未定义，排序保证「首个有 GPS」确定性
    let mut rows = rows;
    rows.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));

    // BTreeMap 键序 = （日期, 中文名, 学名）升序 = 输出顺序（日期字典序即时间序）
    #[derive(Default)]
    struct Agg {
        count: u32,
        lat: Option<f64>,
        lon: Option<f64>,
        needs_review: bool,
    }
    let mut map: BTreeMap<(String, String, String), Agg> = BTreeMap::new();
    for r in rows {
        let key = (r.date, r.cn, r.sci);
        let agg = map.entry(key).or_default();
        agg.count += 1;
        if agg.lat.is_none() {
            agg.lat = r.lat;
        }
        if agg.lon.is_none() {
            agg.lon = r.lon;
        }
        if r.needs_review {
            agg.needs_review = true;
        }
    }

    map.into_iter()
        .map(|((date, cn, sci), agg)| EbirdRow {
            species_cn: cn,
            species_sci: sci,
            count: agg.count,
            date,
            lat: agg.lat,
            lon: agg.lon,
            note: agg.needs_review.then(|| "待确认".to_string()),
        })
        .collect()
}

/// 拍摄日期：EXIF date_time_original 优先（解析 YYYY-MM-DD），回退文件 mtime
/// （照抄 import.rs 的 build_candidate fallback 语义；文件缺失按纪元日兜底）。
fn row_date(path: &Path, exif: Option<&ExifMetadata>) -> String {
    if let Some(d) = exif
        .and_then(|e| e.date_time_original.as_deref())
        .and_then(parse_exif_date)
    {
        return d;
    }
    std::fs::metadata(path)
        .ok()
        .map(|meta| mtime_date(&meta))
        .unwrap_or_else(|| "1970-01-01".to_string())
}

/// 经纬度（十进制度）：EXIF GPS 度分秒 → 十进制；无 GPS 返回 (None, None)。
fn row_gps(exif: Option<&ExifMetadata>) -> (Option<f64>, Option<f64>) {
    let Some(e) = exif else { return (None, None) };
    let lat = e.gps.latitude.map(|(d, m, s)| dms_to_decimal(d, m, s));
    let lon = e.gps.longitude.map(|(d, m, s)| dms_to_decimal(d, m, s));
    (lat, lon)
}

/// EXIF 日期串 → YYYY-MM-DD（与 import.rs::parse_exif_date 同实现）：
/// 支持标准 EXIF 形态 "2024:01:02 10:30:00" 与 "-" 分隔形态（部分相机/手机）。
fn parse_exif_date(raw: &str) -> Option<String> {
    let raw = raw.trim();
    for fmt in ["%Y:%m:%d %H:%M:%S", "%Y-%m-%d %H:%M:%S"] {
        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(raw, fmt) {
            return Some(format!("{:04}-{:02}-{:02}", dt.year(), dt.month(), dt.day()));
        }
    }
    // 仅日期形态（部分机型无时间字段）
    for fmt in ["%Y:%m:%d", "%Y-%m-%d"] {
        if let Ok(d) = chrono::NaiveDate::parse_from_str(raw, fmt) {
            return Some(format!("{:04}-{:02}-{:02}", d.year(), d.month(), d.day()));
        }
    }
    None
}

/// 文件修改时间 → YYYY-MM-DD（与 import.rs::mtime_date 同实现）
fn mtime_date(meta: &std::fs::Metadata) -> String {
    meta.modified()
        .map(|t| {
            let dt: chrono::DateTime<chrono::Local> = t.into();
            format!("{:04}-{:02}-{:02}", dt.year(), dt.month(), dt.day())
        })
        .unwrap_or_else(|_| "1970-01-01".to_string())
}

/// 写出 CSV（UTF-8 BOM + RFC 4180 转义）。父目录不存在自动创建；覆盖已存在文件。
pub fn write_csv(rows: &[EbirdRow], dest: &Path) -> Result<(), EbirdError> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut out = std::io::BufWriter::new(std::fs::File::create(dest)?);
    out.write_all("\u{feff}".as_bytes())?;
    out.write_all("中文名,学名,数量,日期,纬度,经度,备注\n".as_bytes())?;
    for r in rows {
        let line = [
            csv_field(&r.species_cn),
            csv_field(&r.species_sci),
            csv_field(&r.count.to_string()),
            csv_field(&r.date),
            csv_field(&r.lat.map(fmt_coord).unwrap_or_default()),
            csv_field(&r.lon.map(fmt_coord).unwrap_or_default()),
            csv_field(r.note.as_deref().unwrap_or_default()),
        ]
        .join(",");
        out.write_all(line.as_bytes())?;
        out.write_all(b"\n")?;
    }
    out.flush()?;
    Ok(())
}

/// RFC 4180 字段转义：含逗号/引号/换行（CR/LF）时整体加引号，内部引号加倍。
fn csv_field(f: &str) -> String {
    if f.contains(',') || f.contains('"') || f.contains('\n') || f.contains('\r') {
        format!("\"{}\"", f.replace('"', "\"\""))
    } else {
        f.to_string()
    }
}

/// 坐标输出：最多 6 位小数，去掉尾随零与末尾小数点（-116.3914 而非 -116.391400）；
/// -0.0 归一为 0。
fn fmt_coord(v: f64) -> String {
    let mut s = format!("{:.6}", v);
    while s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.pop();
    }
    if s == "-0" {
        s = "0".to_string();
    }
    s
}

/// 路径键归一化：分隔符统一正斜杠；Windows 大小写不敏感（小写化），
/// 用于匹配 exif_cache 的完整路径键（扫描写入路径与 dir.join(rel) 形态可能不同）。
fn norm_key(s: &str) -> String {
    let s = s.replace('\\', "/");
    #[cfg(windows)]
    {
        s.to_lowercase()
    }
    #[cfg(not(windows))]
    {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photo_domain::{BirdMatch, Recognition, RecognitionFailureStage};

    /// 测试用识别行（bird 匹配成功，含学名）
    fn rec(status: RecognitionStatus, cn: &str, latin: &str) -> Recognition {
        Recognition {
            status,
            bird: Some(BirdMatch {
                bird_id: 1,
                cn_name: cn.to_string(),
                latin_name: latin.to_string(),
            }),
            class_index: Some(100),
            confidence: Some(95.0),
            bbox: None,
            eye_sharpness: None,
            eye_bbox: None,
            candidates: Vec::new(),
            failure_stage: RecognitionFailureStage::None,
            recognized_at: "2024-01-01T00:00:00Z".to_string(),
        }
    }

    fn frow(rel: &str, cn: &str, date: &str, needs_review: bool) -> FileRow {
        FileRow {
            rel_path: rel.to_string(),
            cn: cn.to_string(),
            sci: String::new(),
            date: date.to_string(),
            lat: None,
            lon: None,
            needs_review,
        }
    }

    #[test]
    fn test_aggregate_same_species_same_day_merged() {
        let rows = aggregate(vec![
            frow("a.jpg", "乌鸫", "2024-05-06", false),
            frow("b.jpg", "乌鸫", "2024-05-06", false),
            frow("c.jpg", "乌鸫", "2024-05-06", false),
        ]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].species_cn, "乌鸫");
        assert_eq!(rows[0].count, 3);
        assert_eq!(rows[0].date, "2024-05-06");
        assert_eq!(rows[0].note, None);
    }

    #[test]
    fn test_aggregate_split_by_day() {
        let rows = aggregate(vec![
            frow("a.jpg", "乌鸫", "2024-05-06", false),
            frow("b.jpg", "乌鸫", "2024-05-07", false),
            frow("c.jpg", "乌鸫", "2024-05-07", false),
        ]);
        // 跨日拆分：两条记录，日期升序
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].date, "2024-05-06");
        assert_eq!(rows[0].count, 1);
        assert_eq!(rows[1].date, "2024-05-07");
        assert_eq!(rows[1].count, 2);
    }

    #[test]
    fn test_aggregate_needs_review_note() {
        // 组内混入 NeedsReview → 计入并标注「待确认」
        let rows = aggregate(vec![
            frow("a.jpg", "麻雀", "2024-05-06", false),
            frow("b.jpg", "麻雀", "2024-05-06", true),
        ]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].count, 2);
        assert_eq!(rows[0].note.as_deref(), Some("待确认"));
        // 纯 Confirmed 无备注
        let rows = aggregate(vec![frow("a.jpg", "麻雀", "2024-05-06", false)]);
        assert_eq!(rows[0].note, None);
    }

    #[test]
    fn test_aggregate_gps_first_with_gps() {
        // 组内首张无 GPS、次张有 → 取首个有 GPS 的坐标（独立 lat/lon）
        let mut r1 = frow("a.jpg", "白鹭", "2024-05-06", false);
        r1.lat = None;
        r1.lon = None;
        let mut r2 = frow("b.jpg", "白鹭", "2024-05-06", false);
        r2.lat = Some(39.9);
        r2.lon = Some(116.3);
        let rows = aggregate(vec![r1, r2]);
        assert_eq!(rows[0].lat, Some(39.9));
        assert_eq!(rows[0].lon, Some(116.3));
    }

    #[test]
    fn test_parse_exif_date_variants() {
        assert_eq!(parse_exif_date("2024:05:06 07:08:09"), Some("2024-05-06".to_string()));
        assert_eq!(parse_exif_date("2024-05-06"), Some("2024-05-06".to_string()));
        assert_eq!(parse_exif_date("not a date"), None);
    }

    #[test]
    fn test_build_rows_skips_unrecognized_and_no_bird() {
        // 真实 folder_db：Unrecognized（有鸟种名）与 bird None（NeedsReview）均不计入，
        // 只有 Confirmed + bird 匹配成功的行进入结果
        let dir = tempfile::tempdir().unwrap();
        let db = FolderDb::open_in_dir(dir.path()).unwrap();
        db.upsert_recognition("a.jpg", &rec(RecognitionStatus::Unrecognized, "麻雀", "Passer montanus"))
            .unwrap();
        let mut no_bird = rec(RecognitionStatus::NeedsReview, "麻雀", "Passer montanus");
        no_bird.bird = None;
        db.upsert_recognition("b.jpg", &no_bird).unwrap();
        db.upsert_recognition("c.jpg", &rec(RecognitionStatus::Confirmed, "乌鸫", "Turdus merula"))
            .unwrap();

        let rows = build_rows(dir.path()).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].species_cn, "乌鸫");
        assert_eq!(rows[0].count, 1);
    }

    #[test]
    fn test_csv_field_escaping() {
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("乌,鸫"), "\"乌,鸫\"");
        assert_eq!(csv_field("说\"好\""), "\"说\"\"好\"\"\"");
        assert_eq!(csv_field("a\nb"), "\"a\nb\"");
        assert_eq!(csv_field("a\r\nb"), "\"a\r\nb\"");
    }

    #[test]
    fn test_write_csv_bom_and_escaping() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("exports/ebird_test.csv");
        let rows = vec![
            EbirdRow {
                species_cn: "乌,鸫".to_string(),
                species_sci: "Turdus merula".to_string(),
                count: 2,
                date: "2024-05-06".to_string(),
                lat: Some(39.9),
                lon: Some(116.383333),
                note: None,
            },
            EbirdRow {
                species_cn: "大山雀".to_string(),
                species_sci: "Parus major".to_string(),
                count: 1,
                date: "2024-05-06".to_string(),
                lat: None,
                lon: None,
                note: Some("待确认".to_string()),
            },
        ];
        write_csv(&rows, &dest).unwrap();
        let bytes = std::fs::read(&dest).unwrap();
        // UTF-8 BOM
        assert!(bytes.starts_with(b"\xef\xbb\xbf"));
        let text = String::from_utf8(bytes[3..].to_vec()).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "中文名,学名,数量,日期,纬度,经度,备注");
        // 含逗号字段转义 + 坐标尾零裁剪；GPS None 输出空单元格
        assert_eq!(lines[1], "\"乌,鸫\",Turdus merula,2,2024-05-06,39.9,116.383333,");
        // 备注列（无特殊字符不转义）+ 无 GPS 空单元格
        assert_eq!(lines[2], "大山雀,Parus major,1,2024-05-06,,,待确认");
    }

    #[test]
    fn test_build_rows_exif_date_and_gps() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("IMG_0001.jpg");
        std::fs::write(&path, b"fake").unwrap();
        let db = FolderDb::open_in_dir(dir.path()).unwrap();
        db.upsert_recognition("IMG_0001.jpg", &rec(RecognitionStatus::Confirmed, "乌鸫", "Turdus merula"))
            .unwrap();
        // EXIF：日期 + GPS 度分秒（39°54'00"N, 116°23'00"E）
        let mut exif = ExifMetadata::default();
        exif.date_time_original = Some("2024:05:06 07:08:09".to_string());
        exif.gps.latitude = Some((39.0, 54.0, 0.0));
        exif.gps.longitude = Some((116.0, 23.0, 0.0));
        db.put_exif(&path, &exif).unwrap();

        let rows = build_rows(dir.path()).unwrap();
        assert_eq!(rows.len(), 1);
        // 日期：EXIF 优先（"2024:05:06 ..." → YYYY-MM-DD）
        assert_eq!(rows[0].date, "2024-05-06");
        // GPS：度分秒 → 十进制（39 + 54/60 = 39.9；116 + 23/60）
        assert_eq!(rows[0].lat, Some(39.9));
        assert_eq!(rows[0].lon, Some(116.0 + 23.0 / 60.0));
        assert_eq!(rows[0].count, 1);
    }

    #[test]
    fn test_build_rows_mtime_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("IMG_0002.jpg");
        std::fs::write(&path, b"fake").unwrap();
        let db = FolderDb::open_in_dir(dir.path()).unwrap();
        // 只写识别行，不写 EXIF → 日期回退文件 mtime（刚写入 = 今天）
        db.upsert_recognition("IMG_0002.jpg", &rec(RecognitionStatus::Confirmed, "麻雀", "Passer montanus"))
            .unwrap();

        let rows = build_rows(dir.path()).unwrap();
        assert_eq!(rows.len(), 1);
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        assert_eq!(rows[0].date, today);
    }

    #[test]
    fn test_build_rows_empty_folder_returns_empty() {
        // 空目录（无识别行）→ 空结果（FolderDb::open_in_dir 建空库不报错）
        let dir = tempfile::tempdir().unwrap();
        let rows = build_rows(dir.path()).unwrap();
        assert!(rows.is_empty());
    }
}
