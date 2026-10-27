//! Crash/BSOD, tắt máy đột ngột, ứng dụng crash và lỗi Windows Update trong Event Log 7 ngày qua.

use std::collections::HashMap;

use serde::Deserialize;

use super::{parse_list, Check, Finding, Fix, Severity};
use crate::sys;

const SCRIPT: &str = r#"
$since = (Get-Date).AddDays(-7)
$filters = @(
  @{ LogName = 'System'; Id = 41, 1001; StartTime = $since },
  @{ LogName = 'System'; ProviderName = 'Microsoft-Windows-WindowsUpdateClient'; Id = 20; StartTime = $since },
  @{ LogName = 'Application'; Id = 1000, 1002; StartTime = $since }
)
$events = @(Get-WinEvent -FilterHashtable $filters -MaxEvents 500 -ErrorAction SilentlyContinue | ForEach-Object {
  [pscustomobject]@{
    id = $_.Id
    provider = $_.ProviderName
    time = $_.TimeCreated.ToString('yyyy-MM-dd HH:mm', [Globalization.CultureInfo]::InvariantCulture)
    detail = [string](@($_.Properties)[0].Value)
  }
})
ConvertTo-Json -Compress -InputObject $events
"#;

#[derive(Debug, Deserialize)]
struct Event {
    id: u32,
    provider: Option<String>,
    time: Option<String>,
    /// Tham số đầu của sự kiện: mã BugCheck, tên ứng dụng, mã lỗi Windows Update...
    detail: Option<String>,
}

impl Event {
    fn provider(&self) -> &str {
        self.provider.as_deref().unwrap_or("")
    }
    fn time(&self) -> &str {
        self.time.as_deref().unwrap_or("?")
    }
    fn detail(&self) -> &str {
        self.detail.as_deref().unwrap_or("").trim()
    }
}

pub struct Events;

impl Check for Events {
    fn name(&self) -> &'static str {
        "Event Log (crash, BSOD, Windows Update)"
    }

    fn run(&self) -> Result<Vec<Finding>, String> {
        Ok(findings(&parse_list(&sys::powershell(SCRIPT)?)?))
    }
}

/// (thời gian, chi tiết) của sự kiện mới nhất.
fn latest<'a>(list: &[&'a Event]) -> Option<(&'a str, &'a str)> {
    list.iter()
        .max_by(|a, b| a.time().cmp(b.time()))
        .map(|e| (e.time(), e.detail()))
}

fn findings(events: &[Event]) -> Vec<Finding> {
    let select = |id: u32, provider: &str| -> Vec<&Event> {
        events
            .iter()
            .filter(|e| e.id == id && e.provider().contains(provider))
            .collect()
    };
    let mut out = vec![];

    // Kernel-Power 41: mã BugCheck khác 0 là BSOD, bằng 0 là mất điện/treo/giữ nút nguồn.
    let (kp_bsod, power_loss): (Vec<&Event>, Vec<&Event>) = select(41, "Kernel-Power")
        .into_iter()
        .partition(|e| !matches!(e.detail(), "" | "0"));
    let mut bsod = select(1001, "WER-SystemErrorReporting");
    bsod.extend(select(1001, "BugCheck"));
    if bsod.is_empty() {
        bsod = kp_bsod;
    }
    if let Some((time, code)) = latest(&bsod) {
        out.push(
            Finding::new(
                Severity::Critical,
                format!(
                    "Máy bị màn hình xanh (BSOD) {} lần trong 7 ngày (gần nhất {time}, mã {code})",
                    bsod.len()
                ),
            )
            .advice("Nếu vẫn bị: cập nhật driver (nhất là card màn hình), kiểm tra RAM bằng mdsched.exe.")
            .fix(Fix::RestoreHealth)
            .fix(Fix::SfcScannow),
        );
    }
    if let Some((time, _)) = latest(&power_loss) {
        out.push(
            Finding::new(
                Severity::Warning,
                format!(
                    "Máy tắt đột ngột {} lần trong 7 ngày (gần nhất {time})",
                    power_loss.len()
                ),
            )
            .advice("Do mất điện, treo máy hoặc giữ nút nguồn. Kiểm tra nguồn/pin và nhiệt độ máy."),
        );
    }

    let update = select(20, "WindowsUpdateClient");
    if let Some((time, code)) = latest(&update) {
        out.push(
            Finding::new(
                Severity::Warning,
                format!(
                    "Windows Update cài lỗi {} lần trong 7 ngày (gần nhất {time}, mã {code})",
                    update.len()
                ),
            )
            .fix(Fix::ResetWindowsUpdate),
        );
    }

    let mut apps: HashMap<&str, usize> = HashMap::new();
    for e in select(1000, "Application Error")
        .into_iter()
        .chain(select(1002, "Application Hang"))
    {
        if !e.detail().is_empty() {
            *apps.entry(e.detail()).or_default() += 1;
        }
    }
    if !apps.is_empty() {
        let mut apps: Vec<_> = apps.into_iter().collect();
        apps.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        let list: Vec<String> = apps.iter().take(5).map(|(app, n)| format!("{app} ({n})")).collect();
        out.push(
            Finding::new(
                Severity::Warning,
                format!("Ứng dụng bị crash/treo trong 7 ngày: {}", list.join(", ")),
            )
            .advice("Cập nhật hoặc cài lại các ứng dụng này."),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_fixture_events() {
        let events: Vec<Event> = parse_list(include_str!("../../fixtures/events.json")).unwrap();
        let found = findings(&events);
        assert_eq!(found.len(), 4, "{found:#?}");

        assert_eq!(found[0].severity, Severity::Critical);
        assert!(found[0].title.contains("BSOD) 1 lần"), "{}", found[0].title);
        assert!(found[0].title.contains("0x0000009f"));
        assert_eq!(found[0].fixes, vec![Fix::RestoreHealth, Fix::SfcScannow]);

        assert!(found[1].title.contains("tắt đột ngột 1 lần"), "{}", found[1].title);
        assert!(found[1].fixes.is_empty());

        assert!(
            found[2].title.contains("Windows Update cài lỗi 2 lần"),
            "{}",
            found[2].title
        );
        assert!(found[2].title.contains("0x80070643"));
        assert_eq!(found[2].fixes, vec![Fix::ResetWindowsUpdate]);

        assert!(
            found[3].title.ends_with("chrome.exe (2), explorer.exe (1)"),
            "{}",
            found[3].title
        );
    }

    #[test]
    fn kernel_power_bugcheck_counts_as_bsod_when_no_1001() {
        let events: Vec<Event> = parse_list(
            r#"{"id":41,"provider":"Microsoft-Windows-Kernel-Power","time":"2026-10-25 21:14","detail":"159"}"#,
        )
        .unwrap();
        let found = findings(&events);
        assert_eq!(found.len(), 1);
        assert!(found[0].title.contains("BSOD) 1 lần"));
        assert!(found[0].title.contains("mã 159"));
    }

    #[test]
    fn no_events_no_findings() {
        assert!(findings(&parse_list::<Event>("[]").unwrap()).is_empty());
        assert!(findings(&parse_list::<Event>("").unwrap()).is_empty());
    }
}
