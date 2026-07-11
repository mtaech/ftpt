mod commands;

use commands::browse;
use commands::thumbnails;
use commands::exif_cmd;
use commands::xmp;
use commands::files;
use commands::config;
use commands::convert_cmd;
use commands::import_cmd;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_log::Builder::new().build())
        .manage(thumbnails::make_cache())
        .invoke_handler(tauri::generate_handler![
            browse::open_directory,
            browse::get_directory_tree,
            browse::expand_directory,
            thumbnails::get_thumbnail,
            thumbnails::clear_cache,
            thumbnails::get_cache_stats,
            exif_cmd::get_exif,
            xmp::read_capture_xmp,
            xmp::write_capture_xmp,
            files::delete_captures,
            files::move_captures,
            files::copy_captures,
            files::rename_captures,
            config::load_config,
            config::save_config,
            convert_cmd::convert_images,
            import_cmd::detect_drives,
            import_cmd::import_captures,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
