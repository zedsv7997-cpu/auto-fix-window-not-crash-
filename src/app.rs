//! Cửa sổ chính: nút Quét / Sửa, danh sách vấn đề, các bước sửa (tick chọn) và nhật ký.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use eframe::egui::{self, Color32, RichText};

use crate::checks::{self, Finding, Fix, Severity};
use crate::{fix, log, sys};

pub fn run(blocker: Option<String>) -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Auto Fix Windows")
            .with_inner_size([1000.0, 680.0])
            .with_min_inner_size([560.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native(
        "autofix",
        options,
        Box::new(move |cc| {
            load_font(&cc.egui_ctx);
            Ok(Box::new(App::new(blocker)))
        }),
    )
}

/// Font mặc định của egui thiếu một số chữ tiếng Việt: dùng Segoe UI của Windows nếu có.
fn load_font(ctx: &egui::Context) {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    if let Ok(bytes) = std::fs::read(PathBuf::from(root).join("Fonts").join("segoeui.ttf")) {
        ctx.add_font(egui::FontInsert::new(
            "Segoe UI",
            egui::FontData::from_owned(bytes),
            vec![egui::InsertFontFamily {
                family: egui::FontFamily::Proportional,
                priority: egui::FontPriority::Highest,
            }],
        ));
    }
}

/// Việc đang chạy ở luồng nền (để cửa sổ không bị treo).
enum Job {
    Scan(JoinHandle<(Vec<Finding>, bool)>),
    RestorePoint(JoinHandle<Result<String, String>>, Vec<Fix>),
    Fix(JoinHandle<()>),
}

impl Job {
    fn label(&self) -> &'static str {
        match self {
            Job::Scan(_) => "Đang quét... (sfc có thể mất 10–20 phút)",
            Job::RestorePoint(..) => "Đang tạo điểm khôi phục...",
            Job::Fix(_) => "Đang sửa... (DISM/sfc có thể mất 10–30 phút, đừng tắt máy)",
        }
    }
}

enum Dialog {
    Confirm(Vec<Fix>),
    NoRestorePoint(String, Vec<Fix>),
}

struct App {
    /// Lý do không được quét/sửa (không phải Windows, thiếu quyền Admin, không mở được log).
    blocker: Option<String>,
    /// `None` khi chưa quét.
    findings: Option<Vec<Finding>>,
    /// Bước sửa → có được tick không. Bước rủi ro mặc định không tick.
    fixes: BTreeMap<Fix, bool>,
    /// Bước bị chặn vì Windows đang chờ khởi động lại.
    blocked: Vec<Fix>,
    job: Option<Job>,
    dialog: Option<Dialog>,
}

impl App {
    fn new(blocker: Option<String>) -> Self {
        App {
            blocker,
            findings: None,
            fixes: BTreeMap::new(),
            blocked: vec![],
            job: None,
            dialog: None,
        }
    }

    fn start_scan(&mut self) {
        self.findings = None;
        self.fixes.clear();
        self.blocked.clear();
        self.job = Some(Job::Scan(thread::spawn(|| (checks::scan(), checks::reboot_pending()))));
    }

    fn scan_done(&mut self, findings: Vec<Finding>, reboot_pending: bool) {
        let all: BTreeSet<Fix> = findings.iter().flat_map(|f| f.fixes.iter().cloned()).collect();
        let (todo, blocked) = fix::plan(all, reboot_pending);
        self.fixes = todo.into_iter().map(|f| (f.clone(), !f.risky())).collect();
        self.blocked = blocked;
        self.findings = Some(findings);
    }

    fn start_restore_point(&mut self, todo: Vec<Fix>) {
        let names: Vec<String> = todo.iter().map(Fix::describe).collect();
        log::write(&format!("Người dùng chọn sửa: {}", names.join("; ")));
        log::say("Đang tạo điểm khôi phục hệ thống (System Restore)...");
        self.job = Some(Job::RestorePoint(thread::spawn(fix::create_restore_point), todo));
    }

    fn start_fix(&mut self, todo: Vec<Fix>) {
        self.findings = None;
        self.fixes.clear();
        self.blocked.clear();
        self.job = Some(Job::Fix(thread::spawn(move || fix::run_all(&todo))));
    }

    /// Nhận kết quả khi việc nền xong.
    fn poll(&mut self) {
        match self.job.take() {
            Some(Job::Scan(h)) if h.is_finished() => {
                let (findings, pending) = h.join().unwrap_or_else(|_| {
                    log::say("Lỗi bất ngờ khi quét, xem file log.");
                    (vec![], false)
                });
                self.scan_done(findings, pending);
            }
            Some(Job::RestorePoint(h, todo)) if h.is_finished() => {
                match h.join().unwrap_or_else(|_| Err("lỗi bất ngờ".into())) {
                    Ok(msg) => {
                        log::say(&msg);
                        self.start_fix(todo);
                    }
                    Err(e) => {
                        log::say(&format!("Không tạo được điểm khôi phục: {e}"));
                        self.dialog = Some(Dialog::NoRestorePoint(e, todo));
                    }
                }
            }
            Some(Job::Fix(h)) if h.is_finished() => {
                if h.join().is_err() {
                    log::say("Lỗi bất ngờ khi sửa, xem file log.");
                }
            }
            job => self.job = job,
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let idle = self.job.is_none() && self.blocker.is_none();
        ui.horizontal(|ui| {
            if ui
                .add_enabled(idle, egui::Button::new(RichText::new("Quét").heading()))
                .clicked()
            {
                self.start_scan();
            }
            let chosen: Vec<Fix> = self
                .fixes
                .iter()
                .filter(|(_, on)| **on)
                .map(|(f, _)| f.clone())
                .collect();
            let label = RichText::new(format!("Sửa {} mục đã chọn", chosen.len())).heading();
            if ui
                .add_enabled(idle && !chosen.is_empty(), egui::Button::new(label))
                .clicked()
            {
                self.dialog = Some(Dialog::Confirm(chosen));
            }
            if let Some(job) = &self.job {
                ui.spinner();
                ui.label(job.label());
            }
        });
        if let Some(blocker) = &self.blocker {
            ui.colored_label(Color32::from_rgb(220, 50, 50), blocker);
            if cfg!(windows)
                && !sys::is_admin()
                && ui.button("Mở lại với quyền Admin").clicked()
                && sys::relaunch_as_admin()
            {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn results(&mut self, ui: &mut egui::Ui) {
        let busy = self.job.is_some();
        let Some(findings) = &self.findings else {
            if !busy {
                ui.label("Bấm \"Quét\" để kiểm tra máy. Quét chỉ đọc, không thay đổi gì.");
            }
            return;
        };
        if findings.is_empty() {
            ui.label("Không phát hiện vấn đề nào.");
            return;
        }

        ui.heading(format!("Phát hiện {} vấn đề", findings.len()));
        for f in findings {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(severity_color(f.severity), RichText::new(f.severity.label()).strong());
                ui.label(&f.title);
            });
            if let Some(advice) = &f.advice {
                ui.indent(&f.title, |ui| ui.weak(advice));
            }
        }
    }

    /// Cột bên phải: các bước sửa để tick chọn.
    fn fix_list(&mut self, ui: &mut egui::Ui) {
        let busy = self.job.is_some();
        ui.heading("Các bước sửa");
        if self.fixes.is_empty() && self.blocked.is_empty() {
            ui.label("Không có bước sửa tự động, xem gợi ý bên trái.");
            return;
        }
        ui.weak("Bước có dấu ⚠ có rủi ro nên mặc định không chọn: chỉ chạy khi bạn tự tick.");
        ui.add_space(4.0);
        for (fix, on) in &mut self.fixes {
            let text = if fix.risky() {
                format!("⚠ {}", fix.describe())
            } else {
                fix.describe()
            };
            ui.add_enabled(!busy, egui::Checkbox::new(on, text));
        }
        for fix in &self.blocked {
            ui.add_enabled(
                false,
                egui::Checkbox::new(&mut false, format!("{} (khởi động lại máy trước)", fix.describe())),
            );
        }
    }

    fn dialog(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &self.dialog else { return };
        let mut answer = None;
        let modal = egui::Modal::new(egui::Id::unique("dialog")).show(ctx, |ui| {
            ui.set_max_width(560.0);
            let go = match dialog {
                Dialog::Confirm(todo) => {
                    ui.heading("Xác nhận sửa");
                    ui.label("Sẽ tạo điểm khôi phục hệ thống trước, rồi chạy lần lượt:");
                    for f in todo {
                        ui.label(format!("• {}", f.describe()));
                    }
                    "Bắt đầu sửa"
                }
                Dialog::NoRestorePoint(err, _) => {
                    ui.heading("Không tạo được điểm khôi phục");
                    ui.label(err);
                    ui.label("Nếu vẫn sửa thì sẽ không quay lại được bằng System Restore khi có sự cố.");
                    "Vẫn sửa"
                }
            };
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Huỷ").clicked() {
                    answer = Some(false);
                }
                if ui.button(go).clicked() {
                    answer = Some(true);
                }
            });
        });
        if answer.is_none() && modal.should_close() {
            answer = Some(false);
        }
        match (answer, self.dialog.take()) {
            (Some(true), Some(Dialog::Confirm(todo))) => self.start_restore_point(todo),
            (Some(true), Some(Dialog::NoRestorePoint(_, todo))) => {
                log::say("Người dùng chọn vẫn sửa khi không có điểm khôi phục.");
                self.start_fix(todo);
            }
            (Some(_), _) => log::say("Đã huỷ, không sửa gì."),
            (None, dialog) => self.dialog = dialog,
        }
    }
}

fn severity_color(severity: Severity) -> Color32 {
    match severity {
        Severity::Critical => Color32::from_rgb(220, 50, 50),
        Severity::Warning => Color32::from_rgb(230, 150, 0),
        Severity::Info => Color32::from_rgb(70, 140, 220),
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll();
        if self.job.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(250));
        }
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.add_space(6.0);
            self.toolbar(ui);
            ui.add_space(6.0);
        });
        egui::Panel::bottom("log")
            .resizable(true)
            .default_size(160.0)
            .show(ui, |ui| {
                ui.strong("Nhật ký");
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        for line in log::lines().iter() {
                            ui.label(line);
                        }
                    });
            });
        if self.findings.as_ref().is_some_and(|f| !f.is_empty()) {
            egui::Panel::right("fixes")
                .resizable(true)
                .default_size(340.0)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink(false)
                        .show(ui, |ui| self.fix_list(ui));
                });
        }
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .show(ui, |ui| self.results(ui));
        });
        self.dialog(&ui.ctx().clone());
    }
}
