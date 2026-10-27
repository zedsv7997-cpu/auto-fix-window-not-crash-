//! Ghi mọi thao tác ra file log (kèm thời gian) và in ra màn hình.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

static FILE: Mutex<Option<File>> = Mutex::new(None);

pub fn init(path: &Path) -> std::io::Result<()> {
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    *FILE.lock().unwrap() = Some(file);
    Ok(())
}

/// Chỉ ghi vào file log.
pub fn write(msg: &str) {
    if let Some(file) = FILE.lock().unwrap().as_mut() {
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        for line in msg.lines() {
            let _ = writeln!(file, "[{now}] {line}");
        }
    }
}

/// In ra màn hình và ghi vào file log.
pub fn say(msg: &str) {
    println!("{msg}");
    write(msg);
}
