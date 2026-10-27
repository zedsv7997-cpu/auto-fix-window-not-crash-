//! Chạy công cụ hệ thống, gọi PowerShell, đọc output và hỏi xác nhận.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::log;

pub struct Output {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Đường dẫn tuyệt đối trong System32, để không chạy nhầm file .exe giả mạo nằm trong PATH
/// hoặc cùng thư mục (chương trình chạy với quyền Admin).
fn system_tool(rel: &str) -> PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    PathBuf::from(root).join("System32").join(rel)
}

/// Chạy lệnh và thu output.
pub fn run(tool: &str, args: &[&str]) -> Result<Output, String> {
    let out = Command::new(system_tool(tool))
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("không chạy được {tool}: {e}"))?;
    let out = Output {
        code: out.status.code(),
        stdout: decode(&out.stdout),
        stderr: decode(&out.stderr),
    };
    log::write(&format!("$ {tool} {} -> mã {:?}", args.join(" "), out.code));
    Ok(out)
}

/// Chạy lệnh và để output hiện thẳng ra màn hình (cho lệnh lâu như DISM, sfc).
pub fn run_visible(tool: &str, args: &[&str]) -> Result<Option<i32>, String> {
    let status = Command::new(system_tool(tool))
        .args(args)
        .status()
        .map_err(|e| format!("không chạy được {tool}: {e}"))?;
    log::write(&format!("$ {tool} {} -> mã {:?}", args.join(" "), status.code()));
    Ok(status.code())
}

/// Chạy script PowerShell (truyền qua -EncodedCommand để không lỗi dấu nháy), trả về stdout UTF-8.
pub fn powershell(script: &str) -> Result<String, String> {
    let full = format!(
        "$ProgressPreference = 'SilentlyContinue'\n$WarningPreference = 'SilentlyContinue'\n\
         [Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false\n\
         Remove-TypeData System.Array -ErrorAction SilentlyContinue\n{script}"
    );
    let utf16: Vec<u8> = full.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let out = run(
        "WindowsPowerShell\\v1.0\\powershell.exe",
        &["-NoProfile", "-NonInteractive", "-EncodedCommand", &base64(&utf16)],
    )?;
    if out.code != Some(0) {
        return Err(format!("PowerShell lỗi (mã {:?}): {}", out.code, out.stderr.trim()));
    }
    Ok(out.stdout)
}

/// Đặt chuỗi vào nháy đơn của PowerShell.
pub fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Đọc dòng kết quả cuối của script dạng `OK [chi tiết]` hoặc `FAILED <lý do>`.
pub fn parse_status(out: &str) -> Result<String, String> {
    let last = out.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or("");
    match last.split_once(' ').unwrap_or((last, "")) {
        ("OK", rest) => Ok(rest.trim().to_string()),
        ("FAILED", rest) => Err(rest.trim().to_string()),
        _ => Err(format!("kết quả lạ: {last}")),
    }
}

/// Giải mã output của lệnh Windows: UTF-16LE (sfc, có/không BOM) hoặc UTF-8.
pub fn decode(bytes: &[u8]) -> String {
    let odd_zeros = bytes.iter().skip(1).step_by(2).filter(|&&b| b == 0).count();
    let utf16 = bytes.starts_with(&[0xFF, 0xFE]) || (bytes.len() >= 4 && odd_zeros * 4 > bytes.len());
    let text = if utf16 {
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&c| u16::from_le_bytes(c))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    };
    text.trim_start_matches('\u{feff}').replace(['\0', '\r'], "")
}

fn base64(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let byte = |i: usize| u32::from(chunk.get(i).copied().unwrap_or(0));
        let n = byte(0) << 16 | byte(1) << 8 | byte(2);
        for i in 0..4 {
            if i <= chunk.len() {
                s.push(TABLE[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

/// Hỏi có/không; mặc định là KHÔNG.
pub fn confirm(question: &str) -> bool {
    print!("{question} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    let yes = std::io::stdin().read_line(&mut line).is_ok() && is_yes(&line);
    log::write(&format!("{question} -> {}", if yes { "có" } else { "không" }));
    yes
}

fn is_yes(answer: &str) -> bool {
    matches!(answer.trim().to_lowercase().as_str(), "y" | "yes" | "c" | "co" | "có")
}

#[cfg(windows)]
pub fn is_admin() -> bool {
    // SAFETY: hàm Win32 không tham số, chỉ đọc token của tiến trình hiện tại.
    unsafe { windows_sys::Win32::UI::Shell::IsUserAnAdmin() != 0 }
}

#[cfg(not(windows))]
pub fn is_admin() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(s: &str, bom: bool) -> Vec<u8> {
        let mut bytes = if bom { vec![0xFF, 0xFE] } else { vec![] };
        bytes.extend(s.encode_utf16().flat_map(u16::to_le_bytes));
        bytes
    }

    #[test]
    fn decodes_utf16_with_and_without_bom() {
        let text = "Windows Resource Protection\r\r\nđã xong\r\n";
        assert_eq!(decode(&utf16(text, false)), "Windows Resource Protection\nđã xong\n");
        assert_eq!(decode(&utf16(text, true)), "Windows Resource Protection\nđã xong\n");
    }

    #[test]
    fn decodes_utf8_and_strips_bom() {
        assert_eq!(decode("\u{feff}[{\"a\":1}]\r\n".as_bytes()), "[{\"a\":1}]\n");
        assert_eq!(decode(b"Healthy"), "Healthy");
        assert_eq!(decode(b""), "");
    }

    #[test]
    fn base64_matches_rfc4648_vectors() {
        let cases = [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("foobar", "Zm9vYmFy"),
        ];
        for (input, expected) in cases {
            assert_eq!(base64(input.as_bytes()), expected);
        }
    }

    #[test]
    fn ps_quote_escapes_single_quotes() {
        assert_eq!(ps_quote("wuauserv"), "'wuauserv'");
        assert_eq!(ps_quote("a'b"), "'a''b'");
    }

    #[test]
    fn parse_status_reads_last_line() {
        assert_eq!(parse_status("noise\nOK\n\n"), Ok(String::new()));
        assert_eq!(parse_status("OK EXISTS\n"), Ok("EXISTS".into()));
        assert_eq!(
            parse_status("FAILED Access is denied\n"),
            Err("Access is denied".into())
        );
        assert!(parse_status("").is_err());
        assert!(parse_status("OKAY").is_err());
    }

    #[test]
    fn yes_answers() {
        for a in ["y", "Y\n", "yes", "c", "có\r\n", " Co "] {
            assert!(is_yes(a), "{a:?}");
        }
        for a in ["", "n", "no", "k", "yy"] {
            assert!(!is_yes(a), "{a:?}");
        }
    }
}
