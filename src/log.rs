//! Ghi mọi thao tác ra file log (kèm thời gian) và giữ bản sao để hiện trong cửa sổ.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

static FILE: Mutex<Option<File>> = Mutex::new(None);
static LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Khoá mutex kể cả khi một luồng khác từng panic, để log không bao giờ làm sập chương trình.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn init(path: &Path) -> std::io::Result<()> {
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    *lock(&FILE) = Some(file);
    Ok(())
}

/// Chỉ ghi vào file log.
pub fn write(msg: &str) {
    if let Some(file) = lock(&FILE).as_mut() {
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        for line in msg.lines() {
            let _ = writeln!(file, "[{now}] {line}");
        }
    }
}

/// Hiện trong khung nhật ký của cửa sổ và ghi vào file log.
pub fn say(msg: &str) {
    lock(&LINES).extend(msg.lines().map(String::from));
    write(msg);
}

/// Các dòng đã `say`, để cửa sổ hiển thị.
pub fn lines() -> MutexGuard<'static, Vec<String>> {
    lock(&LINES)
}
