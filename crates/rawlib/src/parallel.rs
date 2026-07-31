//! Parallel processing support for rawlib
//!
//! This module provides high-level parallel processing capabilities
//! for batch extracting thumbnails from multiple RAW files.

use crate::{DecodeOptions, RawError, RawProcessor, ThumbnailData};
use rayon::prelude::*;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Configuration for parallel processing
pub struct ParallelConfig {
    /// Number of parallel jobs (None = use all CPU cores)
    pub jobs: Option<usize>,
    /// Enable verbose output during processing
    pub verbose: bool,
    /// Optional progress callback: receives (completed_count, total_count)
    pub on_progress: Option<Arc<dyn Fn(usize, usize) + Send + Sync>>,
}

impl std::fmt::Debug for ParallelConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParallelConfig")
            .field("jobs", &self.jobs)
            .field("verbose", &self.verbose)
            .field(
                "on_progress",
                &self.on_progress.as_ref().map(|_| "<callback>"),
            )
            .finish()
    }
}

impl Clone for ParallelConfig {
    fn clone(&self) -> Self {
        Self {
            jobs: self.jobs.clone(),
            verbose: self.verbose.clone(),
            on_progress: self.on_progress.clone(),
        }
    }
}

impl Default for ParallelConfig {
    fn default() -> Self {
        Self {
            jobs: None,
            verbose: false,
            on_progress: None,
        }
    }
}

/// Result of processing a single file
#[derive(Debug)]
pub struct ProcessResult {
    /// Input file path
    pub path: PathBuf,
    /// Processing result
    pub thumbnail: Result<ThumbnailData, RawError>,
    /// Processing time
    pub elapsed: Duration,
    /// Input file size in bytes
    pub input_size: u64,
}

impl ProcessResult {
    /// Returns true if processing was successful
    pub fn is_success(&self) -> bool {
        self.thumbnail.is_ok()
    }

    /// Returns true if processing failed
    pub fn is_error(&self) -> bool {
        self.thumbnail.is_err()
    }

    /// Get the thumbnail data if successful
    pub fn thumbnail(&self) -> Option<&ThumbnailData> {
        self.thumbnail.as_ref().ok()
    }

    /// Get the error if failed
    pub fn error(&self) -> Option<&RawError> {
        self.thumbnail.as_ref().err()
    }
}

/// Statistics for parallel processing
#[derive(Debug, Default, Serialize)]
pub struct ProcessingStats {
    /// Total number of files
    pub total: usize,
    /// Number of successfully processed files
    pub success: usize,
    /// Number of failed files
    pub failed: usize,
    /// Total processing time
    #[serde(skip)]
    pub total_elapsed: Duration,
    /// Total input bytes
    pub total_input_bytes: u64,
    /// Total output bytes
    pub total_output_bytes: u64,
}

impl ProcessingStats {
    /// Calculate processing speed in files per second
    pub fn files_per_second(&self) -> f64 {
        let secs = self.total_elapsed.as_secs_f64();
        if secs > 0.0 {
            self.total as f64 / secs
        } else {
            0.0
        }
    }

    /// Calculate average time per file in milliseconds
    pub fn ms_per_file(&self) -> f64 {
        if self.total > 0 {
            self.total_elapsed.as_secs_f64() * 1000.0 / self.total as f64
        } else {
            0.0
        }
    }

    /// Calculate compression ratio
    pub fn compression_ratio(&self) -> f64 {
        if self.total_input_bytes > 0 {
            self.total_output_bytes as f64 / self.total_input_bytes as f64 * 100.0
        } else {
            0.0
        }
    }
}

/// Parallel processor for batch processing RAW files
pub struct ParallelProcessor;

impl ParallelProcessor {
    /// Process multiple files in parallel
    pub fn process_files<P: AsRef<Path> + Send + Sync>(
        files: &[P],
        config: &ParallelConfig,
    ) -> Vec<ProcessResult> {
        let start = Instant::now();

        // Configure thread pool
        let jobs = config.jobs.unwrap_or_else(num_cpus::get);
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(jobs)
            .build()
            .ok();

        // Progress tracking
        let progress_counter = Arc::new(AtomicUsize::new(0));
        let on_progress = &config.on_progress;
        let total = files.len();

        // Process files
        let process_fn = |path: &P| {
            let path = path.as_ref();
            let file_start = Instant::now();

            // Get input file size
            let input_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

            // Extract thumbnail
            let result = RawProcessor::extract_thumbnail(path);

            // Report progress
            let completed = progress_counter.fetch_add(1, Ordering::Relaxed) + 1;
            if let Some(ref cb) = on_progress {
                cb(completed, total);
            }

            ProcessResult {
                path: path.to_path_buf(),
                thumbnail: result,
                elapsed: file_start.elapsed(),
                input_size,
            }
        };

        let results: Vec<ProcessResult> = match pool {
            Some(pool) => pool.install(|| files.par_iter().map(process_fn).collect()),
            None => files.par_iter().map(process_fn).collect(),
        };

        if config.verbose {
            let elapsed = start.elapsed();
            let success_count = results
                .iter()
                .filter(|r: &&ProcessResult| r.is_success())
                .count();
            println!(
                "Processed {} files in {:?} ({} succeeded, {} failed)",
                results.len(),
                elapsed,
                success_count,
                results.len() - success_count
            );
        }

        results
    }

    /// Process multiple files in parallel with full RAW decoding
    ///
    /// Unlike `process_files` (thumbnail extraction), this runs the full
    /// decode pipeline (unpack → demosaic → pixel data) with the given
    /// [`DecodeOptions`]. Each worker thread lazily creates and reuses a
    /// single `RawProcessor` instance (`recycle()` between files), avoiding
    /// repeated `libraw_init`/`libraw_close` overhead per file.
    ///
    /// A failing file does not affect the rest of the batch; its error is
    /// reported in that file's [`ProcessResult`].
    ///
    /// # OpenMP 协调
    ///
    /// 实测在满核并行批处理时，LibRaw 内部 OpenMP 与文件级并行会争抢核
    /// （线程超售），吞吐下降约 15-20%。因此当 `jobs > 1` 且用户未显式
    /// 设置 `OMP_NUM_THREADS` 时，本方法会将其设为 `1`，让并行只发生在
    /// 文件级。注意：OpenMP 运行时在首次并行区前读取该环境变量，若进程
    /// 中此前已执行过 LibRaw 解码，此设置不生效（实测此时调用
    /// `omp_set_num_threads` 反而更慢）。对吞吐敏感的场景建议启动进程时
    /// 就在外部设置 `OMP_NUM_THREADS=1`。
    pub fn process_images<P: AsRef<Path> + Send + Sync>(
        files: &[P],
        config: &ParallelConfig,
        opts: &DecodeOptions,
    ) -> Vec<ProcessResult> {
        let start = Instant::now();

        // Configure thread pool
        let jobs = config.jobs.unwrap_or_else(num_cpus::get);

        // 满核并行时关闭 LibRaw 内部 OpenMP，避免线程超售（用户显式设置时尊重用户）
        if jobs > 1 && std::env::var_os("OMP_NUM_THREADS").is_none() {
            std::env::set_var("OMP_NUM_THREADS", "1");
        }

        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(jobs)
            .build()
            .ok();

        // Progress tracking
        let progress_counter = Arc::new(AtomicUsize::new(0));
        let on_progress = &config.on_progress;
        let total = files.len();

        // Process a single file with the thread-local processor
        let process_fn = |proc: &mut Result<RawProcessor, RawError>, path: &P| {
            let path = path.as_ref();
            let file_start = Instant::now();

            // Get input file size
            let input_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

            // Decode full image, reusing the per-thread processor
            let result = match proc {
                Ok(p) => {
                    p.recycle();
                    p.open_file(path)
                        .and_then(|_| {
                            p.set_decode_options(opts);
                            p.unpack()
                        })
                        .and_then(|_| p.dcraw_process())
                        .and_then(|_| p.get_image())
                }
                Err(e) => Err(e.clone()),
            };

            // Report progress
            let completed = progress_counter.fetch_add(1, Ordering::Relaxed) + 1;
            if let Some(ref cb) = on_progress {
                cb(completed, total);
            }

            ProcessResult {
                path: path.to_path_buf(),
                thumbnail: result,
                elapsed: file_start.elapsed(),
                input_size,
            }
        };

        let results: Vec<ProcessResult> = match pool {
            Some(pool) => pool.install(|| {
                files
                    .par_iter()
                    .map_init(RawProcessor::new, &process_fn)
                    .collect()
            }),
            None => files
                .par_iter()
                .map_init(RawProcessor::new, &process_fn)
                .collect(),
        };

        if config.verbose {
            let elapsed = start.elapsed();
            let success_count = results
                .iter()
                .filter(|r: &&ProcessResult| r.is_success())
                .count();
            println!(
                "Processed {} files in {:?} ({} succeeded, {} failed)",
                results.len(),
                elapsed,
                success_count,
                results.len() - success_count
            );
        }

        results
    }

    /// Process files and collect statistics
    pub fn process_with_stats<P: AsRef<Path> + Send + Sync>(
        files: &[P],
        config: &ParallelConfig,
    ) -> (Vec<ProcessResult>, ProcessingStats) {
        let start = Instant::now();
        let results = Self::process_files(files, config);
        let total_elapsed = start.elapsed();

        let mut stats = ProcessingStats {
            total: results.len(),
            total_elapsed,
            ..Default::default()
        };

        for result in &results {
            stats.total_input_bytes += result.input_size;

            if let Ok(ref thumb) = result.thumbnail {
                stats.success += 1;
                stats.total_output_bytes += thumb.data.len() as u64;
            } else {
                stats.failed += 1;
            }
        }

        (results, stats)
    }

    /// Process a single file (convenience method)
    pub fn process_single<P: AsRef<Path>>(path: P) -> ProcessResult {
        let path = path.as_ref();
        let start = Instant::now();

        let input_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

        let result = RawProcessor::extract_thumbnail(path);

        ProcessResult {
            path: path.to_path_buf(),
            thumbnail: result,
            elapsed: start.elapsed(),
            input_size,
        }
    }
}

/// Convenience function for parallel processing
pub fn process_files_parallel<P: AsRef<Path> + Send + Sync>(files: &[P]) -> Vec<ProcessResult> {
    ParallelProcessor::process_files(files, &ParallelConfig::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parallel_config_default() {
        let config = ParallelConfig::default();
        assert!(config.jobs.is_none());
        assert!(!config.verbose);
    }

    #[test]
    fn test_processing_stats() {
        let stats = ProcessingStats {
            total: 10,
            success: 8,
            failed: 2,
            total_elapsed: Duration::from_secs(2),
            total_input_bytes: 1000,
            total_output_bytes: 100,
        };

        assert_eq!(stats.files_per_second(), 5.0);
        assert_eq!(stats.ms_per_file(), 200.0);
        assert_eq!(stats.compression_ratio(), 10.0);
    }

    #[test]
    fn test_process_images_empty_input() {
        let files: Vec<PathBuf> = vec![];
        let results = ParallelProcessor::process_images(
            &files,
            &ParallelConfig::default(),
            &DecodeOptions::preview(),
        );
        assert!(results.is_empty());
    }

    #[test]
    fn test_process_images_nonexistent_file() {
        let files = vec![PathBuf::from("definitely_not_exists_12345.cr2")];
        let results = ParallelProcessor::process_images(
            &files,
            &ParallelConfig::default(),
            &DecodeOptions::preview(),
        );
        assert_eq!(results.len(), 1);
        assert!(results[0].is_error());
        assert_eq!(results[0].input_size, 0);
    }
}
