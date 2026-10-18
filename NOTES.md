# GameMode — ghi chú làm tiếp

Cập nhật: 2026-10-19. Tool Rust + eframe/egui 0.29: tắt app nền trước khi chơi Delta Force, lưu session và khôi phục lại sau.

## Build và chạy
```
cargo build --release
```
- Exe nằm ở `target/release/gamemode.exe`. Bản đang dùng trên laptop là `C:\Users\thong\GameMode\GameMode.exe`, chỉ cần copy exe mới đè lên (phải tắt GameMode trước).
- Manifest yêu cầu quyền admin (`highestAvailable`).
- Các file `gamemode.cfg`, `session.json`, `session.last.json` và `gamemode-crash.log` nằm cạnh exe. `.gitignore` đã loại chúng khỏi repo.

## Cờ dòng lệnh để kiểm tra (không tắt app nào)
| Cờ | Làm gì | Kết quả ghi ra |
|---|---|---|
| `--selftest` | Đọc mọi process, RAM free/standby, Wi-Fi autoconf, Game DVR, UserGpuPreferences, task Scheduler, config | `selftest.txt` |
| `--purgetest` | Xả standby list một lần | `purgetest.txt` |
| `--launchtest` | Mở notepad bằng quyền user thường (token explorer) | `launchtest.txt` |
| `--tasktest` | Tạo task "GameMode", đọc XML, rồi xoá | `tasktest.txt` |
| `--minimized` | Khởi động thu nhỏ (task chạy cùng Windows dùng cờ này) | |

## Cấu trúc
- `src/main.rs`
  - Logic: `do_optimize`, `do_restore`, `start_watcher`, config, session.
  - UI: `App::new`, `update`, `running_app_row`.
  - `Shared` (các `Arc<Mutex>`) là cầu nối giữa UI và thread watcher.
- `src/sys.rs`: WinAPI (snapshot process, cmdline, kill, WM_CLOSE, service có timeout 5 giây, standby purge, WLAN, Game DVR, GPU pref, schtasks, mở app bằng token explorer).
- `src/ui.rs`: bảng màu và widget dùng chung (`card`, `section_header`, `toggle_row` có switch, `ghost_button`, `primary_button`, `callout_warning`).

## Đã xong
- Danh sách app đang chạy có công tắc "Giữ lại", lựa chọn nhớ trong `keep:` của config.
- Mặc định giữ lại: Discord, Steam, JBL Quantum, Tailscale, WARP, UniKey/EVKey, Claude, NVIDIA, webview2, NitroSense, game và anti-cheat ACE.
- Session chỉ lưu app thật: do explorer mở, có cửa sổ hoặc nằm trong danh sách tự khởi động. Restore không bao giờ chạy lại shell, Python hay agent nền.
- Tự động theo game (`auto:enabled`): tối ưu khi `DeltaForceClient-Win64-Shipping.exe` mở, tự Restore 15 giây sau khi game tắt.
- Xả standby list thay cho trim working set. Trong lúc chơi, tự xả khi RAM free < 1 GB.
- Ép game dùng GPU rời, tắt quét Wi-Fi nền và Game DVR khi chơi. Restore trả về như cũ.
- Chạy cùng Windows qua Task Scheduler (`/RL HIGHEST`, không hiện UAC).
- Giao diện mới (Gemini 3.8 Flash làm).

## Việc còn lại: 4 lỗi giao diện
Thấy trong ảnh chụp sau lần làm lại giao diện:
1. **Ô nhập tên exe game vẫn nền trắng**, chưa theo bảng màu tối. Xem `TextEdit::singleline(&mut self.game_input)` trong `main.rs` và `setup_visuals` trong `ui.rs`. Có thể cần `.background_color(...)` trên TextEdit, hoặc set `visuals.text_edit_bg_color`.
2. **Nút xoá game hiện ra ô vuông** thay vì "✕": font mặc định của egui không có glyph này. Đổi sang "×"/"x", hoặc tự vẽ dấu X bằng painter.
3. **Hàng tab/nút ở khối "ỨNG DỤNG ĐANG CHẠY" bị chồng chữ** ở chiều rộng ~500px (tab bị cắt, các nút "Mặc định / Bỏ giữ tất cả / Giữ tất cả / Làm mới" đè lên nhau). Cần xuống dòng (`horizontal_wrapped`) hoặc tách thành 2 hàng.
4. **Danh sách app bị cắt**: ScrollArea bên trong quá thấp nên dòng app bị che nửa. Nên bỏ scroll lồng (cả trang đã có ScrollArea), hoặc đặt chiều cao tối thiểu hợp lý.

Sau khi sửa: build, mở app, chụp màn hình ở chiều rộng mặc định (500px) và tối thiểu (460px) để kiểm tra.

## Lưu ý vận hành
- Service Office `ClickToRunSvc` phản hồi lệnh dừng rất chậm (từng kẹt ở `StopPending`). Nên bỏ tick nếu không cần.
- `svc:sshd=1` trong config sẽ dừng SSH server khi bật Game Mode.
- Auto mode mặc định tắt, phải tự bật trong UI.
- Khi tắt quét Wi-Fi, nếu mạng rớt thì máy không tự kết nối lại cho tới khi Restore.
