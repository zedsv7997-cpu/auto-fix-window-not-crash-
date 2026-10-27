use super::{Check, Finding, Severity};
use crate::sys;

const KEYS: [&str; 2] = [
    r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Component Based Servicing\RebootPending",
    r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\WindowsUpdate\Auto Update\RebootRequired",
];

/// Windows đang chờ khởi động lại (chỉ đọc registry, `reg query` trả mã 0 khi khoá tồn tại).
pub fn reboot_pending() -> bool {
    KEYS.iter()
        .any(|key| matches!(sys::run("reg.exe", &["query", key]), Ok(out) if out.code == Some(0)))
}

pub struct Reboot;

impl Check for Reboot {
    fn name(&self) -> &'static str {
        "Khởi động lại đang chờ"
    }

    fn run(&self) -> Result<Vec<Finding>, String> {
        if !reboot_pending() {
            return Ok(vec![]);
        }
        Ok(vec![Finding::new(
            Severity::Info,
            "Windows đang chờ khởi động lại để hoàn tất cập nhật",
        )
        .advice(
            "Khởi động lại máy trước; DISM, sfc và đặt lại Windows Update sẽ bị bỏ qua cho tới khi khởi động lại.",
        )])
    }
}
