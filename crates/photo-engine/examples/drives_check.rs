//! 打印 detect_removable_drives() 的结果（排查「导入」对话框为何没列出某张卡）。
//! 用法：cargo run -p photo-engine --example drives_check

fn main() {
    let drives = photo_engine::import::detect_removable_drives();
    if drives.is_empty() {
        println!("（未检测到可移动驱动器）");
    }
    for d in drives {
        println!("{:?}\t{:?}", d.path, d.label);
    }
}
