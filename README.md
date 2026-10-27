# auto-fix-window-not-crash-

App Windows (Rust, có giao diện) quét lỗi thường gặp và sửa **an toàn**: bấm **Quét** chỉ đọc, không thay đổi gì; chỉ khi bấm **Sửa** mới sửa các mục bạn đã tick.

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

- **Quét** chỉ đọc, không thay đổi gì.
- **Sửa**: tạo **điểm khôi phục (System Restore)** trước; không tạo được thì hỏi lại, mặc định là dừng.
- Bước rủi ro (DISM, sfc, đặt lại Windows Update, chkdsk) có dấu ⚠ và **mặc định không được chọn**: chỉ chạy khi bạn tự tick, và còn một hộp thoại xác nhận cuối.
- Không bao giờ sửa boot, driver hay ghi registry. Đặt lại Windows Update chỉ **đổi tên** thư mục `SoftwareDistribution`, không xoá.
- Windows đang chờ khởi động lại thì khoá DISM/sfc/Windows Update.
- Ổ cứng báo sắp hỏng thì **không** hẹn chkdsk (tránh làm hỏng thêm).
- chkdsk chỉ được hẹn ở lần khởi động sau, không gỡ ổ khi đang dùng.
- Xoá file tạm: chỉ file cũ hơn 7 ngày trong `%TEMP%` và `C:\Windows\Temp`, bỏ qua file đang dùng và symlink/junction.
- Mọi thao tác ghi vào `autofix.log` (cạnh file .exe).

## Tải / build

- Tải sẵn: tab **Actions** → lần chạy mới nhất → artifact `autofix-windows-x64` (chứa `autofix.exe`).
- Tự build (Windows, đã cài [Rust](https://rustup.rs)): `cargo build --release` → `target\release\autofix.exe`.

## Dùng

1. Bấm đúp `autofix.exe` → Windows hỏi quyền Admin (UAC) → chọn **Yes**.
2. Bấm **Quét** (sfc có thể mất 10–20 phút).
3. Xem danh sách vấn đề bên trái; tick các bước sửa muốn chạy ở cột bên phải.
4. Bấm **Sửa ... mục đã chọn** → xác nhận. Xong thì khởi động lại máy nếu app nhắc, rồi **Quét** lại.

Chạy tự động (không mở cửa sổ): `autofix.exe --scan` chỉ quét, ghi kết quả vào `autofix.log`; mã thoát `0` không có lỗi, `1` có vấn đề, `2` thiếu quyền Admin.

## Phát triển

```bash
cargo test                                               # test parser + logic (chạy được cả trên Linux)
cargo clippy --all-targets --target x86_64-pc-windows-gnu -- -D warnings
```

CI (`.github/workflows/ci.yml`, `windows-latest`): clippy, test, build `.exe`, upload artifact, mở thử cửa sổ và chạy thử quét thật (`--scan`).
