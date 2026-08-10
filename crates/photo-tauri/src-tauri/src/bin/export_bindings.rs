//! 生成前端 TS 绑定（覆盖 src/lib/bindings.ts）。
//!
//! 用法：`cargo run -p photo-tauri --bin export-bindings`
//!
//! 背景：photo-tauri 的 lib #[test] harness 在本机启动即 0xc0000139（环境问题，
//! 见 memory），原导出测试跑不了；改由独立 bin 调用 `specta_builder` 完成导出。
//! 生成物路径相对 src-tauri（cargo run 的工作目录即包根）：../src/lib/bindings.ts

fn main() {
    // BigInt 风格字段（BirdMatch.birdId: i64、CaptureMeta.index: usize 等）导出为
    // TS number（前端数值范围足够，避免 specta 默认禁止导出 BigInt 报错）
    let builder = photo_tauri_lib::specta_builder().dangerously_cast_bigints_to_number();
    builder
        .export(
            specta_typescript::Typescript::default(),
            "../src/lib/bindings.ts",
        )
        .expect("导出 TS 绑定失败");
    println!("bindings.ts 导出完成");
}
