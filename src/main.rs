//! auto-fix-window: quét lỗi Windows thường gặp và sửa an toàn.
//! Mở cửa sổ có nút Quét / Sửa. `autofix.exe --scan` chỉ quét và ghi log (dùng cho CI).

#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]

mod app;
mod checks;
mod fix;
mod log;
mod sys;

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let scan_only = std::env::args().any(|a| a == "--scan");
    // Bấm đúp vào .exe: tự mở lại với quyền Admin (Windows hỏi UAC).
    if !scan_only && cfg!(windows) && !sys::is_admin() && sys::relaunch_as_admin() {
        return ExitCode::SUCCESS;
    }

    let log_path = log_path();
    let blocker = if !cfg!(windows) {
        Some("Công cụ này chỉ chạy trên Windows.".to_string())
    } else if !sys::is_admin() {
        Some("Cần quyền Administrator: chuột phải autofix.exe → \"Run as administrator\".".to_string())
    } else if let Err(e) = log::init(&log_path) {
        Some(format!("Không mở được file log {}: {e}", log_path.display()))
    } else {
        log::say(&format!(
            "auto-fix-window {} | log: {}",
            env!("CARGO_PKG_VERSION"),
            log_path.display()
        ));
        None
    };

    if scan_only {
        if blocker.is_some() {
            return ExitCode::from(2);
        }
        return if checks::scan().is_empty() {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(1)
        };
    }
    match app::run(blocker) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log::write(&format!("Không mở được cửa sổ: {e}"));
            ExitCode::from(2)
        }
    }
}

/// autofix.log cạnh file .exe.
fn log_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("autofix.log")))
        .unwrap_or_else(|| PathBuf::from("autofix.log"))
}
