//! Các bước kiểm tra. Mỗi bước trả về danh sách vấn đề (`Finding`) kèm cách sửa (`Fix`).

mod disks;
mod events;
mod integrity;
mod reboot;
mod services;

pub use reboot::reboot_pending;

use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::log;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Info,
    Warning,
    Critical,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Severity::Info => "THÔNG TIN",
            Severity::Warning => "CẢNH BÁO",
            Severity::Critical => "NGHIÊM TRỌNG",
        }
    }
}

/// Cách sửa tự động. Thứ tự khai báo là thứ tự chạy (DISM trước sfc).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Fix {
    StartService(String),
    CleanTemp,
    RestoreHealth,
    SfcScannow,
    ResetWindowsUpdate,
    ScheduleChkdsk(char),
}

impl Fix {
    pub fn describe(&self) -> String {
        match self {
            Fix::StartService(name) => format!("Khởi động lại dịch vụ {name}"),
            Fix::CleanTemp => "Xoá file tạm cũ hơn 7 ngày (%TEMP% và C:\\Windows\\Temp)".into(),
            Fix::RestoreHealth => {
                "Sửa kho thành phần Windows: DISM /Online /Cleanup-Image /RestoreHealth (10–30 phút)".into()
            }
            Fix::SfcScannow => "Sửa file hệ thống: sfc /scannow (10–20 phút)".into(),
            Fix::ResetWindowsUpdate => {
                "Đặt lại Windows Update (dừng dịch vụ, đổi tên thư mục SoftwareDistribution, bật lại)".into()
            }
            Fix::ScheduleChkdsk(letter) => format!("Hẹn chkdsk ổ {letter}: ở lần khởi động sau"),
        }
    }

    /// Bước có rủi ro: luôn hỏi xác nhận trước khi chạy.
    pub fn risky(&self) -> bool {
        !matches!(self, Fix::StartService(_) | Fix::CleanTemp)
    }

    /// Không chạy khi Windows đang chờ khởi động lại (dễ hỏng thêm kho cập nhật).
    pub fn blocked_by_pending_reboot(&self) -> bool {
        matches!(self, Fix::RestoreHealth | Fix::SfcScannow | Fix::ResetWindowsUpdate)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub severity: Severity,
    pub title: String,
    pub advice: Option<String>,
    pub fixes: Vec<Fix>,
}

impl Finding {
    fn new(severity: Severity, title: impl Into<String>) -> Self {
        Finding {
            severity,
            title: title.into(),
            advice: None,
            fixes: vec![],
        }
    }

    fn advice(mut self, advice: impl Into<String>) -> Self {
        self.advice = Some(advice.into());
        self
    }

    fn fix(mut self, fix: Fix) -> Self {
        self.fixes.push(fix);
        self
    }
}

pub trait Check {
    fn name(&self) -> &'static str;
    fn run(&self) -> Result<Vec<Finding>, String>;
}

/// Các bước kiểm tra, nhanh trước, chậm (sfc) sau.
fn all() -> Vec<Box<dyn Check>> {
    vec![
        Box::new(reboot::Reboot),
        Box::new(events::Events),
        Box::new(services::Services),
        Box::new(disks::Disks),
        Box::new(integrity::ImageHealth),
        Box::new(integrity::Sfc),
    ]
}

/// Chạy mọi bước kiểm tra (chỉ đọc), ghi tiến độ và kết quả vào log. Vấn đề nặng nhất lên đầu.
pub fn scan() -> Vec<Finding> {
    let mut findings = vec![];
    for check in all() {
        log::say(&format!("Đang kiểm tra: {}...", check.name()));
        match check.run() {
            Ok(found) => findings.extend(found),
            Err(e) => log::say(&format!("  ! Không kiểm tra được: {e}")),
        }
    }
    findings.sort_by_key(|f| std::cmp::Reverse(f.severity));
    log::say(&format!("Quét xong: {} vấn đề.", findings.len()));
    for f in &findings {
        log::write(&format!("[{}] {}", f.severity.label(), f.title));
        if let Some(advice) = &f.advice {
            log::write(&format!("    Gợi ý: {advice}"));
        }
        for fix in &f.fixes {
            log::write(&format!("    Sửa được: {}", fix.describe()));
        }
    }
    findings
}

/// Đọc output của `ConvertTo-Json -Compress` (dòng cuối): rỗng, một object hoặc một mảng.
fn parse_list<T: DeserializeOwned>(output: &str) -> Result<Vec<T>, String> {
    let json = output.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or("");
    if json.is_empty() {
        return Ok(vec![]);
    }
    let value: Value = serde_json::from_str(json).map_err(|e| format!("JSON không hợp lệ: {e}"))?;
    let items = match value {
        Value::Array(items) => items,
        Value::Null => vec![],
        item => vec![item],
    };
    items
        .into_iter()
        .filter(|v| !v.is_null())
        .map(|v| serde_json::from_value(v).map_err(|e| format!("JSON sai định dạng: {e}")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize, Debug, PartialEq)]
    struct Item {
        a: u32,
    }

    #[test]
    fn parse_list_accepts_powershell_shapes() {
        assert_eq!(parse_list::<Item>("").unwrap(), vec![]);
        assert_eq!(parse_list::<Item>("[]").unwrap(), vec![]);
        assert_eq!(parse_list::<Item>("null").unwrap(), vec![]);
        assert_eq!(parse_list::<Item>("{\"a\":1}").unwrap(), vec![Item { a: 1 }]);
        assert_eq!(
            parse_list::<Item>("WARNING: x\n[{\"a\":1},null,{\"a\":2}]\n").unwrap(),
            vec![Item { a: 1 }, Item { a: 2 }]
        );
        assert!(parse_list::<Item>("not json").is_err());
    }

    #[test]
    fn fixes_run_dism_before_sfc_and_dedupe() {
        let set: std::collections::BTreeSet<Fix> =
            [Fix::SfcScannow, Fix::RestoreHealth, Fix::SfcScannow, Fix::CleanTemp]
                .into_iter()
                .collect();
        assert_eq!(
            set.into_iter().collect::<Vec<_>>(),
            vec![Fix::CleanTemp, Fix::RestoreHealth, Fix::SfcScannow]
        );
    }

    #[test]
    fn only_harmless_fixes_skip_confirmation() {
        assert!(!Fix::StartService("x".into()).risky());
        assert!(!Fix::CleanTemp.risky());
        assert!(Fix::RestoreHealth.risky());
        assert!(Fix::SfcScannow.risky());
        assert!(Fix::ResetWindowsUpdate.risky());
        assert!(Fix::ScheduleChkdsk('C').risky());
    }
}
