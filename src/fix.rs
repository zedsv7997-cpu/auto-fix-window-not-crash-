//! Thực hiện các bước sửa. Không bước nào đụng tới boot, driver hay ghi registry.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::checks::Fix;
use crate::{log, sys};

/// Tách các bước bị chặn khi Windows đang chờ khởi động lại. Trả về (chạy, bỏ qua).
pub fn plan(fixes: BTreeSet<Fix>, reboot_pending: bool) -> (Vec<Fix>, Vec<Fix>) {
    fixes
        .into_iter()
        .partition(|f| !(reboot_pending && f.blocked_by_pending_reboot()))
}

/// Kiểm tra lại sau khi tạo: Checkpoint-Computer có thể không báo lỗi khi System Restore đang tắt.
const RESTORE_POINT_SCRIPT: &str = r#"
try {
  Checkpoint-Computer -Description 'auto-fix-window' -RestorePointType MODIFY_SETTINGS -ErrorAction Stop -WarningAction SilentlyContinue -WarningVariable w
  $since = (Get-Date).AddHours(-24)
  $recent = @(Get-ComputerRestorePoint -ErrorAction Stop |
    Where-Object { [Management.ManagementDateTimeConverter]::ToDateTime($_.CreationTime) -gt $since })
  if ($recent.Count -eq 0) { 'FAILED không thấy điểm khôi phục nào (System Restore đang tắt?)' }
  elseif ($w) { 'OK EXISTS' } else { 'OK' }
} catch { ('FAILED ' + $_.Exception.Message) -replace '\s+', ' ' }
"#;

pub fn create_restore_point() -> Result<String, String> {
    let status = sys::parse_status(&sys::powershell(RESTORE_POINT_SCRIPT)?)?;
    Ok(if status == "EXISTS" {
        "Windows chỉ cho tạo 1 điểm khôi phục mỗi 24 giờ; dùng điểm đã tạo trong 24 giờ qua.".into()
    } else {
        "Đã tạo điểm khôi phục hệ thống.".into()
    })
}

/// Chỉ đổi tên SoftwareDistribution (không xoá) và luôn bật lại các dịch vụ đã dừng.
const RESET_UPDATE_SCRIPT: &str = r#"
$stopped = @()
try {
  foreach ($name in 'wuauserv', 'bits', 'dosvc') {
    $s = Get-Service -Name $name -ErrorAction SilentlyContinue
    if ($s -and $s.Status -ne 'Stopped') { Stop-Service -Name $name -Force -ErrorAction SilentlyContinue; $stopped += $name }
  }
  $backup = 'SoftwareDistribution.bak-' + (Get-Date -Format 'yyyyMMddHHmmss')
  Rename-Item -Path (Join-Path $env:SystemRoot 'SoftwareDistribution') -NewName $backup -ErrorAction Stop
  'OK ' + (Join-Path $env:SystemRoot $backup)
} catch { ('FAILED ' + $_.Exception.Message) -replace '\s+', ' ' }
finally { foreach ($name in $stopped) { Start-Service -Name $name -ErrorAction SilentlyContinue } }
"#;

/// Chạy lần lượt các bước đã được người dùng chọn, ghi log từng bước.
pub fn run_all(todo: &[Fix]) {
    let mut failed = 0;
    for fix in todo {
        log::say(&format!("-> {}", fix.describe()));
        match apply(fix) {
            Ok(msg) => log::say(&format!("   OK: {msg}")),
            Err(e) => {
                failed += 1;
                log::say(&format!("   LỖI: {e}"));
            }
        }
    }
    log::say(&format!(
        "Sửa xong: {} bước thành công, {failed} bước lỗi.",
        todo.len() - failed
    ));
    if todo
        .iter()
        .any(|f| matches!(f, Fix::ScheduleChkdsk(_) | Fix::SfcScannow | Fix::RestoreHealth))
    {
        log::say("Hãy khởi động lại máy, rồi bấm Quét để kiểm tra lại.");
    } else {
        log::say("Bấm Quét để kiểm tra lại.");
    }
}

fn apply(fix: &Fix) -> Result<String, String> {
    match fix {
        Fix::StartService(name) => {
            let script = format!(
                "try {{ Start-Service -Name {} -ErrorAction Stop; 'OK' }} catch {{ ('FAILED ' + $_.Exception.Message) -replace '\\s+', ' ' }}",
                sys::ps_quote(name)
            );
            sys::parse_status(&sys::powershell(&script)?).map(|_| "Dịch vụ đã chạy.".into())
        }
        Fix::CleanTemp => {
            let cutoff = SystemTime::now() - Duration::from_secs(7 * 24 * 3600);
            let (files, bytes) = temp_dirs().iter().fold((0, 0), |acc, dir| {
                let (f, b) = clean_temp(dir, cutoff);
                (acc.0 + f, acc.1 + b)
            });
            Ok(format!("Đã xoá {files} file, giải phóng {} MB.", bytes >> 20))
        }
        Fix::RestoreHealth => {
            let out = sys::run("dism.exe", &["/Online", "/Cleanup-Image", "/RestoreHealth"])?;
            log::write(&format!("Output DISM:\n{}", out.stdout.trim()));
            match out.code {
                Some(0) => Ok(sys::last_line(&out.stdout).to_string()),
                code => Err(format!("DISM thoát với mã {code:?}: {}", sys::last_line(&out.stdout))),
            }
        }
        Fix::SfcScannow => {
            let out = sys::run("sfc.exe", &["/scannow"])?;
            log::write(&format!("Output sfc /scannow:\n{}", out.stdout.trim()));
            Ok(format!("sfc chạy xong: {}", sys::last_line(&out.stdout)))
        }
        Fix::ResetWindowsUpdate => sys::parse_status(&sys::powershell(RESET_UPDATE_SCRIPT)?)
            .map(|backup| format!("Đã đặt lại; thư mục cũ đổi tên thành {backup}.")),
        Fix::ScheduleChkdsk(letter) => {
            // Đặt "dirty bit": Windows tự chạy chkdsk lúc khởi động, không gỡ ổ (dismount) khi đang dùng.
            let out = sys::run("fsutil.exe", &["dirty", "set", &format!("{letter}:")])?;
            match out.code {
                Some(0) => Ok("Khởi động lại máy để chkdsk chạy.".into()),
                code => Err(format!("fsutil thoát với mã {code:?}: {}", out.stdout.trim())),
            }
        }
    }
}

fn temp_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![std::env::temp_dir()];
    if let Some(root) = std::env::var_os("SystemRoot") {
        dirs.push(PathBuf::from(root).join("Temp"));
    }
    dirs.dedup();
    dirs
}

/// Xoá file cũ hơn `cutoff` trong thư mục tạm. Bỏ qua file đang dùng, symlink/junction,
/// và từ chối mọi thư mục không tên "Temp"/"tmp" (phòng biến TEMP bị đặt sai thành C:\).
fn clean_temp(dir: &Path, cutoff: SystemTime) -> (u64, u64) {
    let is_temp = dir
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.eq_ignore_ascii_case("temp") || n.eq_ignore_ascii_case("tmp"));
    let mut stats = (0, 0);
    if is_temp {
        clean_dir(dir, cutoff, &mut stats);
    }
    stats
}

fn clean_dir(dir: &Path, cutoff: SystemTime, stats: &mut (u64, u64)) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        let old = meta.modified().is_ok_and(|t| t < cutoff);
        if is_link(&meta) {
            continue;
        }
        if meta.is_dir() {
            clean_dir(&path, cutoff, stats);
            if old {
                let _ = fs::remove_dir(&path); // chỉ xoá được khi đã rỗng
            }
        } else if old && fs::remove_file(&path).is_ok() {
            stats.0 += 1;
            stats.1 += meta.len();
        }
    }
}

fn is_link(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    meta.file_type().is_symlink()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

    #[test]
    fn pending_reboot_blocks_system_repairs_only() {
        let all: BTreeSet<Fix> = [
            Fix::StartService("Spooler".into()),
            Fix::CleanTemp,
            Fix::RestoreHealth,
            Fix::SfcScannow,
            Fix::ResetWindowsUpdate,
            Fix::ScheduleChkdsk('D'),
        ]
        .into_iter()
        .collect();
        let (run, skipped) = plan(all.clone(), true);
        assert_eq!(
            run,
            vec![
                Fix::StartService("Spooler".into()),
                Fix::CleanTemp,
                Fix::ScheduleChkdsk('D')
            ]
        );
        assert_eq!(
            skipped,
            vec![Fix::RestoreHealth, Fix::SfcScannow, Fix::ResetWindowsUpdate]
        );
        let (run, skipped) = plan(all, false);
        assert_eq!(run.len(), 6);
        assert!(skipped.is_empty());
    }

    fn old_file(path: &Path, age: Duration) {
        let file = File::create(path).unwrap();
        file.set_len(1024).unwrap();
        file.set_modified(SystemTime::now() - age).unwrap();
    }

    #[test]
    fn clean_temp_deletes_only_old_files() {
        let root = std::env::temp_dir().join(format!("autofix-test-{}", std::process::id()));
        let temp = root.join("Temp");
        fs::create_dir_all(temp.join("sub")).unwrap();
        let week = Duration::from_secs(8 * 24 * 3600);
        old_file(&temp.join("old.tmp"), week);
        old_file(&temp.join("sub").join("old.log"), week);
        old_file(&temp.join("new.tmp"), Duration::ZERO);
        let cutoff = SystemTime::now() - Duration::from_secs(7 * 24 * 3600);

        assert_eq!(clean_temp(&temp, cutoff), (2, 2048));
        assert!(!temp.join("old.tmp").exists());
        assert!(!temp.join("sub").join("old.log").exists());
        assert!(temp.join("new.tmp").exists());
        assert!(temp.join("sub").exists(), "thư mục mới tạo không bị xoá");

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn clean_temp_refuses_non_temp_dirs() {
        let root = std::env::temp_dir().join(format!("autofix-guard-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        old_file(&root.join("important.doc"), Duration::from_secs(30 * 24 * 3600));
        assert_eq!(clean_temp(&root, SystemTime::now()), (0, 0));
        assert!(root.join("important.doc").exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn clean_temp_does_not_follow_symlinks() {
        let root = std::env::temp_dir().join(format!("autofix-link-{}", std::process::id()));
        let (temp, outside) = (root.join("tmp"), root.join("outside"));
        fs::create_dir_all(&temp).unwrap();
        fs::create_dir_all(&outside).unwrap();
        old_file(&outside.join("keep.txt"), Duration::from_secs(30 * 24 * 3600));
        std::os::unix::fs::symlink(&outside, temp.join("link")).unwrap();
        assert_eq!(clean_temp(&temp, SystemTime::now()), (0, 0));
        assert!(outside.join("keep.txt").exists());
        fs::remove_dir_all(&root).unwrap();
    }
}
