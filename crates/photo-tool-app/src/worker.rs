use std::any::Any;
use std::panic::{catch_unwind, AssertUnwindSafe};

use gpui::*;
use rayon::ThreadPoolBuilder;

use crate::state::app::RootView;

pub struct Worker {
    /// 批量任务池：缩略图预加载、EXIF 后台提取、DB 同步、缓存清理、文件操作、识别
    pool: rayon::ThreadPool,
    /// 交互任务池：预览/全分辨率/网格懒加载等用户等待结果的加载，
    /// 独立线程数避免被批量任务排队阻塞（rayon 无优先级，共用一池时 50 个预加载
    /// 任务会堵住预览任务数秒）。
    fast_pool: rayon::ThreadPool,
}

impl Worker {
    pub fn new() -> Self {
        let n = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        // fast 池 4 线程：交互加载（预览/全分辨率/网格缩略图）任务小而多，
        // 快速拖动滚动条时大量缩略图任务排队，2 线程吞吐不够导致渐显慢
        let fast = n.clamp(1, 4);
        let pool = ThreadPoolBuilder::new()
            .num_threads(n)
            .build()
            .expect("Failed to create rayon thread pool");
        let fast_pool = ThreadPoolBuilder::new()
            .num_threads(fast)
            .build()
            .expect("Failed to create rayon fast thread pool");
        Self { pool, fast_pool }
    }

    /// Spawn work on rayon, deliver result via GPUI async context.
    ///
    /// 泛型约束 `R: Default`：worker 闭包 panic 时无法构造结果值，用 `R::default()`
    /// 兜底（Option 类型即 None），保证 on_done 必定执行——否则 UI 侧 loading/进度
    /// 哨兵（grid_loading、preview_loading、batch_in_progress 等）会永久置位，
    /// 且 GPUI 吞掉 UI 线程 panic，表现为「点了没反应」。
    pub fn spawn<F, R>(
        &self,
        cx: &Context<RootView>,
        f: F,
        on_done: impl Fn(&mut RootView, R, &mut Context<RootView>) + 'static,
    ) where
        F: FnOnce() -> R + Send + 'static,
        R: Default + Send + 'static,
    {
        self.spawn_on(&self.pool, cx, f, on_done, std::panic::Location::caller());
    }

    /// 交互优先池：用于用户等待结果的加载（预览/全分辨率/网格懒加载），
    /// 不被批量预加载任务排队阻塞。
    pub fn spawn_fast<F, R>(
        &self,
        cx: &Context<RootView>,
        f: F,
        on_done: impl Fn(&mut RootView, R, &mut Context<RootView>) + 'static,
    ) where
        F: FnOnce() -> R + Send + 'static,
        R: Default + Send + 'static,
    {
        self.spawn_on(&self.fast_pool, cx, f, on_done, std::panic::Location::caller());
    }

    fn spawn_on<F, R>(
        &self,
        pool: &rayon::ThreadPool,
        cx: &Context<RootView>,
        f: F,
        on_done: impl Fn(&mut RootView, R, &mut Context<RootView>) + 'static,
        location: &'static std::panic::Location<'static>,
    ) where
        F: FnOnce() -> R + Send + 'static,
        R: Default + Send + 'static,
    {
        let (tx, rx) = oneshot::channel();
        pool.spawn(move || {
            // 闭包 panic 防护：捕获后记录载荷（含调用位置）并以 R::default() 兜底，
            // 照常 send 让 on_done 必定执行、UI 侧 loading/进度哨兵复位
            let result = catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|payload| {
                log_worker_panic(payload, location);
                R::default()
            });
            let _ = tx.send(result);
        });
        // GPUI 的 Task 丢弃即取消，必须 detach 让桥接任务跑完
        cx.spawn(|weak_view: WeakEntity<RootView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                if let (Ok(result), Some(view)) = (rx.await, weak_view.upgrade()) {
                    cx.update_entity(&view, |this, cx| {
                        on_done(this, result, cx);
                    });
                }
            }
        })
        .detach();
    }
}

/// 记录 worker 闭包 panic 的载荷与调用位置；on_done 会收到 `R::default()` 兜底值。
fn log_worker_panic(
    payload: Box<dyn Any + Send>,
    location: &'static std::panic::Location<'static>,
) {
    let msg = if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "非字符串 panic 载荷".to_string()
    };
    tracing::error!(
        "worker 任务 panic（调用位置 {location}），结果以默认值兜底，on_done 仍会执行: {msg}"
    );
}
