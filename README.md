# auto-fix-window-not-crash-

Công cụ dòng lệnh (Rust) quét lỗi Windows thường gặp và sửa **an toàn**: mặc định chỉ quét, không đụng gì tới máy.

## Quét những gì

| Kiểm tra | Cách làm | Sửa được |
|---|---|---|
| Màn hình xanh (BSOD), tắt máy đột ngột | Event Log System 7 ngày: BugCheck 1001, Kernel-Power 41 | DISM + sfc |
| Ứng dụng crash/treo | Event Log Application: 1000, 1002 | (chỉ gợi ý) |
| Windows Update cài lỗi | WindowsUpdateClient 20 | Đặt lại Windows Update |
| Dịch vụ tự khởi động bị dừng vì lỗi | WMI `Win32_Service` | Khởi động lại dịch vụ |
| Ổ đĩa sắp đầy (< 10%) | WMI `MSFT_Volume` | Xoá file tạm > 7 ngày (ổ hệ thống) |
| Lỗi hệ thống file NTFS | WMI `MSFT_Volume.HealthStatus` | Hẹn chkdsk lần khởi động sau |
| Ổ cứng sắp hỏng (SMART) | WMI `MSFT_PhysicalDisk.HealthStatus` | (chỉ cảnh báo: sao lưu, thay ổ) |
| Kho thành phần Windows hỏng | `Repair-WindowsImage -CheckHealth` | DISM RestoreHealth + sfc |
| File hệ thống hỏng | `sfc /verifyonly` (10–20 phút) | DISM + sfc /scannow |
| Windows đang chờ khởi động lại | registry (chỉ đọc) | (khởi động lại) |

## An toàn

- Không có `--fix` thì **chỉ quét và báo cáo**.
- Có `--fix`: tạo **điểm khôi phục (System Restore)** trước khi sửa; không tạo được thì hỏi, mặc định là dừng.
- DISM, sfc, đặt lại Windows Update, chkdsk: **luôn hỏi `[y/N]`** từng bước (mặc định Không).
- Không bao giờ sửa boot, driver hay ghi registry. Đặt lại Windows Update chỉ **đổi tên** thư mục `SoftwareDistribution`, không xoá.
- Windows đang chờ khởi động lại thì bỏ qua DISM/sfc/Windows Update.
- Ổ cứng báo sắp hỏng thì **không** hẹn chkdsk (tránh làm hỏng thêm).
- chkdsk chỉ được hẹn ở lần khởi động sau, không gỡ ổ khi đang dùng.
- Xoá file tạm: chỉ file cũ hơn 7 ngày trong `%TEMP%` và `C:\Windows\Temp`, bỏ qua file đang dùng và symlink/junction.
- Mọi thao tác ghi vào `autofix.log` (cạnh file .exe).

## Tải / build

- Tải sẵn: tab **Actions** → lần chạy mới nhất → artifact `autofix-windows-x64` (chứa `autofix.exe`).
- Tự build (Windows, đã cài [Rust](https://rustup.rs)):

```powershell
cargo build --release
# file: target\release\autofix.exe
```

## Chạy

Mở **PowerShell bằng quyền Administrator** (chuột phải → *Run as administrator*), vào thư mục chứa `autofix.exe`:

```powershell
.\autofix.exe          # chỉ quét, báo cáo
.\autofix.exe --fix    # quét rồi sửa (hỏi trước các bước rủi ro)
.\autofix.exe --log D:\autofix.log   # đổi chỗ lưu log
```

Mã thoát: `0` không có lỗi / sửa xong, `1` có vấn đề hoặc có bước sửa lỗi, `2` thiếu quyền Admin hoặc không mở được log.

## Phát triển

```bash
cargo test                                               # test parser + logic (chạy được cả trên Linux)
cargo clippy --all-targets --target x86_64-pc-windows-gnu -- -D warnings
```

CI (`.github/workflows/ci.yml`, `windows-latest`): clippy, test, build `.exe`, upload artifact và chạy thử quét thật (không `--fix`).
