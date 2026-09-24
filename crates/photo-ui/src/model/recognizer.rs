//! 常驻识别器的空闲卸载判据（纯逻辑，docs/todo.md #17）。
//!
//! 识别器是懒装配 + 常驻（避免每张重装 1-2 秒），代价是约 600MB 常驻内存。
//! 这里只回答「现在该不该释放」；真正取走 Arc 里的实例在 `engine_ops::start_recognizer_idle_watch`。

use std::time::Duration;

/// 是否应该释放常驻识别器。
///
/// 四个条件缺一不可，任何一条不满足都**不释放**：
/// - `configured_minutes` == 0：用户关掉了自动卸载；
/// - `assembled`：本来就没装配，没有可释放的东西；
/// - `busy`：识别/框选正在进行——释放会打断推理（也会让正在跑的 worker 拿到 None）；
/// - 空闲时长 >= 阈值。
pub fn should_unload_recognizer(
    configured_minutes: u32,
    assembled: bool,
    busy: bool,
    idle: Duration,
) -> bool {
    if configured_minutes == 0 || !assembled || busy {
        return false;
    }
    idle >= Duration::from_secs(configured_minutes as u64 * 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_idle_unload_requires_all_conditions() {
        let idle = Duration::from_secs(20 * 60);
        // 配置 0 = 关闭
        assert!(!should_unload_recognizer(0, true, false, idle));
        // 没装配 = 没东西可释放
        assert!(!should_unload_recognizer(15, false, false, idle));
        // 识别进行中 = 绝不释放（哪怕已经「空闲」超过阈值：批量识别中途不重启模型）
        assert!(!should_unload_recognizer(15, true, true, idle));
        // 四条齐了才释放
        assert!(should_unload_recognizer(15, true, false, idle));
    }

    #[test]
    fn test_idle_unload_threshold_boundary() {
        let minutes = 15u32;
        // 差一秒不释放，正好到点释放（阈值是闭区间）
        assert!(!should_unload_recognizer(
            minutes,
            true,
            false,
            Duration::from_secs(15 * 60 - 1)
        ));
        assert!(should_unload_recognizer(
            minutes,
            true,
            false,
            Duration::from_secs(15 * 60)
        ));
    }
}
