// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Эта же программа служит разборщиком PDF в отдельном процессе (`attach.rs`).
    if let Some(code) = ollivo_lib::helper_main() {
        std::process::exit(code);
    }
    ollivo_lib::run()
}
