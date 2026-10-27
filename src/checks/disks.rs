//! Dung lượng trống, lỗi hệ thống file (NTFS) và sức khoẻ ổ cứng (SMART) qua WMI Storage.

use serde::Deserialize;

use super::{Check, Finding, Fix, Severity};
use crate::sys;

const SCRIPT: &str = r#"
$ns = 'root/Microsoft/Windows/Storage'
$volumes = @(Get-CimInstance -Namespace $ns -ClassName MSFT_Volume -Filter 'DriveType = 3' | ForEach-Object {
  [pscustomobject]@{ letter = [string]$_.DriveLetter; size = $_.Size; free = $_.SizeRemaining; health = $_.HealthStatus }
})
$disks = @(Get-CimInstance -Namespace $ns -ClassName MSFT_PhysicalDisk | ForEach-Object {
  [pscustomobject]@{ name = $_.FriendlyName; health = $_.HealthStatus }
})
ConvertTo-Json -Compress -Depth 4 -InputObject @{ volumes = $volumes; disks = $disks }
"#;

/// HealthStatus của MSFT_Volume / MSFT_PhysicalDisk: 0 Healthy, 1 Warning, 2 Unhealthy, 5 Unknown.
fn unhealthy(health: Option<u16>) -> bool {
    matches!(health, Some(1 | 2))
}

#[derive(Debug, Deserialize)]
struct Storage {
    #[serde(default)]
    volumes: Vec<Option<Volume>>,
    #[serde(default)]
    disks: Vec<Option<Disk>>,
}

#[derive(Debug, Deserialize)]
struct Volume {
    letter: Option<String>,
    size: Option<u64>,
    free: Option<u64>,
    health: Option<u16>,
}

#[derive(Debug, Deserialize)]
struct Disk {
    name: Option<String>,
    health: Option<u16>,
}

pub struct Disks;

impl Check for Disks {
    fn name(&self) -> &'static str {
        "Ổ đĩa (dung lượng, NTFS, SMART)"
    }

    fn run(&self) -> Result<Vec<Finding>, String> {
        let out = sys::powershell(SCRIPT)?;
        let json = out.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or("{}");
        let storage: Storage = serde_json::from_str(json).map_err(|e| format!("JSON không hợp lệ: {e}"))?;
        let system_drive = std::env::var("SystemDrive")
            .ok()
            .and_then(|d| d.chars().next())
            .unwrap_or('C');
        Ok(findings(&storage, system_drive))
    }
}

fn findings(storage: &Storage, system_drive: char) -> Vec<Finding> {
    let mut out = vec![];
    let bad_disks: Vec<&Disk> = storage.disks.iter().flatten().filter(|d| unhealthy(d.health)).collect();
    for disk in &bad_disks {
        out.push(
            Finding::new(
                Severity::Critical,
                format!(
                    "Ổ cứng \"{}\" báo sắp hỏng (SMART)",
                    disk.name.as_deref().unwrap_or("?")
                ),
            )
            .advice("Sao lưu dữ liệu NGAY và thay ổ. Không chạy chkdsk trên ổ đang hỏng."),
        );
    }

    for volume in storage.volumes.iter().flatten() {
        let Some(letter) = volume
            .letter
            .as_deref()
            .and_then(|l| l.chars().next())
            .filter(char::is_ascii_alphabetic)
        else {
            continue;
        };
        let letter = letter.to_ascii_uppercase();
        let (size, free) = (volume.size.unwrap_or(0), volume.free.unwrap_or(0));
        if size > 0 && free * 10 < size {
            let severity = if free * 20 < size {
                Severity::Critical
            } else {
                Severity::Warning
            };
            let gb = free as f64 / (1u64 << 30) as f64;
            let mut finding = Finding::new(
                severity,
                format!("Ổ {letter}: sắp đầy (còn {gb:.1} GB, {}%)", free * 100 / size),
            )
            .advice("Gỡ ứng dụng không dùng, dọn Downloads, chạy Disk Cleanup (cleanmgr).");
            if letter == system_drive.to_ascii_uppercase() {
                finding = finding.fix(Fix::CleanTemp);
            }
            out.push(finding);
        }
        if unhealthy(volume.health) {
            let finding = Finding::new(Severity::Critical, format!("Ổ {letter}: có lỗi hệ thống file (NTFS)"));
            // chkdsk đọc/ghi toàn ổ: làm ổ đang hỏng hỏng nhanh hơn, nên chỉ hẹn khi ổ cứng còn tốt.
            out.push(if bad_disks.is_empty() {
                finding
                    .advice("chkdsk sẽ chạy khi khởi động lại máy, có thể mất khá lâu.")
                    .fix(Fix::ScheduleChkdsk(letter))
            } else {
                finding.advice("Không hẹn chkdsk vì có ổ cứng đang hỏng; sao lưu dữ liệu trước.")
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Storage {
        serde_json::from_str(include_str!("../../fixtures/disks.json")).unwrap()
    }

    #[test]
    fn low_space_and_ntfs_errors() {
        let found = findings(&fixture(), 'C');
        let titles: Vec<&str> = found.iter().map(|f| f.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "Ổ C: sắp đầy (còn 10.0 GB, 4%)",
                "Ổ D: có lỗi hệ thống file (NTFS)",
                "Ổ E: sắp đầy (còn 3.7 GB, 4%)",
            ]
        );
        assert_eq!(found[0].severity, Severity::Critical);
        assert_eq!(found[0].fixes, vec![Fix::CleanTemp]);
        assert_eq!(found[1].fixes, vec![Fix::ScheduleChkdsk('D')]);
        assert!(found[2].fixes.is_empty(), "chỉ dọn file tạm khi ổ hệ thống đầy");
    }

    #[test]
    fn failing_disk_blocks_chkdsk() {
        let mut storage = fixture();
        storage.disks[1].as_mut().unwrap().health = Some(2);
        let found = findings(&storage, 'C');
        assert!(found[0].title.contains("WDC WD10EZEX"));
        assert_eq!(found[0].severity, Severity::Critical);
        assert!(found
            .iter()
            .all(|f| !f.fixes.iter().any(|x| matches!(x, Fix::ScheduleChkdsk(_)))));
    }

    #[test]
    fn healthy_machine_has_no_findings() {
        let storage: Storage = serde_json::from_str(
            r#"{"volumes":[{"letter":"C","size":100,"free":50,"health":0}],"disks":[{"name":"x","health":5}]}"#,
        )
        .unwrap();
        assert!(findings(&storage, 'C').is_empty());
        let empty: Storage = serde_json::from_str(r#"{"volumes":[null],"disks":[]}"#).unwrap();
        assert!(findings(&empty, 'C').is_empty());
    }
}
