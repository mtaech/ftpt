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
    pub fn spawn<F, R>(
        &self,
        cx: &Context<RootView>,
        f: F,
        on_done: impl Fn(&mut RootView, R, &mut Context<RootView>) + 'static,
    ) where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        self.spawn_on(&self.pool, cx, f, on_done);
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
        R: Send + 'static,
    {
        self.spawn_on(&self.fast_pool, cx, f, on_done);
    }

    fn spawn_on<F, R>(
        &self,
        pool: &rayon::ThreadPool,
        cx: &Context<RootView>,
        f: F,
        on_done: impl Fn(&mut RootView, R, &mut Context<RootView>) + 'static,
    ) where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        let (tx, rx) = oneshot::channel();
        pool.spawn(move || {
            let _ = tx.send(f());
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
