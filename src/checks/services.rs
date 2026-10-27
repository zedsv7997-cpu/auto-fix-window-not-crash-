//! Dịch vụ tự khởi động (Automatic) đang dừng vì lỗi.

use serde::Deserialize;

use super::{parse_list, Check, Finding, Fix, Severity};
use crate::sys;

const SCRIPT: &str = r#"
$services = @(Get-CimInstance -ClassName Win32_Service -Filter "StartMode='Auto' AND State='Stopped'" |
  Select-Object Name, DisplayName, ExitCode)
ConvertTo-Json -Compress -InputObject $services
"#;

/// ERROR_SERVICE_NEVER_STARTED: dịch vụ khởi động trễ/theo sự kiện chưa chạy, không phải lỗi.
const NEVER_STARTED: u32 = 1077;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Service {
    name: String,
    display_name: Option<String>,
    exit_code: Option<u32>,
}

pub struct Services;

impl Check for Services {
    fn name(&self) -> &'static str {
        "Dịch vụ Windows bị dừng"
    }

    fn run(&self) -> Result<Vec<Finding>, String> {
        Ok(findings(&parse_list(&sys::powershell(SCRIPT)?)?))
    }
}

/// Chỉ báo dịch vụ dừng kèm mã lỗi; dịch vụ tự dừng bình thường có mã 0.
fn findings(services: &[Service]) -> Vec<Finding> {
    services
        .iter()
        .filter_map(|s| {
            let code = s.exit_code.filter(|&c| c != 0 && c != NEVER_STARTED)?;
            let shown = s.display_name.as_deref().unwrap_or(&s.name);
            Some(
                Finding::new(
                    Severity::Warning,
                    format!("Dịch vụ \"{shown}\" ({}) bị dừng, mã lỗi {code}", s.name),
                )
                .fix(Fix::StartService(s.name.clone())),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_only_services_that_failed() {
        let services: Vec<Service> = parse_list(include_str!("../../fixtures/services.json")).unwrap();
        let found = findings(&services);
        assert_eq!(found.len(), 2);
        assert_eq!(
            found[0].title,
            "Dịch vụ \"Print Spooler\" (Spooler) bị dừng, mã lỗi 1067"
        );
        assert_eq!(found[0].fixes, vec![Fix::StartService("Spooler".into())]);
        assert_eq!(found[1].fixes, vec![Fix::StartService("BadSvc".into())]);
    }
}
