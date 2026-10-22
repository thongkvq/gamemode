# GameMode — ghi chú làm tiếp

Cập nhật: 2026-10-23. Tool Rust + eframe/egui 0.29: tắt app nền trước khi chơi Delta Force, lưu session và khôi phục lại sau.

## Bản 1.4.0 / 1.4.1 (2026-10-22) — Paroda GameMode
- Đổi tên hiển thị thành **Paroda GameMode** (title bar, khay, Task Manager, bộ cài, shortcut). GIỮ nguyên: `GameMode.exe`, `AppId`, mutex, task `GameMode`, thư mục cài `%LOCALAPPDATA%\Programs\GameMode`, tên bộ cài `GameMode-Setup-<ver>.exe`, `"app": "GameMode"` của manifest.
- Mới: tab Khởi động (task-22), phím tắt toàn cục (task-23), task chạy cùng Windows bằng XML — chạy cả khi dùng pin, không giới hạn 72 giờ, tự sửa task cũ lúc mở app (task-24/26), banner + `--restore` cho phiên Boost bỏ lửng, chặn `NOT_GAMES` (task-25), hồ sơ theo game (task-26).
- Claude sửa thêm khi thử trên laptop: `<UserId>` của task dùng **SID** (`whoami /user`) — tên `USERDOMAIN\USERNAME` từ env có thể sai (phiên SSH ra `WORKGROUP\thong`, schtasks từ chối); `--tasktest` **không tạo/xoá khi task đã có** (bản cũ từng xoá mất task thật); `show_main_window` đặt lại cỡ cửa sổ khi vị trí "bình thường" chỉ là ô thu nhỏ 160×28 (lỗi có từ 1.3.2: app mở bằng `--minimized` lúc đăng nhập thì "Mở" ở khay ra cửa sổ tí hon).
- Đã thử trên laptop (build ở `C:\build\gamemode-1.4.0`, 71 unit test qua): cài đè 1.3.4 → shortcut/uninstall mang tên mới, shortcut "GameMode" cũ bị xoá; task cũ (pin = true) được tự sửa khi mở app; `Ctrl+Alt+G` hiện/ẩn cửa sổ khi đang ở khay; tab Khởi động khớp Task Manager (15 mục, 5 bật); `--restore` không có session → exit 1.
- **Chưa thử**: Boost theo hồ sơ (sẽ tắt app thật), banner phiên bỏ lửng bằng mắt, đổi tổ hợp phím + Lưu, bật/tắt mục ở tab Khởi động.
- **1.4.1** (cùng ngày): vtable `IShellLinkW` tự khai trong `startup.rs` sai thứ tự (gọi SetRelativePath thay GetPath) nên `.lnk` không ra exe — đã sắp lại theo `shobjidl_core.h`, `Tailscale.lnk` hiện `tailscale-ipn.exe`. Card Cập nhật/Phím tắt lấn mép phải vì `ui.columns` nới max_rect của `ui` → bọc hai card trong `allocate_ui_with_layout(full_w)` với bề rộng lấy trước `columns`. Cả hai đã xem bằng ảnh chụp trên laptop.
- Mẹo thử UI từ xa: click bằng `PostMessage(WM_MOUSEMOVE/LBUTTONDOWN/UP)` vào hwnd của app (toạ độ client), ĐỪNG dùng `SetCursorPos`+`mouse_event` — khi chủ máy đang dùng laptop, cửa sổ GameMode không lên trên cùng và click rơi vào app khác.
- Mẹo thử UI từ xa: script PowerShell chạy bằng task `/it /rl highest` trong phiên người dùng (`SetProcessDPIAware`, `keybd_event`, `PrintWindow`) — từ phiên SSH (session 0) không thấy cửa sổ.

## Tối ưu Win (1.5.0)
- Tab **🔧 Tối ưu Win** (cột phải, sau tab "Tự động & Tối ưu", `ConfigTab::WinOpt`) — bản 1.5.0 (task-27 wintweaks, task-28 hwinfo, task-29 UI).
- **Claude sửa thêm trong 1.5.0**:
  - **FPS trong báo cáo trận chưa từng chạy trên laptop (1.3.3–1.4.1)**: Delta Force trên laptop Optimus phát event DXGI **180** (`IDXGISwapChain1_Present1`, task 81), KHÔNG có 178 nào (trace `logman` 8 s lúc đang chơi: DF 242× event 180/181 + 42/43; dwm 196/197). 3 trận 2026-10-22 tối đều `fps: null`. `fps.rs` giờ đếm cả 178 và 180, PID đã phát 180 thì bỏ 178. `--fpstest 10` sau khi sửa: ~30 FPS (lúc đó có thể đang ở sảnh/khóa FPS), frametime dao động 33↔74 ms — **cần đối chiếu bằng báo cáo một trận thật**.
  - `wintweaks`: hoàn tác `defender_games` khi lúc bật chưa có thư mục loại trừ nào trước đây không gỡ gì; đọc setting nguồn bị ẩn (CPU boost mode) bằng cách bỏ cờ ẩn rồi `/q` lại (trước luôn "Không rõ").
  - Icon tab đổi 🪟 → 🔧 (font egui không có glyph 🪟, hiện ô vuông).
  - UI đã xem bằng ảnh chụp: card Phần cứng (8 gợi ý), đầu card Tinh chỉnh (5/26, nút gói khuyến nghị / hoàn tác mờ khi chưa có sao lưu). **Chưa xem bằng mắt các hàng tinh chỉnh bên dưới** (cuộn bằng PostMessage WM_MOUSEWHEEL không ăn khi con trỏ thật ở ngoài cửa sổ).
  - Đã thử trên laptop: `--hwinfo` (8 gợi ý đúng máy), `--tweakstatus` (26/26 đọc được), vòng `--tweakapply/--tweakrevert startup_delay_0` (registry về "không có", backup rỗng lại) và `pcie_aspm_off_ac`.
- **Card "Phần cứng"** (trên cùng): `hwinfo::summary_line` + danh sách `hwinfo::advice` (chấm màu: Good xanh,
  Info accent, Warn vàng; tiêu đề đậm, chi tiết chữ mờ) + nút *Quét lại*. `hwinfo::collect()` chạy ở thread
  nền khi mở tab lần đầu và mỗi lần *Quét lại* (WMI/PowerShell chặn vài giây) — đang quét hiện
  "Đang đọc thông tin phần cứng…", lỗi hiện đỏ. Chỉ đọc, không đổi gì trên máy.
- **Card "Tinh chỉnh Windows"**: dòng đếm "N/M đang bật"; nút **Áp dụng gói khuyến nghị** (các tweak
  `recommended` chưa bật) và **Hoàn tác tất cả** (mọi tweak có trong bản sao lưu), cả hai **bấm 2 lần**
  ("Bấm lại để xác nhận", tự huỷ sau 4 giây, `CONFIRM_WINDOW`). Từng tweak một hàng: tiêu đề (tooltip =
  nhóm · rủi ro · số thao tác và từng thao tác), badge rủi ro (An toàn xanh / Cân nhắc vàng / Nâng cao đỏ),
  badge "Khởi động lại" khi `needs_reboot`, mô tả chữ mờ, trạng thái (Đang bật xanh / Chưa bật mờ /
  Một phần vàng / Không rõ + lý do trong tooltip), nút **Bật** (khi chưa bật; tweak **Nâng cao** cũng phải
  bấm 2 lần) và **Hoàn tác** (chỉ khi có trong bản sao lưu). Trong lúc một thao tác chạy thì mọi nút bị khoá.
- Mọi thao tác (bật/hoàn tác/đọc lại) chạy **tuần tự trên thread nền**, ghi log từng tweak
  (`✔ [WIN] Đã bật: …`, `↺ [WIN] Đã hoàn tác: …`, lỗi `⚠ [WIN] <tiêu đề>: <lỗi>`). Trạng thái đọc lại sau
  mỗi thao tác (powercfg / bcdedit / PowerShell chậm). Tweak `needs_reboot` vừa đổi trong lần mở này hiện
  callout vàng "Cần khởi động lại máy để có hiệu lực: …".
- **Bản sao lưu**: `wintweaks-backup.json` cạnh exe (cùng thư mục `config_path()`), ghi lại NGAY sau mỗi
  tweak bật/hoàn tác. **KHÔNG** thêm vào `[UninstallDelete]` của bộ cài (bộ cài cũ chỉ xoá `{app}\*.txt`,
  `gamemode.cfg`, `session*.json`, `reports\*`) — gỡ app vẫn giữ file này để cài lại còn hoàn tác được.
  Đã thêm vào `.gitignore`.
- Danh sách tweak theo nhóm (`wintweaks::all_tweaks(game_dirs)`; `game_dirs` = thư mục cha của mọi
  `gamepath:` trong config, bỏ trùng):
  - **Hiệu năng game**: `game_mode`, `hags`, `windowed_opt`, `mmcss_games`, `no_gamedvr`, `mouse_raw`,
    `defender_games` (chỉ xuất hiện khi có `gamepath:`), `power_throttling_off`.
  - **Giảm tác vụ nền**: `bg_apps_off`, `no_suggestions`, `no_widgets`, `telemetry_min`,
    `delivery_opt_p2p_off`, `edge_bg_off`, `startup_delay_0`, `animations_off`, `services_trim`,
    `sysmain_off`, `search_index_off`.
  - **Bộ nhớ**: `mem_compression_on`, `pagefile_fixed`.
  - **Hệ thống**: `fast_startup_off`, `pcie_aspm_off_ac`, `cpu_boost_aggressive_ac`, `usb_suspend_off_ac`,
    `hypervisor_off` (rủi ro Nâng cao; tắt VBS/hypervisor — WSL2/Docker/BlueStacks không chạy cho tới khi hoàn tác).
- Cờ dòng lệnh mới để thử từ xa: `--hwinfo` (`hwinfo.txt`), `--tweakstatus` (`wintweaks-status.txt`),
  `--tweakapply <id>` / `--tweakrevert <id>` (`wintweaks-result.txt`) — xem bảng "Cờ dòng lệnh" dưới.

## Bàn giao sang laptop (2026-10-22 tối, từ PC)
- **Bản mới nhất: v1.5.0** trên kênh OTA thật và GitHub Release. Tải trực tiếp:
  `https://update-dhcs.paroda.net/apps/gamemode/releases/GameMode-Setup-1.5.0.exe`
  (hoặc `https://github.com/thongkvq/gamemode/releases/download/v1.5.0/GameMode-Setup-1.5.0.exe`).
  **Laptop đã cài 1.3.3 (2026-10-22, qua SSH)** vào `%LOCALAPPDATA%\Programs\GameMode` (trước đó chạy bản 1.1.0 portable ở `C:\Users\thong\GameMode`, thư mục cũ còn nguyên, shortcut cũ dời vào `old-shortcuts\`). Đã chép `gamemode.cfg` + `session.last.json` sang; task "GameMode" trỏ exe mới, thêm `Delay PT2M30S` ở LogonTrigger để né lỗi dưới. Từ đây các bản sau đến qua OTA.
- **Đã sửa ở 1.3.4** (2026-10-22): crash khi mở trong ~2 phút đầu sau khi bật máy — `Instant::now() - X` (watcher và nút task trong `src/main.rs`) panic khi uptime < X; giờ dùng `checked_sub(..).unwrap_or_else(Instant::now)`. 1.3.4 build trên laptop (PC tắt) từ `main` + commit `8f3efbe` (đã push lên `main`, tag `v1.3.4` + GitHub Release kèm bộ cài), publish OTA thật lúc 18:31. Laptop đã lên 1.3.4.
- **Config laptop**: từng có `game:msedgewebview2.exe=1` khiến auto mode Boost ngay khi mở (WebView2 luôn chạy). Đã bỏ dòng đó (bản cũ: `gamemode.cfg.bak-20261022`). Báo cáo `20261022-175503` là trận giả do lỗi này.
- **Code**: repo `thongkvq/gamemode`, nhánh `main`. Trên laptop `git pull` là có đủ (PC commit từ `I:\dev\GameMode`; thư mục `X:\Test AI\GameMode` là bản làm việc đồng bộ).
- **1.3.3 có gì**: đo FPS/frametime vào báo cáo trận (`src/fps.rs`, xem mục "FPS" dưới); báo cáo không mất khi GameMode tắt giữa trận (lưu tạm `reports/<id>.partial.json` mỗi phút, chốt khi thoát, tự khôi phục lần mở sau); tự khôi phục/tự Boost không xoá log nữa; bấm Khôi phục tay giữa trận thì lúc game tắt không khôi phục lần 2; xoá báo cáo khi đang ghi trận chọn lại đúng; `--minimized` mà không tạo được icon khay thì chỉ thu nhỏ.
- **CHƯA KIỂM CHỨNG — làm đầu tiên trên laptop**: số FPS thật với Delta Force bằng code mới. Đã kiểm bằng bản ghi ETW thô (`logman` provider DXGI) lúc DF đang chạy: 191 FPS, p99 7,2 ms — đúng nguyên lý, nhưng `FpsMeter` (session ETW real-time + callback) chưa thấy frame thật lần nào vì Edge/Discord không phát event 178. Cách thử: mở Delta Force, đợi vào sảnh, chạy `GameMode.exe --fpstest 20` (thư mục cạnh exe, config có `game:deltaforceclient-win64-shipping.exe=1`) → đọc `fpstest.txt`; hoặc chơi 1 trận rồi xem tab Báo cáo trận có thẻ "FPS TB / 1% low" và biểu đồ FPS. Nếu "không có frame nào": nghi `Wnode.ClientContext`/`PROCESS_TRACE_MODE_RAW_TIMESTAMP` hoặc `EnableTraceEx2` trong `src/fps.rs` (mục "Chưa chắc" của `.delegate/task-19.result.md`).
- Lịch sử các bản: 1.3.0 báo cáo trận + OTA + giữ con cháu app; 1.3.1 giữ Discord/OBS, không dừng sshd; 1.3.2 sửa tốn CPU/GPU, version trên title bar; 1.3.3 như trên.

## FPS / frametime trong báo cáo trận (1.3.3)
- Không dùng PresentMon: bản 2.5.1/2.6.0 trên PC báo hàng chục nghìn "ETW events were lost" và không ghi được frame nào. Thay bằng tự đọc ETW provider **Microsoft-Windows-DXGI** (`CA11C036-0102-4A2D-A6AD-F03CFED5D3C9`), event **178** (task 80 `IDXGISwapChain_Present`); frametime = khoảng cách giữa hai event 178 cùng PID. Session tên `fps::DEFAULT_SESSION`, mở được cả khi không có quyền admin (đã thử trên PC).
- Lọc theo **mọi PID trùng tên exe game** (`report::Live::game_pids`, dò lại mỗi mẫu 5 giây). Chromium/Edge/Discord vẽ qua DirectComposition nên KHÔNG phát event 178 — đừng dùng chúng để thử.
- `Sample`: `fps`, `fps_low1`, `frametime_p99_ms`, `frametime_max_ms`, `hitches`. `Summary`: `fps_avg`, `fps_low1`, `fps_low01`, `frametime_p99_ms`, `frametime_max_ms`, `hitches` (số chính xác từ histogram 0,1 ms của `FpsMeter::total()` lúc `finish`; bản tạm/khôi phục thì tính gần đúng từ mẫu). Field mới đều `#[serde(default)]`, báo cáo cũ vẫn đọc được.
- Event `hitch` khi frame > 100 ms (cách nhau ≥ 30 s); `fps_off` khi không mở được session. Nhận xét nói rõ khi khựng trùng lúc xả standby (≤ 5 s).
- UI: thẻ "FPS TB / 1% low", "Khựng > 100 ms" (12 thẻ), biểu đồ FPS full width trên 4 biểu đồ cũ, chấm đỏ = khựng; danh sách trận ghi thêm "· 191 FPS".
- Làm bởi harness: task-19 (fps.rs), task-20 (lưu tạm báo cáo), task-21 (nối vào báo cáo + UI), DeepSeek Flash.
- **Hiệu năng (1.3.2)**: eframe 0.29 quay 100% một nhân khi cửa sổ bị `SW_HIDE` (cửa sổ ẩn không nhận WM_PAINT, event loop kẹt ở `ControlFlow::Poll`). Vì vậy "thu vào khay" = thu nhỏ + `WS_EX_TOOLWINDOW` (`sys::hide_main_window`), **không dùng SW_HIDE**. Khi mất focus / thu nhỏ, `update()` chỉ repaint 1–2 giây/lần, hiệu ứng nhấp nháy đi qua `ui::anim_repaint` (không repaint liên tục), và không gọi `schtasks`. Đo 2026-10-22: 1.3.1 lúc đang chơi ~1,2 nhân + ~9% GPU 3D; 1.3.2 mọi trạng thái < 1% một nhân, GPU ~0.
- **Tab "Báo cáo trận" (task-14, Gemini)** cùng banner cảnh báo commit và biểu đồ `egui_plot` 0.29. Các bước đã làm:
  1. ~~Kiểm tra tab "Báo cáo trận"~~ — đã xem trên PC ngày 2026-10-22: danh sách, thẻ chỉ số, nhận xét, 4 biểu đồ, trận đang ghi cập nhật trực tiếp đều đúng. Nút xoá hiện bước "Bấm lại để xoá" nhưng **chưa xác nhận lần bấm thứ hai xoá thật** (chỉ test bằng click giả).
  2. Đã sửa trên PC (task-15 Gemini + vài dòng tay) và đã commit lên `main`; PC giờ có bản clone ở `I:\dev\GameMode` (ngoài thư mục đồng bộ): nhãn trục Y không còn bị mất, trục X dạng `m:ss`, chú thích chuyển ra hàng trên biểu đồ (chấm tự vẽ vì font không có glyph `●`), tên game thân thiện ("Delta Force"), thêm 4 unit test (`cargo test` cần `__COMPAT_LAYER=RunAsInvoker` vì exe test cũng đòi admin).
  3. Còn lại: thử nút xoá báo cáo bằng tay.
- ~~**Tính năng còn trong danh sách đề xuất**: quản lý app khởi động cùng Windows, hồ sơ theo game, phím tắt toàn cục~~ — cả ba đã làm xong ở task-22 → task-26 (xem mục "Khởi động cùng Windows", "Phím tắt toàn cục", "Hồ sơ theo game" dưới).
- **Ngoài phần mềm (đã khuyên chủ máy)**: thêm thanh RAM 16 GB (đang single channel); đặt pagefile cố định 16–24 GB (commit từng chạm 95%); cắm dây LAN thay Wi-Fi; driver NVIDIA hiện tên "GTX 750 Ti" trong khi card thật là RTX 3060 Laptop (DEV_2520), nên cài lại driver chính hãng nếu FPS bất thường.
- **Lưu ý khi giao việc**: launcher `delegate.sh` hay thoát sớm khi chạy nền lâu, nhưng worker trên harness vẫn chạy tiếp; theo dõi bằng file `task-NN.status.json` (`"terminal": true` là xong).

## Build và chạy
```
cargo build --release
```
- Exe nằm ở `target/release/gamemode.exe`. Laptop giờ dùng bản cài ở `%LOCALAPPDATA%\Programs\GameMode\GameMode.exe` (cập nhật qua OTA hoặc chạy bộ cài).
- Manifest yêu cầu quyền admin (`highestAvailable`).
- Các file `gamemode.cfg`, `session.json`, `session.last.json` và `gamemode-crash.log` nằm cạnh exe. `.gitignore` đã loại chúng khỏi repo.
- Unit test phần THUẦN (chạy được trên Linux, không cần Windows): `sh .tools/task26test/extract.sh && cd .tools/task26test && cargo test --offline` — `extract.sh` cắt nguyên văn vùng `BEGIN/END PURE-PROFILE-HOTKEY` của `src/main.rs` (hồ sơ theo game, phím tắt, `effective_opts`) rồi nạp kèm `src/hotkey.rs`. Type-check Windows: `CARGO_TARGET_DIR=target/w26 CC_x86_64_pc_windows_gnu=gcc CFLAGS_x86_64_pc_windows_gnu=-m64 cargo check --target x86_64-pc-windows-gnu --offline --all-targets`.
- Feature `windows-sys` phải có trong `Cargo.toml`: `Win32_UI_Input_KeyboardAndMouse` (phím tắt), `Win32_UI_Shell` + `Win32_System_Com` (đích file `.lnk` của thư mục Startup). Thiếu là `hotkey.rs` / `startup.rs` không biên dịch được.

## Cờ dòng lệnh để kiểm tra (không tắt app nào)
| Cờ | Làm gì | Kết quả ghi ra |
|---|---|---|
| `--selftest` | Đọc mọi process, RAM free/standby, Wi-Fi autoconf, Game DVR, UserGpuPreferences, task Scheduler, config | `selftest.txt` |
| `--restore` | Khôi phục phiên Game Boost bị bỏ lửng mà **không mở UI** (mở lại app, service, Wi-Fi, Game DVR...). Từ chối nếu còn game trong danh sách đang chạy. Exit code 0 = xong, 1 = lỗi / không có `session.json` | `restore.txt` |
| `--purgetest` | Xả standby list một lần | `purgetest.txt` |
| `--launchtest` | Mở notepad bằng quyền user thường (token explorer) | `launchtest.txt` |
| `--tasktest` | Tạo task "GameMode", đọc XML, rồi xoá | `tasktest.txt` |
| `--traytest` | Tạo icon khay + menu trong 5 giây rồi thoát | `traytest.txt` |
| `--pingtest` | Ping gateway (`GetBestRoute`) và host internet 5 lần mỗi đích (IP, RTT, mất gói) | `pingtest.txt` |
| `--reporttest` | Chạy recorder giả lập 20 giây (mẫu mỗi 2 giây, không cần game, cho phép trận < 60 s) và lưu một báo cáo `game = "test"` | `reporttest.txt` |
| `--updatetest` | Kiểm tra manifest cập nhật; nếu có bản mới thì tải và kiểm SHA-256 (KHÔNG chạy bộ cài) | `updatetest.txt` |
| `--fpstest [giây]` | Đo FPS / frametime của game (theo danh sách game trong config) đang chạy, mặc định 10 giây | `fpstest.txt` |
| `--hwinfo` | Đọc phần cứng (WMI/PowerShell, chỉ đọc): dòng tóm tắt, từng gợi ý, rồi JSON thô. Exit 0 = đọc được, 1 = lỗi | `hwinfo.txt` |
| `--tweakstatus` | Trạng thái từng tinh chỉnh Windows, mỗi dòng `id \| group \| risk \| state \| có backup`. Exit 1 nếu không đọc được trạng thái nào | `wintweaks-status.txt` |
| `--tweakapply <id>` | Bật MỘT tinh chỉnh (cùng hàm + cùng file sao lưu như UI; in trạng thái trước/sau). Exit 0/1 | `wintweaks-result.txt` |
| `--tweakrevert <id>` | Hoàn tác MỘT tinh chỉnh về giá trị gốc (cùng hàm như UI). Exit 0/1 | `wintweaks-result.txt` |
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
- `src/fps.rs`: đo FPS / frametime bằng ETW DXGI (`FpsMeter`: start/set_target_pids/take_window/total/stop), phần tính toán thuần có unit test.
- `src/tray.rs`: icon khay + menu chuột phải, thread nhận sự kiện menu/click (chạy được khi cửa sổ đã ẩn).
- `src/startup.rs`: mục khởi động cùng Windows (`list_startup`, `set_enabled`, `StartupEntry`, `StartupLocation`) — đọc `Run` HKCU/HKLM/WOW6432Node + 2 thư mục Startup, bật/tắt bằng cờ `StartupApproved` giống Task Manager (không xoá mục). File `.lnk` trong thư mục Startup được đọc đích thật bằng `IShellLinkW`/`IPersistFile` (`CoInitializeEx` trên thread gọi, `CoUninitialize` sau); lỗi thì `command` = đường dẫn `.lnk`, `exe_path` = `None`.
- `src/hotkey.rs`: phím tắt toàn cục (`Hotkey::parse/display`, `HotkeyAction`, `HotkeyService::start/rebind/errors`, `default_bindings`) — thread message loop riêng + `RegisterHotKey(null, …)` như `tray.rs`, vì `update()` không chạy khi cửa sổ ẩn.
- `src/ui.rs`: bảng màu và widget dùng chung (`card`, `section_header`, `toggle_row` có switch, `ghost_button`, `primary_button`, `callout_warning`, `badge` cho nhãn nhỏ, `dot` cho chấm màu).
- `src/wintweaks.rs`: tinh chỉnh Windows lâu dài (`all_tweaks`, `Tweak`, `Group`, `Risk`, `state`, `apply`, `revert`, `Backup::load/save`, `summary`, `recommended_ids`) — mọi thay đổi đều sao lưu giá trị gốc và hoàn tác được; phần đụng registry/powercfg/bcdedit/PowerShell nằm sau `#[cfg(windows)]`, phần thuần có unit test chạy trên Linux.
- `src/hwinfo.rs`: chẩn đoán phần cứng (`collect`, `parse`, `advice`, `summary_line`, `Advice`, `Level`) — một script PowerShell trả JSON, luật gợi ý (kênh RAM, sai tên driver NVIDIA, tuổi driver, ổ C, tần số màn hình, pin…) xếp Warn → Info → Good.

## Đã xong
- **Khởi động cùng Windows (task-22, 26)**: tab **🚀 Khởi động** ở cột phải liệt kê mọi mục Run/Startup (tên đậm, nơi lưu, command rút gọn + tooltip đầy đủ) kèm switch bật/tắt; đọc ở thread nền khi mở tab lần đầu và khi bấm *Làm mới*; dòng đếm "N mục, M đang bật"; mục trỏ đúng exe GameMode đang chạy bị **khoá switch** (tắt nó là tự tắt app — việc chạy cùng Windows do task lo).
- **Tự sửa task chạy cùng Windows (task-24, 26)**: lúc mở app, thread nền gọi `sys::task_needs_repair()`; task cũ (`/SC ONLOGON`: chỉ chạy khi cắm sạc, tự chết sau 72 giờ) hoặc trỏ sai exe thì `sys::task_create()` lại và log `✔ [TASK] Đã cập nhật task GameMode: trỏ đúng exe, chạy cả khi dùng pin.` Nút bật task trong UI cũng dùng `task_create` (XML) mới.
- **Phím tắt toàn cục (task-23, 26)**: `hotkey:enabled` (mặc định 1) + `hotkey:boost|restore|window` (mặc định `Ctrl+Alt+B/R/G`; **chuỗi rỗng = tắt phím đó**). Đăng ký trong `App::new`, chạy trên thread phím tắt; Boost/Restore làm y như mục menu khay, "Hiện/ẩn cửa sổ" dùng `sys::toggle_main_window` (thu nhỏ + `WS_EX_TOOLWINDOW`, **không SW_HIDE**). Mỗi lần bấm ghi một dòng `⌨ [PHÍM TẮT] …`. Card **Phím tắt** (dưới card Cập nhật OTA ở tab Tự động & Tối ưu): công tắc chung, mỗi hành động một ô nhập + nút *Lưu* (parse lỗi thì hiện đỏ ngay dưới ô, không lưu), hiện luôn lỗi đăng ký (`errors()` — vd tổ hợp đã bị app khác chiếm).
- **Hồ sơ theo game (task-26)**: `profile:<exe>:<key>=1/0` với key `standby, trim, gpu, wlan, gamedvr, report, close_apps, stop_services`; **không có dòng = theo cài đặt chung**. Tự Boost do game X mở dùng hồ sơ X; Boost tay (nút / khay / phím tắt) chỉ dùng hồ sơ khi **đúng một** game trong danh sách đang chạy. `close_apps=0` thì Boost không tắt app nào và **không ghi app vào session**; `stop_services=0` thì không dừng service và không ghi service. Log một dòng `• [HỒ SƠ] Dùng hồ sơ <tên game>: …`. UI: nút **Hồ sơ** trên từng dòng game mở khối 8 tuỳ chọn, mỗi tuỳ chọn 3 lựa chọn **Chung / Bật / Tắt** ("Chung" = không ghi dòng config); game có hồ sơ riêng hiện nhãn nhỏ "hồ sơ riêng". Xoá game khỏi danh sách thì xoá luôn hồ sơ của nó (`write_config` cũng tự bỏ hồ sơ của game không còn trong danh sách); game trong `NOT_GAMES` bị bỏ qua. Hàm thuần `effective_opts()` có unit test.
- Tên hiển thị là **Paroda GameMode** (title bar + log READY). Tên game hiển thị ("Delta Force"), task `GameMode`, mutex và tên file giữ nguyên.
- Phiên Game Boost bị bỏ lửng (app bị tắt giữa phiên, `session.json` còn lại): lần mở sau nếu game trong danh sách **đang chạy** thì watcher tiếp tục phiên đó (không Boost lần nữa, game tắt vẫn tự khôi phục theo `auto:restore`); nếu **không có game nào chạy** thì bật banner cột phải "Còn phiên Game Boost chưa khôi phục…" với nút *Khôi phục ngay* / *Để sau*, hiện cửa sổ dù có `--minimized`, và watcher không tự Boost cho tới khi phiên được xử lý. Cờ `--restore` khôi phục không cần UI.
- Danh sách cấm `NOT_GAMES`: exe hệ thống / app luôn chạy (WebView2, trình duyệt, explorer, svchost, steam, obs64, claude, node, gamemode...) và mọi exe bắt đầu bằng `discord` không vào được danh sách game — dòng `game:` cũ trong config bị bỏ qua (lần ghi sau tự xoá), UI thêm game và "Đánh dấu là game" đều bị chặn kèm thông báo.
- Danh sách app đang chạy có công tắc "Giữ lại", lựa chọn nhớ trong `keep:` của config.
- Mặc định giữ lại: mọi exe bắt đầu bằng `discord` (gồm DiscordSystemHelper — cần cho livestream), OBS (`obs64.exe`), Steam, JBL Quantum, Tailscale, WARP, UniKey/EVKey, Claude, NVIDIA, webview2, NitroSense, game và anti-cheat ACE. Discord/OBS cũng không bị nén RAM (giật stream).
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
- OpenSSH Server (`sshd`) không còn trong danh sách service bị dừng (từ sau 1.3.0): Game Boost không cắt SSH vào máy; dòng `svc:sshd=` cũ trong config bị bỏ qua. `ssh.exe` do Claude mở được giữ vì là con cháu của `claude.exe`.
- Auto mode mặc định tắt, phải tự bật trong UI.
- Chạy quyền thường (vd mở bằng `__COMPAT_LAYER=RunAsInvoker` khi test) thì không xả được standby; lỗi này chỉ log một lần mỗi lần mở app, kèm gợi ý mở lại bằng quyền admin.
- `--selftest` ghi `kept_children` / `kept_by` ở mỗi dòng `APP` để kiểm quy tắc giữ con cháu của app được giữ.
- Khi tắt quét Wi-Fi, nếu mạng rớt thì máy không tự kết nối lại cho tới khi Restore.

## Bộ cài (Inno Setup)
- Cài Inno Setup 6: `winget install JRSoftware.InnoSetup --scope user`.
- Build: `cargo build --release`, rồi `"%LOCALAPPDATA%\Programs\Inno Setup 6\ISCC.exe" installer\gamemode.iss`, ra file `dist\GameMode-Setup-<version>.exe` (thư mục `dist/` không đưa lên git).
- Version (hiện 1.5.0) phải sửa ở 2 chỗ: `Cargo.toml` và `#define AppVersion` trong `installer/gamemode.iss`.
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
  Vòng cài thật 1.1.9 → 1.2.0 qua kênh thử đã chạy trọn trên PC (app thoát, bộ cài `/VERYSILENT /UPDATE`, tự mở lại, log "Đã cập nhật"), nhưng lúc đó app chạy **quyền thường**; vòng cài khi app chạy quyền admin chưa thử.
  Kênh thật `apps/gamemode` có **1.3.0** (publish 2026-10-22). Changelog có dấu `"` thì gọi script ngay trong phiên PowerShell (`& .\scripts\publish_update.ps1 ...`), đừng qua `powershell -File` (PS 5.1 tách sai tham số).
