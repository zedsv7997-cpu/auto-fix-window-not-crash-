//! File hệ thống Windows: kho thành phần (DISM CheckHealth) và file hệ thống (sfc /verifyonly).

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use super::{Check, Finding, Fix, Severity};
use crate::{log, sys};

/// Trả về tên enum (không phụ thuộc ngôn ngữ Windows): Healthy, Repairable, NonRepairable.
const DISM_SCRIPT: &str = "[string](Repair-WindowsImage -Online -CheckHealth -ErrorAction Stop).ImageHealthState";

pub struct ImageHealth;

impl Check for ImageHealth {
    fn name(&self) -> &'static str {
        "Kho thành phần Windows (DISM CheckHealth)"
    }

    fn run(&self) -> Result<Vec<Finding>, String> {
        dism_findings(&sys::powershell(DISM_SCRIPT)?)
    }
}

fn dism_findings(output: &str) -> Result<Vec<Finding>, String> {
    let state = output.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or("");
    Ok(match state {
        "Healthy" => vec![],
        "Repairable" => vec![
            Finding::new(Severity::Critical, "Kho thành phần Windows bị hỏng (sửa được)")
                .fix(Fix::RestoreHealth)
                .fix(Fix::SfcScannow),
        ],
        "NonRepairable" => vec![Finding::new(
            Severity::Critical,
            "Kho thành phần Windows hỏng nặng, không tự sửa được",
        )
        .advice("Cài đè Windows giữ nguyên dữ liệu (in-place upgrade) bằng bộ cài Windows chính thức.")],
        _ => return Err(format!("kết quả DISM lạ: {state:?}")),
    })
}

#[derive(Debug, PartialEq)]
enum SfcResult {
    Clean,
    Corrupt,
    RebootPending,
    Unknown,
}

pub struct Sfc;

impl Check for Sfc {
    fn name(&self) -> &'static str {
        "File hệ thống (sfc /verifyonly, có thể mất 10–20 phút)"
    }

    fn run(&self) -> Result<Vec<Finding>, String> {
        let cbs = cbs_log();
        let start = std::fs::metadata(&cbs).map(|m| m.len()).unwrap_or(0);
        let out = sys::run("sfc.exe", &["/verifyonly"])?;
        log::write(&format!("Output sfc /verifyonly:\n{}", out.stdout.trim()));
        let mut result = parse_sfc_output(&out.stdout);
        if result == SfcResult::Unknown {
            // Windows không phải tiếng Anh: đọc phần CBS.log mà sfc vừa ghi (luôn bằng tiếng Anh).
            result = parse_cbs(&read_from(&cbs, start).unwrap_or_default());
        }
        Ok(sfc_findings(result))
    }
}

fn cbs_log() -> PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    PathBuf::from(root).join("Logs").join("CBS").join("CBS.log")
}

/// Đọc phần log ghi thêm từ vị trí `start` (tối đa 16 MB cuối). Nếu log đã bị xoay vòng thì đọc từ đầu.
fn read_from(path: &Path, start: u64) -> std::io::Result<String> {
    const MAX: u64 = 16 << 20;
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    let start = if start > len { 0 } else { start };
    file.seek(SeekFrom::Start(start.max(len.saturating_sub(MAX))))?;
    let mut bytes = vec![];
    file.read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn parse_sfc_output(text: &str) -> SfcResult {
    let text = text.to_lowercase();
    if text.contains("did not find any integrity violations") {
        SfcResult::Clean
    } else if text.contains("found integrity violations") || text.contains("found corrupt files") {
        SfcResult::Corrupt
    } else if text.contains("repair pending") {
        SfcResult::RebootPending
    } else {
        SfcResult::Unknown
    }
}

fn parse_cbs(log: &str) -> SfcResult {
    let mut verified = false;
    for line in log.lines() {
        let lower = line.to_lowercase();
        if lower.contains("do not match actual file") {
            return SfcResult::Corrupt;
        }
        if let Some((_, sr)) = lower.split_once("[sr]") {
            if sr.contains("cannot") || sr.contains("corrupt") {
                return SfcResult::Corrupt;
            }
            verified |= sr.contains("verify complete");
        }
    }
    if verified {
        SfcResult::Clean
    } else {
        SfcResult::Unknown
    }
}

fn sfc_findings(result: SfcResult) -> Vec<Finding> {
    match result {
        SfcResult::Clean => vec![],
        SfcResult::Corrupt => vec![
            Finding::new(Severity::Critical, "sfc phát hiện file hệ thống Windows bị hỏng")
                .fix(Fix::RestoreHealth)
                .fix(Fix::SfcScannow),
        ],
        SfcResult::RebootPending => vec![
            Finding::new(Severity::Info, "sfc: có bản sửa đang chờ khởi động lại máy")
                .advice("Khởi động lại máy rồi quét lại."),
        ],
        SfcResult::Unknown => vec![
            Finding::new(Severity::Warning, "Không đọc được kết quả sfc /verifyonly")
                .advice("Xem output trong file log và C:\\Windows\\Logs\\CBS\\CBS.log.")
                .fix(Fix::SfcScannow),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn as_utf16(text: &str) -> Vec<u8> {
        text.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    #[test]
    fn dism_states() {
        assert!(dism_findings("Healthy\r\n").unwrap().is_empty());
        assert_eq!(
            dism_findings("Repairable").unwrap()[0].fixes,
            vec![Fix::RestoreHealth, Fix::SfcScannow]
        );
        assert!(dism_findings("NonRepairable").unwrap()[0].fixes.is_empty());
        assert!(dism_findings("").is_err());
    }

    #[test]
    fn sfc_english_output_from_utf16() {
        let parse = |s: &str| parse_sfc_output(&sys::decode(&as_utf16(s)));
        assert_eq!(parse(include_str!("../../fixtures/sfc_clean.txt")), SfcResult::Clean);
        assert_eq!(
            parse(include_str!("../../fixtures/sfc_violations.txt")),
            SfcResult::Corrupt
        );
        assert_eq!(
            parse("There is a system repair pending which requires reboot to complete."),
            SfcResult::RebootPending
        );
        assert_eq!(parse("Bảo vệ Tài nguyên Windows ..."), SfcResult::Unknown);
    }

    #[test]
    fn sfc_cbs_log_fallback() {
        assert_eq!(
            parse_cbs(include_str!("../../fixtures/cbs_corrupt.log")),
            SfcResult::Corrupt
        );
        assert_eq!(
            parse_cbs(include_str!("../../fixtures/cbs_clean.log")),
            SfcResult::Clean
        );
        assert_eq!(parse_cbs(""), SfcResult::Unknown);
    }

    #[test]
    fn read_from_only_returns_new_part() {
        let path = std::env::temp_dir().join(format!("autofix-cbs-{}.log", std::process::id()));
        std::fs::write(&path, "old\nnew\n").unwrap();
        assert_eq!(read_from(&path, 4).unwrap(), "new\n");
        assert_eq!(
            read_from(&path, 100).unwrap(),
            "old\nnew\n",
            "log bị xoay vòng thì đọc lại từ đầu"
        );
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn sfc_results_map_to_fixes() {
        assert!(sfc_findings(SfcResult::Clean).is_empty());
        assert_eq!(
            sfc_findings(SfcResult::Corrupt)[0].fixes,
            vec![Fix::RestoreHealth, Fix::SfcScannow]
        );
        assert!(sfc_findings(SfcResult::RebootPending)[0].fixes.is_empty());
        assert_eq!(sfc_findings(SfcResult::Unknown)[0].fixes, vec![Fix::SfcScannow]);
    }
}
