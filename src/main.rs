//! auto-fix-window: quét lỗi Windows thường gặp và sửa an toàn.
//! Mặc định chỉ quét và báo cáo; `--fix` mới sửa, bước rủi ro luôn hỏi trước.

mod checks;
mod fix;
mod log;
mod sys;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

use checks::{Finding, Fix};
use log::say;

#[derive(Parser)]
#[command(
    version,
    about = "Quét lỗi Windows và sửa an toàn. Mặc định chỉ quét; thêm --fix để sửa."
)]
struct Args {
    /// Sửa các lỗi tìm được (tạo điểm khôi phục trước, bước rủi ro sẽ hỏi xác nhận)
    #[arg(long)]
    fix: bool,
    /// File log (mặc định: autofix.log cạnh file .exe)
    #[arg(long, value_name = "FILE")]
    log: Option<PathBuf>,
}

fn main() -> ExitCode {
    let args = Args::parse();
    if !cfg!(windows) {
        eprintln!("Công cụ này chỉ chạy trên Windows.");
        return ExitCode::from(2);
    }
    if !sys::is_admin() {
        eprintln!(
            "Cần quyền Administrator: chuột phải PowerShell (hoặc CMD) → \"Run as administrator\", rồi chạy lại."
        );
        return ExitCode::from(2);
    }
    let log_path = args.log.unwrap_or_else(default_log_path);
    if let Err(e) = log::init(&log_path) {
        eprintln!("Không mở được file log {}: {e}", log_path.display());
        return ExitCode::from(2);
    }
    let mode = if args.fix { "QUÉT + SỬA" } else { "CHỈ QUÉT" };
    say(&format!(
        "auto-fix-window {} | chế độ: {mode} | log: {}",
        env!("CARGO_PKG_VERSION"),
        log_path.display()
    ));

    let findings = scan();
    report(&findings);
    if !args.fix {
        if findings.iter().any(|f| !f.fixes.is_empty()) {
            say("\nChạy lại với --fix để sửa (sẽ hỏi trước các bước rủi ro).");
        }
        return if findings.is_empty() {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(1)
        };
    }
    run_fixes(&findings)
}

fn default_log_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("autofix.log")))
        .unwrap_or_else(|| PathBuf::from("autofix.log"))
}

fn scan() -> Vec<Finding> {
    let mut findings = vec![];
    for check in checks::all() {
        say(&format!("Đang kiểm tra: {}...", check.name()));
        match check.run() {
            Ok(found) => findings.extend(found),
            Err(e) => say(&format!("  ! Không kiểm tra được: {e}")),
        }
    }
    findings.sort_by_key(|f| std::cmp::Reverse(f.severity));
    findings
}

fn report(findings: &[Finding]) {
    if findings.is_empty() {
        say("\nKhông phát hiện vấn đề nào.");
        return;
    }
    say(&format!("\n== Phát hiện {} vấn đề ==", findings.len()));
    for f in findings {
        say(&format!("[{}] {}", f.severity.label(), f.title));
        if let Some(advice) = &f.advice {
            say(&format!("    Gợi ý: {advice}"));
        }
        for fix in &f.fixes {
            say(&format!("    Sửa được: {}", fix.describe()));
        }
    }
}

fn run_fixes(findings: &[Finding]) -> ExitCode {
    let fixes: BTreeSet<Fix> = findings.iter().flat_map(|f| f.fixes.iter().cloned()).collect();
    if fixes.is_empty() {
        say("\nKhông có lỗi nào sửa tự động được.");
        return ExitCode::SUCCESS;
    }
    let (todo, blocked) = fix::plan(fixes, checks::reboot_pending());
    for f in &blocked {
        say(&format!("Bỏ qua (khởi động lại máy rồi chạy lại): {}", f.describe()));
    }

    // Hỏi hết các bước rủi ro trước, sau đó chạy liền một lượt.
    say("");
    let todo: Vec<Fix> = todo
        .into_iter()
        .filter(|f| !f.risky() || sys::confirm(&format!("Thực hiện: {}?", f.describe())))
        .collect();
    if todo.is_empty() {
        say("Không có bước nào được chọn, không sửa gì.");
        return ExitCode::SUCCESS;
    }

    say("\nĐang tạo điểm khôi phục hệ thống (System Restore)...");
    match fix::create_restore_point() {
        Ok(msg) => say(&format!("  {msg}")),
        Err(e) => {
            say(&format!("  Không tạo được điểm khôi phục: {e}"));
            if !sys::confirm("Vẫn sửa khi KHÔNG có điểm khôi phục?") {
                say("Đã dừng, không sửa gì.");
                return ExitCode::from(1);
            }
        }
    }

    let mut failed = 0;
    for f in &todo {
        say(&format!("\n-> {}", f.describe()));
        match fix::apply(f) {
            Ok(msg) => say(&format!("   OK: {msg}")),
            Err(e) => {
                failed += 1;
                say(&format!("   LỖI: {e}"));
            }
        }
    }

    say(&format!(
        "\nXong: {} bước thành công, {failed} bước lỗi.",
        todo.len() - failed
    ));
    if !blocked.is_empty()
        || todo
            .iter()
            .any(|f| matches!(f, Fix::ScheduleChkdsk(_) | Fix::SfcScannow))
    {
        say("Hãy khởi động lại máy để hoàn tất.");
    }
    if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}
