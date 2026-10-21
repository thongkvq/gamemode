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

## Đã làm thêm (2026-10-21)
- Nén RAM (trim working set) có chọn lọc, chạy sau bước xả standby. Không nén game, anti-cheat, audiodg, dwm, csrss. Log ghi 5 app còn giữ nhiều RAM nhất. Tuỳ chọn `opt:trim`.
- Giao diện 2 cột (1120×760, tối thiểu 900×600):
  - Cột trái: RAM, Boost, session, log.
  - Cột phải: 3 tab, danh sách app và service dạng lưới 2 cột.
- Đã sửa 4 lỗi UI cũ, cùng lỗi tên app dài bị đè chữ.
- Không bao giờ tắt bản `gamemode.exe` khác đang mở.

## Phân công model trên harness
Logic giao DeepSeek, giao diện giao Gemini 3.8 Flash.

## Mẹo chụp màn hình UI để kiểm tra
App đòi quyền admin nên shell thường không chụp được cửa sổ. Mở bằng `__COMPAT_LAYER=RunAsInvoker` rồi chụp bằng `PrintWindow(hwnd, hdc, 2)`. Chế độ này không có quyền admin nên một số số liệu (Free RAM, service) có thể khác bản thật.

## Lưu ý vận hành
- Service Office `ClickToRunSvc` phản hồi lệnh dừng rất chậm (từng kẹt ở `StopPending`). Nên bỏ tick nếu không cần.
- `svc:sshd=1` trong config sẽ dừng SSH server khi bật Game Mode.
- Auto mode mặc định tắt, phải tự bật trong UI.
- Khi tắt quét Wi-Fi, nếu mạng rớt thì máy không tự kết nối lại cho tới khi Restore.
