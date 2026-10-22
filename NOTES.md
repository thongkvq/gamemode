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
| `--updatetest` | Kiểm tra manifest cập nhật; nếu có bản mới thì tải và kiểm SHA-256 (KHÔNG chạy bộ cài) | `updatetest.txt` |
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
- Con cháu của app được giữ (`keep:` + danh sách mặc định) cũng được giữ: khi Game Boost / auto optimize / nén RAM, cả cây con cháu theo ppid của app được giữ (vd Claude mở `node.exe`, `cua-driver.exe`) đều không bị tắt và không bị trim, không lưu vào session nên Restore không mở lại. Chỉ đi xuống — tổ tiên của app được giữ không được bảo vệ; instance không phải con cháu vẫn theo quy tắc cũ. Danh sách "Ứng dụng đang chạy" hiện "GIỮ LẠI" kèm chú thích "con của Claude" và khoá công tắc của dòng đó; dòng có cả instance được giữ lẫn instance sẽ tắt thì ghi "N giữ (con của Claude) · M sẽ tắt". Log Game Boost thêm dòng `· Giữ N process con của app được giữ (<app>: <exe> ×n, …)`.

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
- Chạy quyền thường (vd mở bằng `__COMPAT_LAYER=RunAsInvoker` khi test) thì không xả được standby; lỗi này chỉ log một lần mỗi lần mở app, kèm gợi ý mở lại bằng quyền admin.
- `--selftest` ghi `kept_children` / `kept_by` ở mỗi dòng `APP` để kiểm quy tắc giữ con cháu của app được giữ.
- Khi tắt quét Wi-Fi, nếu mạng rớt thì máy không tự kết nối lại cho tới khi Restore.

## Bộ cài (Inno Setup)
- Cài Inno Setup 6: `winget install JRSoftware.InnoSetup --scope user`.
- Build: `cargo build --release`, rồi `"%LOCALAPPDATA%\Programs\Inno Setup 6\ISCC.exe" installer\gamemode.iss`, ra file `dist\GameMode-Setup-<version>.exe` (thư mục `dist/` không đưa lên git).
- Version (hiện 1.2.0) phải sửa ở 2 chỗ: `Cargo.toml` và `#define AppVersion` trong `installer/gamemode.iss`.
- Bộ cài cài theo từng user vào `%LOCALAPPDATA%\Programs\GameMode`, không cần admin khi cài. Gỡ cài đặt sẽ xoá config/session và task "chạy cùng Windows".
- Exe chưa ký số nên lần đầu chạy sẽ bị SmartScreen cảnh báo.
- Bộ cài có nhánh cập nhật OTA: chạy kèm `/UPDATE` thì `[Code] IsUpdateRun` bật entry `[Run]`
  không có `skipifsilent`, nên cài xong (kể cả `/VERYSILENT`) là tự mở lại `{app}\GameMode.exe`.

## Cập nhật OTA (2026-10-22)
- Manifest: `https://update-dhcs.paroda.net/apps/gamemode/manifest.json` (nginx, `Cache-Control: no-store`).
  Bộ cài: `https://update-dhcs.paroda.net/apps/gamemode/releases/<filename>` (kèm `releases.json` — mảng manifest cũ, mới nhất ở đầu).
- Config trong `gamemode.cfg`: `update:enabled=1/0` (mặc định 1) và `update:url=<url manifest>`.
  Công tắc "Tự động kiểm tra cập nhật" + một dòng trạng thái và nút Kiểm tra/Cài nằm ở card
  "Cập nhật OTA" trong tab **Tự động & Tối ưu**. Có bản mới thì banner trên cùng cột phải: Cập nhật / Xem thay đổi / Để sau,
  thanh tiến độ khi tải, rồi "Cài và khởi động lại".
- `src/update.rs`: `UpdateState` = `Idle | Checking | UpToDate | Available(manifest) | Downloading{done,total} | Ready(path) | Failed(lỗi)`,
  giữ trong `Shared` (`update_state`, `update_manifest`, `update_last_check`, `update_request`, `update_url`).
  Thread nền kiểm tra 20 giây sau khi mở app rồi mỗi 6 giờ (timeout HTTP 10 giây); lỗi mạng chỉ ghi `Failed` + 1 dòng log.
  Bộ cài tải vào `%TEMP%\GameMode-update\`, phải khớp `size` và `sha256` của manifest (sai thì xoá file), chỉ nhận `download_url` dạng `https://`.
- Cài: từ chối khi đang có game trong danh sách chạy hoặc đang trong phiên Game Boost; ghi
  `update-pending.json` (`{"from":"1.2.0","to":"1.3.0"}`) cạnh exe rồi chạy
  `cmd.exe /c ping -n 4 127.0.0.1 >nul & start "" "<setup>" /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /UPDATE`
  (cửa sổ ẩn, trễ ~3 giây cho app kịp thoát trước khi Inno kiểm `AppMutex`), sau đó thoát như menu khay "Thoát".
  Lần mở sau thấy file đánh dấu thì log `✔ Đã cập nhật 1.2.0 → 1.3.0` (hoặc báo không thành công), hiện cửa sổ dù có `--minimized`, rồi xoá file.
- `--updatetest`: kiểm tra manifest + tải thử + kiểm SHA-256 (không chạy bộ cài, không cần admin) → `updatetest.txt`.
  `GAMEMODE_FAKE_VERSION=1.0.0` để giả version hiện tại khi thử (dùng cho cả `--updatetest`).
- Publish: `cargo build --release` → `ISCC installer\gamemode.iss` → `scripts\publish_update.ps1 -Version 1.3.0 -Changelog "# 1.3.0 - ..."`
  (thêm `-DryRun` để chỉ tạo manifest, không upload). Script kiểm version khớp `Cargo.toml` + `installer/gamemode.iss`,
  scp lên host Proxmox rồi `pct push` vào container 106, `mv` manifest nguyên tử, chown `www-data`, và đọc lại qua HTTPS để so version + sha256.
  Lưu `.iss` và `.ps1` dạng UTF-8 **có BOM** (Inno Setup và PowerShell 5.1 cần BOM để đọc đúng tiếng Việt).
- Kênh thử: `-RemoteDir /srv/dhcs-update/apps/gamemode-test -PublicBaseUrl https://update-dhcs.paroda.net/apps/gamemode-test`,
  app trỏ vào bằng `update:url=https://update-dhcs.paroda.net/apps/gamemode-test/manifest.json`. Kênh này đang có bản 1.2.0 thử.
- Đã thử trên PC (2026-10-22): publish kênh thử, `--updatetest` tải + khớp SHA-256, banner hiện đủ 3 trạng thái, từ chối cài khi đang chơi.
  **Chưa thử** vòng cài thật (thoát app → bộ cài `/VERYSILENT /UPDATE` → tự mở lại). Kênh thật `apps/gamemode` **chưa có manifest**: bản đầu tiên publish lên đó nên là 1.3.0.
