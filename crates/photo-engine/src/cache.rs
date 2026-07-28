use std::path::Path;
use std::sync::Arc;
use parking_lot::Mutex;
use thiserror::Error;
use rusqlite_migration::{Migrations, M};

use photo_domain::ImageFormat;

use photo_domain::ExifMetadata;

/// EXIF 缓存表迁移
fn cache_migrations() -> Migrations<'static> {
    Migrations::new(vec![
        M::up(
            "CREATE TABLE IF NOT EXISTS exif_cache (
                path        TEXT PRIMARY KEY,
                file_size   INTEGER NOT NULL,
                mtime_ns    INTEGER NOT NULL,
                make        TEXT,
                model       TEXT,
                lens        TEXT,
                exposure_time TEXT,
                f_number    TEXT,
                iso         INTEGER,
                focal_length TEXT,
                exposure_compensation TEXT,
                white_balance TEXT,
                date_time_original TEXT,
                image_width  INTEGER,
                image_height INTEGER,
                file_size_cache INTEGER,
                color_space  TEXT,
                orientation  INTEGER,
                gps_lat_deg  REAL,
                gps_lat_min  REAL,
                gps_lat_sec  REAL,
                gps_lon_deg  REAL,
                gps_lon_min  REAL,
                gps_lon_sec  REAL,
                gps_altitude REAL
            );",
        ),
    ])
}
#[derive(Error, Debug)]
pub enum CacheError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Migration error: {0}")]
    Migration(#[from] rusqlite_migration::Error),
}

#[derive(Clone)]
pub struct ExifCache {
    conn: Arc<Mutex<rusqlite::Connection>>,
}

impl ExifCache {
    /// 在指定目录中打开或创建 `.pt-cache.db`，自动迁移表结构。
    pub fn open_in_dir(dir: &Path) -> Result<Self, CacheError> {
        let db_path = dir.join(".pt-cache.db");
        let mut conn = rusqlite::Connection::open(&db_path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        cache_migrations().to_latest(&mut conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// 查询缓存。返回 `None` 表示未命中或已失效。
    pub fn get(&self, path: &Path) -> Result<Option<ExifMetadata>, CacheError> {
        let path_str = path.to_string_lossy();
        let (size, mtime) = match file_fingerprint(path) {
            Ok(f) => f,
            Err(_) => return Ok(None),
        };

        let conn = self.conn.lock();
        let mut stmt = conn.prepare_cached(
            "SELECT make, model, lens, exposure_time, f_number, iso, focal_length,
                    exposure_compensation, white_balance, date_time_original,
                    image_width, image_height, file_size_cache, color_space, orientation,
                    gps_lat_deg, gps_lat_min, gps_lat_sec,
                    gps_lon_deg, gps_lon_min, gps_lon_sec, gps_altitude
             FROM exif_cache
             WHERE path = ?1 AND file_size = ?2 AND mtime_ns = ?3",
        )?;

        match stmt.query_row(
            rusqlite::params![path_str.as_ref(), size as i64, mtime as i64],
            |row| row_to_exif(row),
        ) {
            Ok(exif) => Ok(Some(exif)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// 写入缓存。如果路径已存在则更新（UPSERT）。
    pub fn put(&self, path: &Path, exif: &ExifMetadata) -> Result<(), CacheError> {
        let path_str = path.to_string_lossy();
        let (size, mtime) = match file_fingerprint(path) {
            Ok(f) => f,
            Err(_) => return Ok(()),
        };

        let conn = self.conn.lock();
        let params = exif_to_params(path_str.as_ref(), size, mtime, exif);
        conn.execute(
            "INSERT OR REPLACE INTO exif_cache
             (path, file_size, mtime_ns, make, model, lens, exposure_time, f_number, iso,
              focal_length, exposure_compensation, white_balance, date_time_original,
              image_width, image_height, file_size_cache, color_space, orientation,
              gps_lat_deg, gps_lat_min, gps_lat_sec,
              gps_lon_deg, gps_lon_min, gps_lon_sec, gps_altitude)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25)",
            rusqlite::params_from_iter(params.iter().map(|b| b.as_ref())),
        )?;
        Ok(())
    }

    /// 获取或提取：缓存命中直接返回；未命中则提取、写入缓存、返回。
    pub fn get_or_extract(
        &self,
        path: &Path,
        format: &ImageFormat,
    ) -> Result<ExifMetadata, CacheError> {
        if let Some(cached) = self.get(path)? {
            return Ok(cached);
        }
        let exif = crate::exif::extract_exif(path, format)
            .unwrap_or_default();
        let _ = self.put(path, &exif);
        Ok(exif)
    }

}
fn file_fingerprint(path: &Path) -> std::io::Result<(u64, u64)> {
    let meta = std::fs::metadata(path)?;
    let size = meta.len();
    let mtime = meta
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    Ok((size, mtime))
}

fn row_to_exif(row: &rusqlite::Row) -> rusqlite::Result<ExifMetadata> {
    use photo_domain::{CameraInfo, GpsInfo, ShootingParams};

    let gps_lat = match (
        row.get::<_, Option<f64>>(15)?,
        row.get::<_, Option<f64>>(16)?,
        row.get::<_, Option<f64>>(17)?,
    ) {
        (Some(d), Some(m), Some(s)) => Some((d, m, s)),
        _ => None,
    };
    let gps_lon = match (
        row.get::<_, Option<f64>>(18)?,
        row.get::<_, Option<f64>>(19)?,
        row.get::<_, Option<f64>>(20)?,
    ) {
        (Some(d), Some(m), Some(s)) => Some((d, m, s)),
        _ => None,
    };

    Ok(ExifMetadata {
        camera: CameraInfo {
            make: row.get(0)?,
            model: row.get(1)?,
            lens: row.get(2)?,
        },
        shooting: ShootingParams {
            exposure_time: row.get(3)?,
            f_number: row.get(4)?,
            iso: row.get(5)?,
            focal_length: row.get(6)?,
            exposure_compensation: row.get(7)?,
            white_balance: row.get(8)?,
        },
        date_time_original: row.get(9)?,
        image_width: row.get(10)?,
        image_height: row.get(11)?,
        file_size: row.get::<_, Option<i64>>(12)?.map(|v| v as u64),
        color_space: row.get(13)?,
        orientation: row.get(14)?,
        gps: GpsInfo {
            latitude: gps_lat,
            longitude: gps_lon,
            altitude: row.get(21)?,
        },
    })
}

fn exif_to_params<'a>(
    path: &'a str,
    size: u64,
    mtime: u64,
    exif: &'a ExifMetadata,
) -> Vec<Box<dyn rusqlite::types::ToSql + 'a>> {
    let (lat_deg, lat_min, lat_sec) = exif.gps.latitude.map(|(d, m, s)| (Some(d), Some(m), Some(s))).unwrap_or((None, None, None));
    let (lon_deg, lon_min, lon_sec) = exif.gps.longitude.map(|(d, m, s)| (Some(d), Some(m), Some(s))).unwrap_or((None, None, None));

    vec![
        Box::new(path.to_string()),
        Box::new(size as i64),
        Box::new(mtime as i64),
        Box::new(exif.camera.make.clone()),
        Box::new(exif.camera.model.clone()),
        Box::new(exif.camera.lens.clone()),
        Box::new(exif.shooting.exposure_time.clone()),
        Box::new(exif.shooting.f_number.clone()),
        Box::new(exif.shooting.iso.map(|v| v as i64)),
        Box::new(exif.shooting.focal_length.clone()),
        Box::new(exif.shooting.exposure_compensation.clone()),
        Box::new(exif.shooting.white_balance.clone()),
        Box::new(exif.date_time_original.clone()),
        Box::new(exif.image_width.map(|v| v as i64)),
        Box::new(exif.image_height.map(|v| v as i64)),
        Box::new(exif.file_size.map(|v| v as i64)),
        Box::new(exif.color_space.clone()),
        Box::new(exif.orientation.map(|v| v as i64)),
        Box::new(lat_deg),
        Box::new(lat_min),
        Box::new(lat_sec),
        Box::new(lon_deg),
        Box::new(lon_min),
        Box::new(lon_sec),
        Box::new(exif.gps.altitude),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use photo_domain::{CameraInfo, GpsInfo, ShootingParams};
    use tempfile::TempDir;

    fn make_exif() -> ExifMetadata {
        ExifMetadata {
            camera: CameraInfo {
                make: Some("Nikon".into()),
                model: Some("D850".into()),
                lens: Some("24-70mm f/2.8".into()),
            },
            shooting: ShootingParams {
                exposure_time: Some("1/250".into()),
                f_number: Some("f/5.6".into()),
                iso: Some(200),
                focal_length: Some("50mm".into()),
                exposure_compensation: None,
                white_balance: None,
            },
            date_time_original: Some("2024-01-15 10:30:00".into()),
            image_width: Some(8256),
            image_height: Some(5504),
            file_size: Some(42_000_000),
            color_space: Some("sRGB".into()),
            orientation: Some(1),
            gps: GpsInfo {
                latitude: Some((39.0, 54.0, 26.0)),
                longitude: Some((116.0, 23.0, 29.0)),
                altitude: Some(50.5),
            },
        }
    }

    #[test]
    fn test_cache_put_get_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let jpg = tmp.path().join("test.jpg");
        std::fs::write(&jpg, b"fake jpeg data").unwrap();
        let cache = ExifCache::open_in_dir(tmp.path()).unwrap();
        let exif = make_exif();
        assert!(cache.get(&jpg).unwrap().is_none());
        cache.put(&jpg, &exif).unwrap();
        let got = cache.get(&jpg).unwrap().expect("should hit");
        assert_eq!(got.camera.make.as_deref(), Some("Nikon"));
        assert_eq!(got.shooting.iso, Some(200));
        assert_eq!(got.gps.latitude, Some((39.0, 54.0, 26.0)));
    }

    #[test]
    fn test_cache_stale_on_size_change() {
        let tmp = TempDir::new().unwrap();
        let jpg = tmp.path().join("test.jpg");
        std::fs::write(&jpg, b"original").unwrap();
        let cache = ExifCache::open_in_dir(tmp.path()).unwrap();
        cache.put(&jpg, &make_exif()).unwrap();
        std::fs::write(&jpg, b"modified data here").unwrap();
        assert!(cache.get(&jpg).unwrap().is_none());
    }

    #[test]
    fn test_cache_nonexistent_file() {
        let tmp = TempDir::new().unwrap();
        let cache = ExifCache::open_in_dir(tmp.path()).unwrap();
        assert!(cache.get(Path::new("/nonexistent/file.orf")).unwrap().is_none());
    }

    #[test]
    fn test_cache_upsert() {
        let tmp = TempDir::new().unwrap();
        let jpg = tmp.path().join("test.jpg");
        std::fs::write(&jpg, b"fake jpeg").unwrap();
        let cache = ExifCache::open_in_dir(tmp.path()).unwrap();
        let mut exif1 = make_exif();
        exif1.shooting.iso = Some(100);
        cache.put(&jpg, &exif1).unwrap();
        let mut exif2 = make_exif();
        exif2.shooting.iso = Some(400);
        cache.put(&jpg, &exif2).unwrap();
        let got = cache.get(&jpg).unwrap().unwrap();
        assert_eq!(got.shooting.iso, Some(400));
    }
}
