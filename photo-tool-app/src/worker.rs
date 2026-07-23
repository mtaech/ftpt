use gpui::*;
use rayon::ThreadPoolBuilder;

use crate::state::app::RootView;

pub struct Worker {
    pool: rayon::ThreadPool,
}

impl Worker {
    pub fn new() -> Self {
        let pool = ThreadPoolBuilder::new()
            .num_threads(
                std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(4),
            )
            .build()
            .expect("Failed to create rayon thread pool");
        Self { pool }
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
        let (tx, rx) = oneshot::channel();
        self.pool.spawn(move || {
            let _ = tx.send(f());
        });
        let _task: Task<()> = cx.spawn(|weak_view: WeakEntity<RootView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                if let Ok(result) = rx.await {
                    if let Some(view) = weak_view.upgrade() {
                        cx.update_entity(&view, |this, cx| {
                            on_done(this, result, cx);
                        });
                    }
                }
            }
        });
    }
}
