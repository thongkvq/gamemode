# GameMode — ghi chú làm tiếp

Cập nhật: 2026-10-22. Tool Rust + eframe/egui 0.29: tắt app nền trước khi chơi Delta Force, lưu session và khôi phục lại sau.

## Đang dở và làm tiếp (2026-10-22)
- **Bản đã phát hành**: v1.2.0 trên GitHub Releases (có thu xuống khay). Bản trong `main` mới hơn: đã có phần **ghi báo cáo trận** (`src/report.rs`), đã test `--pingtest` và `--reporttest` trên máy thật, nhưng **chưa có giao diện xem báo cáo**.
- **Tab "Báo cáo trận" (task-14, Gemini) đã vào `main`** cùng banner cảnh báo commit và biểu đồ `egui_plot` 0.29. Build Windows OK, logic không đổi, **nhưng chưa xem giao diện**: lúc đó GameMode bản cài đang chạy dưới khay nên bản test bị chặn bởi chế độ một bản. Việc tiếp theo:
  1. ~~Kiểm tra tab "Báo cáo trận"~~ — đã xem trên PC ngày 2026-10-22: danh sách, thẻ chỉ số, nhận xét, 4 biểu đồ, trận đang ghi cập nhật trực tiếp đều đúng. Nút xoá hiện bước "Bấm lại để xoá" nhưng **chưa xác nhận lần bấm thứ hai xoá thật** (chỉ test bằng click giả).
  2. Đã sửa trên PC (task-15 Gemini + vài dòng tay) và đã commit lên `main`; PC giờ có bản clone ở `I:\dev\GameMode` (ngoài thư mục đồng bộ): nhãn trục Y không còn bị mất, trục X dạng `m:ss`, chú thích chuyển ra hàng trên biểu đồ (chấm tự vẽ vì font không có glyph `●`), tên game thân thiện ("Delta Force"), thêm 4 unit test (`cargo test` cần `__COMPAT_LAYER=RunAsInvoker` vì exe test cũng đòi admin).
  3. Thử nút xoá bằng tay, rồi nâng version 1.3.0 (Cargo.toml + installer), build bộ cài, tạo Release.
- **Tính năng còn trong danh sách đề xuất**: quản lý app khởi động cùng Windows (máy đang có ~13 mục Run + 1 file Startup), hồ sơ theo game, phím tắt toàn cục.
- **Ngoài phần mềm (đã khuyên chủ máy)**: thêm thanh RAM 16 GB (đang single channel); đặt pagefile cố định 16–24 GB (commit từng chạm 95%); cắm dây LAN thay Wi-Fi; driver NVIDIA hiện tên "GTX 750 Ti" trong khi card thật là RTX 3060 Laptop (DEV_2520), nên cài lại driver chính hãng nếu FPS bất thường.
- **Lưu ý khi giao việc**: launcher `delegate.sh` hay thoát sớm khi chạy nền lâu, nhưng worker trên harness vẫn chạy tiếp; theo dõi bằng file `task-NN.status.json` (`"terminal": true` là xong).

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
| `--traytest` | Tạo icon khay + menu trong 5 giây rồi thoát | `traytest.txt` |
| `--pingtest` | Ping gateway (`GetBestRoute`) và host internet 5 lần mỗi đích (IP, RTT, mất gói) | `pingtest.txt` |
| `--reporttest` | Chạy recorder giả lập 20 giây (mẫu mỗi 2 giây, không cần game, cho phép trận < 60 s) và lưu một báo cáo `game = "test"` | `reporttest.txt` |
| `--minimized` | Khởi động **ẩn xuống khay hệ thống** (task chạy cùng Windows dùng cờ này) | |

## Báo cáo trận đấu (2026-10-22)
- Bật/tắt bằng `report:enabled=1/0` (mặc định 1) — công tắc "Ghi báo cáo trận đấu" trong card "Tối ưu khi chơi". Ghi cả khi auto mode tắt.
- Watcher thấy game trong danh sách chạy: mở báo cáo, chạy thread ping (1 gói/giây tới gateway + host internet) và thread lấy mẫu (mỗi 5 giây: RAM khả dụng, Free/Standby, commit, CPU toàn hệ thống, ping TB/max, % mất gói).
- Game tắt (đúng mốc 15 giây của watcher): dừng ghi, tính `Summary` + nhận xét tiếng Việt, ghi `reports/<id>.json` cạnh exe (id dạng `20261022-213005`) rồi log một dòng `▣ [BÁO CÁO] ...`. Trận dưới 60 giây không lưu; chỉ giữ 30 báo cáo mới nhất.
- `report:ping_host=<host>` (mặc định `1.1.1.1`) là đích ping internet, có thể là tên miền.
- Thư mục `reports/` nằm cạnh exe, đã thêm vào `.gitignore` và `[UninstallDelete]` của bộ cài.
- Cảnh báo commit: watcher kiểm tra mỗi 5 giây; `commit >= 90%` thì log đỏ (tối thiểu 2 phút một lần), bật `Shared.commit_alert` (icon khay thêm `⚠ Commit X%`), thêm event `commit_warn` vào báo cáo và — nếu đang chơi — xả standby ngay **một lần** (xả standby không giảm commit nên vẫn giữ cảnh báo). Dưới 85% thì tắt cảnh báo.
- API cho tab báo cáo (task sau): `report::list_reports()`, `report::load_report(id)`, `report::delete_report(id)`.

## Khay hệ thống (2026-10-22)
- Nút X trên title bar và Alt+F4 **không thoát** nữa: cửa sổ ẩn xuống khay (icon cạnh đồng hồ). Thoát hẳn bằng menu chuột phải ở icon khay → `Thoát`.
- Menu khay: `Mở GameMode` (click trái icon cũng hiện cửa sổ), `Bật Game Boost`, `Khôi phục`, `Tự động khi mở game` (tick, đồng bộ với `auto:enabled`), `Thoát`.
- Tuỳ chọn `ui:close_to_tray=1/0` (mặc định 1), có công tắc trong card "Tự động khi chơi game". Đặt 0 thì nút X thoát hẳn như trước.
- Chỉ chạy một bản: mở bản thứ hai chỉ hiện cửa sổ bản cũ rồi thoát (named mutex `Local\GameModeTacticalRamPurge`).
- Sự kiện khay xử lý ở thread riêng (`src/tray.rs`) vì eframe không gọi `update()` khi cửa sổ đã ẩn.

## Cấu trúc
- `src/main.rs`
  - Logic: `do_optimize`, `do_restore`, `start_watcher`, config, session.
  - UI: `App::new`, `update`, `running_app_row`.
  - `Shared` (các `Arc<Mutex>`) là cầu nối giữa UI và thread watcher.
- `src/sys.rs`: WinAPI (snapshot process, cmdline, kill, WM_CLOSE, service có timeout 5 giây, standby purge, WLAN, Game DVR, GPU pref, schtasks, mở app bằng token explorer, ẩn/hiện cửa sổ chính + named mutex một bản, commit charge, `CpuMeter` từ `GetSystemTimes`, gateway + ICMP ping).
- `src/report.rs`: báo cáo trận đấu (`Sample`, `Event`, `Summary`, `GameReport`, `ReportMeta`, `Live` đang ghi, đọc/ghi/xoá file trong `reports/`).
- `src/tray.rs`: icon khay + menu chuột phải, thread nhận sự kiện menu/click (chạy được khi cửa sổ đã ẩn).
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

## Bộ cài (Inno Setup)
- Cài Inno Setup 6: `winget install JRSoftware.InnoSetup --scope user`.
- Build: `cargo build --release`, rồi `"%LOCALAPPDATA%\Programs\Inno Setup 6\ISCC.exe" installer\gamemode.iss`, ra file `dist\GameMode-Setup-<version>.exe` (thư mục `dist/` không đưa lên git).
- Version (hiện 1.2.0) phải sửa ở 2 chỗ: `Cargo.toml` và `#define AppVersion` trong `installer/gamemode.iss`.
- Bộ cài cài theo từng user vào `%LOCALAPPDATA%\Programs\GameMode`, không cần admin khi cài. Gỡ cài đặt sẽ xoá config/session và task "chạy cùng Windows".
- Exe chưa ký số nên lần đầu chạy sẽ bị SmartScreen cảnh báo.
