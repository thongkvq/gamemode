#![windows_subsystem = "windows"]

mod report;
mod sys;
mod tray;
mod ui;

use eframe::egui::{self, Color32, FontId, Margin, Rect, RichText, Rounding, Stroke, pos2, vec2};
use egui_plot::{HLine, Line, LineStyle, MarkerShape, Plot, PlotPoints, Points, VLine};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// (Tên service Windows, Tên hiển thị, Lý do tạm tắt khi chơi game, Mặc định có tắt không)
const SERVICE_TARGETS: &[(&str, &str, &str, bool)] = &[
    (
        "wuauserv",
        "Windows Update",
        "Tránh tự tải cập nhật ngầm làm drop FPS, tụt ping",
        true,
    ),
    (
        "Spooler",
        "Print Spooler (Máy in)",
        "Dịch vụ máy in, hoàn toàn không cần khi chơi game",
        true,
    ),
    (
        "ClickToRunSvc",
        "Microsoft Office Click-to-Run",
        "Dịch vụ nền Office, chiếm ~80-100 MB RAM ngầm",
        true,
    ),
    (
        "InventorySvc",
        "Compatibility Appraisal",
        "Quét tương thích phần mềm Windows, tốn CPU & Disk",
        true,
    ),
    (
        "lfsvc",
        "Geolocation Service",
        "Dịch vụ định vị GPS / vị trí máy tính",
        true,
    ),
    (
        "TrkWks",
        "Distributed Link Tracking",
        "Theo dõi liên kết file mạng LAN, tốn I/O đĩa",
        true,
    ),
    (
        "sshd",
        "OpenSSH Server",
        "Máy chủ SSH nhận kết nối từ xa",
        true,
    ),
    (
        "WpcMonSvc",
        "Parental Controls",
        "Giám sát trẻ em ngầm của Windows",
        true,
    ),
    (
        "PhoneSvc",
        "Phone Service",
        "Dịch vụ điện thoại chạy ngầm",
        true,
    ),
    (
        "UsoSvc",
        "Update Orchestrator",
        "Dịch vụ điều phối tiến trình update Windows",
        true,
    ),
    (
        "CodexSandboxService.OpenAI.Codex",
        "ChatGPT Daemon",
        "Dịch vụ ChatGPT chạy nền",
        true,
    ),
    (
        "Tailscale",
        "Tailscale VPN",
        "VPN Mesh Tailscale (tắt để giảm RAM và ping)",
        false,
    ),
    (
        "CloudflareWARP",
        "Cloudflare WARP",
        "VPN Cloudflare (tắt để tránh đổi route mạng game)",
        false,
    ),
];

/// Process KHÔNG BAO GIỜ bị tắt và không xuất hiện trong danh sách chọn.
/// (Tool tự loại thêm chính nó và process cha của nó theo PID, xem `scan_running_apps`.)
const NEVER_KILL: &[&str] = &[
    // Bản GameMode khác đang mở (chạy thử, chạy song song) cũng không được tự tắt nhau.
    "gamemode.exe",
    "explorer.exe",
    "dwm.exe",
    "csrss.exe",
    "winlogon.exe",
    "wininit.exe",
    "lsass.exe",
    "services.exe",
    "smss.exe",
    "svchost.exe",
    "fontdrvhost.exe",
    "sihost.exe",
    "ctfmon.exe",
    "conhost.exe",
    "taskhostw.exe",
    "runtimebroker.exe",
    "shellexperiencehost.exe",
    "startmenuexperiencehost.exe",
    "searchhost.exe",
    "textinputhost.exe",
    "securityhealthsystray.exe",
    "dllhost.exe",
    "audiodg.exe",
    "system",
    "registry",
    "idle",
    "msmpeng.exe",
    "nissrv.exe",
    "lockapp.exe",
    "applicationframehost.exe",
    "systemsettings.exe",
    "useroobebroker.exe",
    "wmiprvse.exe",
];

/// App ĐÓNG GÓI nhưng là app người dùng thật: nằm trong `C:\Windows\SystemApps\` hoặc
/// `C:\Windows\WindowsApps\` (Widgets, Phone Link, Copilot, Teams...). Mặc định mọi exe
/// nằm trong `C:\Windows\` đều bị loại khỏi danh sách; riêng các tên dưới đây được phép
/// hiện ra và tắt vì chúng chiếm RAM đáng kể mà không phải thành phần lõi của Windows.
const PACKAGED_USER_APPS: &[&str] = &[
    "widgets.exe",
    "widgetservice.exe",
    "phoneexperiencehost.exe",
    "crossdeviceservice.exe",
    "yourphoneappproxyhost.exe",
    "crossdeviceresume.exe",
    "copilot.exe",
    "ms-teams.exe",
    "teams.exe",
];

/// App mặc định được GIỮ LẠI (không tắt) khi config chưa có dòng `keep:` cho nó.
const DEFAULT_KEEP: &[&str] = &[
    "discord.exe",
    "steam.exe",
    "steamwebhelper.exe",
    "warp-svc.exe",
    "cloudflare warp.exe",
    "unikeynt.exe",
    "unikey.exe",
    "evkey64.exe",
    "evkey.exe",
    "claude.exe",
    "nvcontainer.exe",
    "nvidia overlay.exe",
    "nvidia app.exe",
    "windowsterminal.exe",
    "openconsole.exe",
    "msedgewebview2.exe",
    // NitroSense (Acer): quạt & profile hiệu năng, phải giữ để máy không bị khóa quạt.
    "psagent.exe",
    "psadminagent.exe",
];

/// Exe game mặc định (dùng khi config chưa có dòng `game:` nào).
const DEFAULT_GAMES: &[&str] = &["deltaforceclient-win64-shipping.exe"];
/// Tiền tố tên exe mặc định được giữ (Tailscale: tailscale.exe, tailscaled.exe...).
const DEFAULT_KEEP_PREFIX: &[&str] = &["tailscale", "deltaforce", "df_garena", "sguard", "unrealcef", "ace-", "gamebar"];
/// Chuỗi con trong tên exe mặc định được giữ (JBL Quantum).
const DEFAULT_KEEP_CONTAINS: &[&str] = &["quantum", "-win64-shipping", "无极画质"];

/// Process anti-cheat ACE của Delta Force. Cùng với exe game, đây là nhóm TUYỆT ĐỐI chỉ được
/// quan sát tên: không mở quyền đọc bộ nhớ, không đọc command line / thư mục làm việc.
const ACE_SKIP: &[&str] = &[
    "sguard64.exe",
    "sguard.exe",
    "sguardsvc64.exe",
    "ace-",
    "anticheatexpert.exe",
];

/// Process hệ thống KHÔNG nén working set: nén audiodg (âm thanh) / dwm (compositor, vẽ hình)
/// / csrss sẽ gây giật âm thanh và hình ngay khi đang chơi.
const TRIM_SKIP_EXE: &[&str] = &["audiodg.exe", "dwm.exe", "csrss.exe"];

/// Danh sách exe không được mở quyền đọc bộ nhớ: exe game + process anti-cheat.
fn skip_list(games: &[String]) -> Vec<String> {
    let mut v: Vec<String> = games.iter().map(|g| g.to_lowercase()).collect();
    v.extend(ACE_SKIP.iter().map(|s| s.to_string()));
    v
}

/// Tên exe (viết thường) có nằm trong nhóm game / anti-cheat không.
fn is_ace_or_game(name_lower: &str, skip: &[String]) -> bool {
    skip.iter()
        .any(|s| name_lower == s.as_str() || name_lower.starts_with(s.as_str()))
}

/// Launcher đã biết của game. Khi game KHÔNG chạy, chính launcher (và con cháu của nó) cũng
/// thuộc "họ" process game nên không được tắt.
const GAME_LAUNCHERS: &[&str] = &["df_garena_launcher.exe"];

/// Process DỪNG khi đi ngược ppid từ process game: shell của người dùng và các process
/// Windows luôn chạy. Bản thân những process này KHÔNG được bảo vệ.
const FAMILY_STOP_EXE: &[&str] = &["explorer.exe", "svchost.exe", "services.exe", "wininit.exe"];

/// Vì sao một PID thuộc "họ" process của game (thứ tự này cũng là thứ tự liệt kê).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum FamilyReason {
    Game,
    Ancestor,
    Descendant,
    Launcher,
}

impl FamilyReason {
    fn label(self) -> &'static str {
        match self {
            FamilyReason::Game => "game",
            FamilyReason::Ancestor => "tổ tiên",
            FamilyReason::Descendant => "con cháu",
            FamilyReason::Launcher => "launcher",
        }
    }
}

/// Tập PID được bảo vệ của "họ" process game: mọi process có tên exe trong danh sách game,
/// tổ tiên của chúng, rồi TOÀN BỘ con cháu của những PID đã bảo vệ. Khi game không chạy:
/// process có exe trong `GAME_LAUNCHERS` cùng con cháu của nó.
fn game_family(snapshot: &[sys::ProcInfo], games: &[String]) -> HashSet<u32> {
    game_family_reasons(snapshot, games).into_keys().collect()
}

/// Như `game_family` nhưng kèm lý do bảo vệ của từng PID (dùng cho log và --selftest).
fn game_family_reasons(snapshot: &[sys::ProcInfo], games: &[String]) -> HashMap<u32, FamilyReason> {
    let nodes: Vec<sys::ProcNode> = snapshot
        .iter()
        .map(|p| sys::ProcNode {
            pid: p.pid,
            ppid: p.ppid,
            name: p.name.to_lowercase(),
        })
        .collect();
    game_family_from_nodes(&nodes, games)
}

/// Lõi tính "họ" process game trên cây process nhẹ (Toolhelp32) — xem `game_family`.
fn game_family_from_nodes(nodes: &[sys::ProcNode], games: &[String]) -> HashMap<u32, FamilyReason> {
    let by_pid: HashMap<u32, &sys::ProcNode> = nodes.iter().map(|n| (n.pid, n)).collect();
    let game_names: HashSet<String> = games.iter().map(|g| g.to_lowercase()).collect();
    let launcher_names: HashSet<&str> = GAME_LAUNCHERS.iter().copied().collect();

    let mut fam: HashMap<u32, FamilyReason> = HashMap::new();

    // 1. Chính process game (khớp đúng tên exe trong danh sách game).
    for n in nodes.iter() {
        if game_names.contains(&n.name) {
            fam.entry(n.pid).or_insert(FamilyReason::Game);
        }
    }

    // 2. Tổ tiên của process game: đi ngược ppid tới khi gặp process dừng, pid <= 4, hoặc
    //    process đã chết / không còn trong snapshot. Chính process dừng không được bảo vệ.
    if !fam.is_empty() {
        let mut seen: HashSet<u32> = fam.keys().copied().collect();
        let mut queue: Vec<u32> = seen.iter().copied().collect();
        while let Some(pid) = queue.pop() {
            let Some(node) = by_pid.get(&pid) else { continue };
            let ppid = node.ppid;
            // `seen.insert` cũng chặn vòng lặp khi PID bị tái sử dụng (A là cha của B và ngược lại).
            if ppid <= 4 || ppid == pid || !seen.insert(ppid) {
                continue;
            }
            let Some(parent) = by_pid.get(&ppid) else { continue };
            if FAMILY_STOP_EXE.contains(&parent.name.as_str()) {
                continue;
            }
            fam.entry(ppid).or_insert(FamilyReason::Ancestor);
            queue.push(ppid);
        }
    }

    // 3. Game không chạy: launcher đã biết cũng thuộc họ game.
    if fam.is_empty() {
        for n in nodes.iter() {
            if launcher_names.contains(n.name.as_str()) {
                fam.entry(n.pid).or_insert(FamilyReason::Launcher);
            }
        }
    }

    // 4. Toàn bộ con cháu của mọi PID đã bảo vệ (duyệt theo ppid). Mỗi PID chỉ được thêm một
    //    lần nên vòng lặp do PID tái sử dụng cũng không thể chạy mãi.
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for n in nodes.iter() {
        if n.ppid != n.pid {
            children.entry(n.ppid).or_default().push(n.pid);
        }
    }
    let mut queue: Vec<u32> = fam.keys().copied().collect();
    while let Some(pid) = queue.pop() {
        let Some(kids) = children.get(&pid) else { continue };
        for &kid in kids.iter() {
            if fam.contains_key(&kid) {
                continue;
            }
            fam.insert(kid, FamilyReason::Descendant);
            queue.push(kid);
        }
    }

    fam
}

/// Chuỗi (viết thường) trong command line chứng tỏ process là process PHỤ do app khác mở:
/// COM server, renderer/GPU helper của Chrome, helper do Chrome/extension mở, hay helper
/// do launcher game mở. Loại khi lưu session và bỏ qua khi Restore.
/// Exe không bao giờ mở lại khi Restore: shell / trình thông dịch (chạy lại lệnh cũ là nguy hiểm)
/// và agent nền do dịch vụ của hãng tự bật lại.
const NEVER_RESTORE_EXE: &[&str] = &[
    "bash.exe", "sh.exe", "cmd.exe", "powershell.exe", "pwsh.exe", "wsl.exe", "conhost.exe",
    "python.exe", "pythonw.exe", "py.exe", "node.exe", "git.exe", "ssh.exe",
    "defendersessionhelper.exe", "psagent.exe", "psadminagent.exe", "service.exe",
];

fn is_never_restore_exe(exe: &str) -> bool {
    NEVER_RESTORE_EXE.contains(&exe.to_lowercase().as_str())
}

const CHILD_CMDLINE_MARKERS: &[&str] = &[
    "--type=",
    "-embedding",
    "/embedding",
    "chrome-extension://",
    r"\\.\pipe\",
    "--launcherpid",
];

const GB: f64 = 1024.0 * 1024.0 * 1024.0;
const MB: f64 = 1024.0 * 1024.0;

/// Tiêu đề cửa sổ chính: dùng cho `ViewportBuilder` và cho `FindWindowW` khi bản chạy
/// thứ hai cần tìm lại cửa sổ của bản đang chạy.
const WINDOW_TITLE: &str = "DELTA FORCE - TACTICAL RAM PURGE";

fn config_path() -> PathBuf {
    std::env::current_exe()
        .map(|p| p.with_file_name("gamemode.cfg"))
        .unwrap_or_else(|_| PathBuf::from("gamemode.cfg"))
}

/// Session khôi phục nằm cạnh file config.
fn session_path() -> PathBuf {
    config_path().with_file_name("session.json")
}

/// App mặc định giữ lại theo tên exe (viết thường).
fn default_keep(exe_lower: &str) -> bool {
    DEFAULT_KEEP.contains(&exe_lower)
        || DEFAULT_KEEP_PREFIX.iter().any(|p| exe_lower.starts_with(p))
        || DEFAULT_KEEP_CONTAINS.iter().any(|s| exe_lower.contains(s))
}

/// Quyết định giữ/tắt cho một tên exe: config đã lưu thắng, chưa có thì dùng mặc định.
fn keep_decision(keep: &HashMap<String, bool>, exe_lower: &str) -> bool {
    keep.get(exe_lower).copied().unwrap_or_else(|| default_keep(exe_lower))
}

/// Process hệ thống / shell / tool: luôn bị loại khỏi danh sách.
fn is_never_kill(exe_lower: &str, exe_path: Option<&str>) -> bool {
    if NEVER_KILL.contains(&exe_lower) {
        return true;
    }
    if let Some(p) = exe_path {
        let lower = p.to_lowercase();
        if lower.starts_with(r"c:\windows\") {
            let packaged = lower.contains(r"\systemapps\") || lower.contains(r"\windowsapps\");
            if !(packaged && PACKAGED_USER_APPS.contains(&exe_lower)) {
                return true;
            }
        }
    }
    false
}

/// Tuỳ chọn tối ưu khi chơi (khối "Tối ưu khi chơi" + khối "Tự động khi chơi game").
#[derive(Clone, Copy)]
struct Opts {
    /// Xả standby list khi tối ưu và khi RAM trống thấp lúc đang chơi.
    standby: bool,
    /// Nén working set của các app được giữ lại (một lần, lúc tối ưu).
    trim: bool,
    /// Ép game dùng GPU rời (GpuPreference=2).
    gpu: bool,
    /// Tắt quét nền Wi-Fi khi chơi.
    wlan: bool,
    /// Tắt Game DVR / App Capture khi chơi.
    gamedvr: bool,
    /// Tự tối ưu khi phát hiện game mở.
    auto_enabled: bool,
    /// Tự Restore khi game đã tắt.
    auto_restore: bool,
    /// `ui:close_to_tray`: bấm X thì ẩn xuống khay thay vì thoát.
    close_to_tray: bool,
    /// `report:enabled`: ghi số liệu và lưu báo cáo sau mỗi trận.
    report_enabled: bool,
}

impl Default for Opts {
    fn default() -> Self {
        Self {
            standby: true,
            trim: true,
            gpu: true,
            wlan: true,
            gamedvr: true,
            auto_enabled: false,
            auto_restore: true,
            close_to_tray: true,
            report_enabled: true,
        }
    }
}

/// Toàn bộ nội dung gamemode.cfg.
struct Config {
    keep: HashMap<String, bool>,
    svcs: Vec<bool>,
    /// Danh sách exe game (viết thường).
    games: Vec<String>,
    /// Đường dẫn exe đã biết của từng game: exe (viết thường) -> đường dẫn đầy đủ.
    game_paths: HashMap<String, String>,
    /// Host ping internet của báo cáo trận (`report:ping_host`).
    ping_host: String,
    opts: Opts,
}

fn is_on(v: &str) -> bool {
    v.trim() == "1"
}

/// Đọc cấu hình: `keep:<exe>=1/0`, `svc:<tên>=1/0`, `game:<exe>=1`, `gamepath:<exe>=<path>`,
/// `auto:enabled/restore=1/0`, `opt:standby/trim/gpu/wlan/gamedvr=1/0`,
/// `ui:close_to_tray=1/0`, `report:enabled=1/0`, `report:ping_host=<host>`.
/// Các dòng `app:...` của bản cũ được bỏ qua (không còn danh sách app cố định).
fn load_config() -> Config {
    let text = std::fs::read_to_string(config_path()).unwrap_or_default();
    let mut keep_saved: HashMap<String, bool> = HashMap::new();
    let mut svc_saved: HashMap<String, bool> = HashMap::new();
    let mut games: Vec<String> = Vec::new();
    let mut game_paths: HashMap<String, String> = HashMap::new();
    let mut ping_host = report::PING_HOST_DEFAULT.to_string();
    let mut opts = Opts::default();

    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(rest) = line.strip_prefix("keep:") {
            if let Some((k, v)) = rest.rsplit_once('=') {
                keep_saved.insert(k.trim().to_lowercase(), is_on(v));
            }
        } else if let Some(rest) = line.strip_prefix("svc:") {
            if let Some((k, v)) = rest.rsplit_once('=') {
                svc_saved.insert(k.trim().to_string(), is_on(v));
            }
        } else if let Some(rest) = line.strip_prefix("gamepath:") {
            // Đường dẫn có thể chứa '=' -> cắt ở dấu '=' đầu tiên.
            if let Some((k, v)) = rest.split_once('=') {
                let exe = k.trim().to_lowercase();
                let path = v.trim().to_string();
                if !exe.is_empty() && !path.is_empty() {
                    game_paths.insert(exe, path);
                }
            }
        } else if let Some(rest) = line.strip_prefix("game:") {
            if let Some((k, v)) = rest.rsplit_once('=') {
                let exe = k.trim().to_lowercase();
                if is_on(v) && !exe.is_empty() && !games.contains(&exe) {
                    games.push(exe);
                }
            }
        } else if let Some(rest) = line.strip_prefix("opt:") {
            if let Some((k, v)) = rest.rsplit_once('=') {
                match k.trim() {
                    "standby" => opts.standby = is_on(v),
                    "trim" => opts.trim = is_on(v),
                    "gpu" => opts.gpu = is_on(v),
                    "wlan" => opts.wlan = is_on(v),
                    "gamedvr" => opts.gamedvr = is_on(v),
                    _ => {}
                }
            }
        } else if let Some(rest) = line.strip_prefix("auto:") {
            if let Some((k, v)) = rest.rsplit_once('=') {
                match k.trim() {
                    "enabled" => opts.auto_enabled = is_on(v),
                    "restore" => opts.auto_restore = is_on(v),
                    _ => {}
                }
            }
        } else if let Some(rest) = line.strip_prefix("ui:") {
            if let Some((k, v)) = rest.rsplit_once('=') {
                if k.trim() == "close_to_tray" {
                    opts.close_to_tray = is_on(v);
                }
            }
        } else if let Some(rest) = line.strip_prefix("report:") {
            if let Some((k, v)) = rest.rsplit_once('=') {
                match k.trim() {
                    "enabled" => opts.report_enabled = is_on(v),
                    "ping_host" => {
                        let host = v.trim();
                        if !host.is_empty() {
                            ping_host = host.to_string();
                        }
                    }
                    _ => {}
                }
            }
        }
        // dòng "app:..." / dòng rác của bản cũ: bỏ qua
    }

    if games.is_empty() {
        games = DEFAULT_GAMES.iter().map(|s| s.to_string()).collect();
    }

    let svcs = SERVICE_TARGETS
        .iter()
        .map(|(name, _, _, def)| svc_saved.get(*name).copied().unwrap_or(*def))
        .collect();

    Config {
        keep: keep_saved,
        svcs,
        games,
        game_paths,
        ping_host,
        opts,
    }
}

fn write_config(cfg: &Config) -> bool {
    let mut lines: Vec<String> = Vec::new();
    let mut keys: Vec<&String> = cfg.keep.keys().collect();
    keys.sort();
    for k in keys {
        lines.push(format!("keep:{}={}", k, if cfg.keep[k] { 1 } else { 0 }));
    }
    for ((name, _, _, _), on) in SERVICE_TARGETS.iter().zip(&cfg.svcs) {
        lines.push(format!("svc:{}={}", name, if *on { 1 } else { 0 }));
    }
    let mut games: Vec<&String> = cfg.games.iter().collect();
    games.sort();
    for exe in games {
        lines.push(format!("game:{}={}", exe, 1));
    }
    let mut paths: Vec<(&String, &String)> = cfg.game_paths.iter().collect();
    paths.sort_by(|a, b| a.0.cmp(b.0));
    for (exe, path) in paths {
        lines.push(format!("gamepath:{}={}", exe, path));
    }
    let o = &cfg.opts;
    lines.push(format!("auto:enabled={}", if o.auto_enabled { 1 } else { 0 }));
    lines.push(format!("auto:restore={}", if o.auto_restore { 1 } else { 0 }));
    lines.push(format!("opt:standby={}", if o.standby { 1 } else { 0 }));
    lines.push(format!("opt:trim={}", if o.trim { 1 } else { 0 }));
    lines.push(format!("opt:gpu={}", if o.gpu { 1 } else { 0 }));
    lines.push(format!("opt:wlan={}", if o.wlan { 1 } else { 0 }));
    lines.push(format!("opt:gamedvr={}", if o.gamedvr { 1 } else { 0 }));
    lines.push(format!("ui:close_to_tray={}", if o.close_to_tray { 1 } else { 0 }));
    lines.push(format!("report:enabled={}", if o.report_enabled { 1 } else { 0 }));
    lines.push(format!("report:ping_host={}", cfg.ping_host));
    std::fs::write(config_path(), lines.join("\n")).is_ok()
}

/// Một app sẽ được mở lại trong phiên Restore.
#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
struct SessionApp {
    /// Tên exe, ví dụ "chrome.exe"
    exe: String,
    /// Tên hiển thị (FileDescription) nếu lấy được
    #[serde(default)]
    display: String,
    /// Đường dẫn đầy đủ tới exe
    #[serde(default)]
    path: String,
    /// Command line đầy đủ của process gốc
    #[serde(default)]
    cmdline: String,
    /// Thư mục làm việc (best-effort)
    #[serde(default)]
    cwd: Option<String>,
}

/// Giá trị registry "trước khi tool sửa" của một tuỳ chọn Game DVR.
/// `value = None` nghĩa là value KHÔNG tồn tại lúc chụp -> Restore phải xoá nó đi.
#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
struct RegPrev {
    #[serde(default)]
    value: Option<u32>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
struct SessionData {
    #[serde(default)]
    saved_at: String,
    #[serde(default)]
    apps: Vec<SessionApp>,
    #[serde(default)]
    services: Vec<String>,
    /// GUID các interface Wi-Fi đã bị tắt quét nền (autoconf) trong phiên.
    #[serde(default)]
    wlan_autoconf_off: Vec<String>,
    /// Giá trị `GameDVR_Enabled` trước khi tool đặt về 0.
    #[serde(default)]
    gamedvr_prev: Option<RegPrev>,
    /// Giá trị `AppCaptureEnabled` trước khi tool đặt về 0.
    #[serde(default)]
    appcapture_prev: Option<RegPrev>,
}

impl SessionData {
    fn load() -> Option<Self> {
        let text = std::fs::read_to_string(session_path()).ok()?;
        serde_json::from_str(&text).ok()
    }

    fn save(&self) -> bool {
        match serde_json::to_string_pretty(self) {
            Ok(text) => std::fs::write(session_path(), text).is_ok(),
            Err(_) => false,
        }
    }

    /// Gộp thêm app/service vào session cũ (không ghi đè mất app của lần trước), loại trùng.
    fn merge(&mut self, apps: Vec<SessionApp>, services: Vec<String>) {
        let mut seen: HashSet<String> = self.apps.iter().map(app_key).collect();
        for a in apps {
            if a.path.is_empty() && a.cmdline.is_empty() {
                continue;
            }
            if seen.insert(app_key(&a)) {
                self.apps.push(a);
            }
        }
        for s in services {
            if !self.services.iter().any(|x| x.eq_ignore_ascii_case(&s)) {
                self.services.push(s);
            }
        }
    }

    /// Gộp danh sách GUID Wi-Fi đã tắt quét nền vào session (không ghi đè, loại trùng).
    fn merge_wlan(&mut self, guids: &[String]) {
        for g in guids {
            if !self
                .wlan_autoconf_off
                .iter()
                .any(|x| x.eq_ignore_ascii_case(g))
            {
                self.wlan_autoconf_off.push(g.clone());
            }
        }
    }
}

/// Khoá loại trùng: command line đã chuẩn hoá, không có thì dùng đường dẫn exe.
fn app_key(a: &SessionApp) -> String {
    let cmd = normalize_cmdline(&a.cmdline);
    if cmd.is_empty() {
        a.path.to_lowercase()
    } else {
        cmd
    }
}

/// Chuẩn hoá command line để so trùng: bỏ khoảng trắng thừa (đầu/cuối + gộp giữa) và hạ chữ thường.
fn normalize_cmdline(cmd: &str) -> String {
    cmd.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// Command line có dấu hiệu của process phụ (COM server / renderer / helper do app khác mở).
fn is_child_process_cmdline(cmdline: &str) -> bool {
    let lower = cmdline.to_lowercase();
    CHILD_CMDLINE_MARKERS.iter().any(|m| lower.contains(m))
}

/// Exe nằm trong `\WindowsApps\`: app đóng gói, mở lại bằng đường dẫn exe là không được.
fn is_packaged_path(path: &str) -> bool {
    path.to_lowercase().contains(r"\windowsapps\")
}

/// Đường dẫn exe có xuất hiện trong một lệnh tự khởi động (đã hạ chữ thường) không.
fn in_startup_entries(path: &str, startup_lower: &[String]) -> bool {
    let p = path.trim().to_lowercase();
    !p.is_empty() && startup_lower.iter().any(|c| c.contains(&p))
}

/// Một dòng trong danh sách app đang chạy (gom theo tên exe).
#[derive(Clone)]
struct RunningApp {
    exe_lower: String,
    exe_name: String,
    display: Option<String>,
    exe_path: Option<String>,
    count: usize,
    total_ws: u64,
}

fn load_app_icon() -> Option<egui::IconData> {
    let bytes = include_bytes!("../icon.png");
    let img = image::load_from_memory(bytes).ok()?.into_rgba8();
    let (width, height) = img.dimensions();
    Some(egui::IconData {
        rgba: img.into_raw(),
        width,
        height,
    })
}

/// HWND của cửa sổ chính, lấy từ `CreationContext` (eframe 0.29 dùng raw-window-handle 0.6).
/// Trả 0 nếu không lấy được; khi đó các lệnh ẩn/hiện cửa sổ chỉ đơn giản là không làm gì.
fn window_hwnd(cc: &eframe::CreationContext<'_>) -> isize {
    match cc.window_handle().map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Win32(w)) => w.hwnd.get(),
        _ => 0,
    }
}

fn extract_exe_path(raw_cmd: &str) -> Option<String> {
    let trimmed = raw_cmd.trim();
    if trimmed.starts_with('"') {
        let rest = &trimmed[1..];
        if let Some(end_idx) = rest.find('"') {
            return Some(rest[..end_idx].to_string());
        }
    }
    if let Some(idx) = trimmed.to_lowercase().find(".exe") {
        return Some(trimmed[..idx + 4].to_string());
    }
    Some(trimmed.split_whitespace().next().unwrap_or(trimmed).to_string())
}

fn fmt_ram(bytes: u64) -> String {
    if bytes as f64 >= GB {
        format!("{:.2} GB", bytes as f64 / GB)
    } else {
        format!("{:.0} MB", bytes as f64 / MB)
    }
}

/// Quét app đang chạy thuộc session của user hiện tại, gom theo tên exe, sắp xếp RAM giảm dần.
/// `skip` = exe game + anti-cheat: không mở quyền đọc bộ nhớ và không hiện trong danh sách
/// (game luôn được giữ lại nên không có gì để chọn). `games` = danh sách game trong config:
/// nhóm exe có PID thuộc "họ" process game cũng không hiện.
fn scan_running_apps(skip: &[String], games: &[String]) -> Vec<RunningApp> {
    let me = std::process::id();
    let my_session = sys::current_session();
    // Họ process game tính trước khi quét sâu: con cháu game (helper của launcher, CEF...)
    // cũng chỉ được mở bằng quyền hạn chế, không đọc bộ nhớ.
    let protect: HashSet<u32> = game_family_from_nodes(&sys::process_nodes(), games)
        .into_keys()
        .collect();
    let all = sys::snapshot_skip(skip, &protect);
    let family = game_family(&all, games);
    let parent = all.iter().find(|p| p.pid == me).map(|p| p.ppid).unwrap_or(0);
    // Nhóm exe chỉ cần có MỘT pid thuộc họ game là ẩn cả nhóm.
    let family_exes: HashSet<String> = all
        .iter()
        .filter(|p| family.contains(&p.pid))
        .map(|p| p.name.to_lowercase())
        .collect();

    let mut map: HashMap<String, RunningApp> = HashMap::new();
    for p in all.iter() {
        if p.pid == 0 || p.pid == me || p.pid == parent || p.session != my_session {
            continue;
        }
        let exe_lower = p.name.to_lowercase();
        if exe_lower.is_empty() || is_never_kill(&exe_lower, p.exe_path.as_deref()) {
            continue;
        }
        // Game + anti-cheat + họ process game: luôn được giữ, không hiện trong danh sách để chọn.
        if is_ace_or_game(&exe_lower, skip)
            || family.contains(&p.pid)
            || family_exes.contains(&exe_lower)
        {
            continue;
        }
        let entry = map.entry(exe_lower.clone()).or_insert_with(|| RunningApp {
            exe_lower: exe_lower.clone(),
            exe_name: p.name.clone(),
            display: None,
            exe_path: None,
            count: 0,
            total_ws: 0,
        });
        entry.count += 1;
        entry.total_ws += p.ws;
        if entry.display.is_none() {
            entry.display = p.display_name.clone();
        }
        if entry.exe_path.is_none() {
            entry.exe_path = p.exe_path.clone();
        }
    }

    let mut out: Vec<RunningApp> = map.into_values().collect();
    out.sort_by(|a, b| {
        b.total_ws
            .cmp(&a.total_ws)
            .then_with(|| a.exe_lower.cmp(&b.exe_lower))
    });
    out
}

fn restart_as_admin() {
    if let Ok(exe) = std::env::current_exe() {
        let exe_str = exe.to_string_lossy().to_string();
        let _ = std::process::Command::new("powershell")
            .args(["-WindowStyle", "Hidden", "-Command", &format!("Start-Process '{}' -Verb RunAs", exe_str)])
            .spawn();
        std::process::exit(0);
    }
}

#[derive(PartialEq, Clone, Copy)]
enum ConfigTab {
    Apps,
    Services,
    AutoOpt,
    Reports,
}

/// Trạng thái dùng chung giữa UI và watcher. Watcher chạy ở thread riêng nên KHÔNG đọc
/// được field của `App`; mọi lựa chọn nó cần đều nằm trong các `Arc<Mutex<..>>` ở đây.
#[derive(Clone)]
struct Shared {
    log: Arc<Mutex<Vec<(String, Color32)>>>,
    busy: Arc<AtomicBool>,
    /// Session đã lưu (đọc từ session.json) để hiển thị.
    session: Arc<Mutex<Option<SessionData>>>,
    /// App đang chạy thực tế (quét ở thread nền).
    running: Arc<Mutex<Vec<RunningApp>>>,
    /// Lựa chọn "Giữ lại" theo tên exe (viết thường) — lưu vào config.
    keep: Arc<Mutex<HashMap<String, bool>>>,
    svc: Arc<Mutex<Vec<bool>>>,
    /// Danh sách exe game (viết thường).
    games: Arc<Mutex<Vec<String>>>,
    /// Đường dẫn exe đã biết của từng game: exe (viết thường) -> đường dẫn đầy đủ.
    game_paths: Arc<Mutex<HashMap<String, String>>>,
    opts: Arc<Mutex<Opts>>,
    /// App nào của session được tick để Restore mở lại.
    restore_sel: Arc<Mutex<HashMap<String, bool>>>,
    is_admin: bool,
    /// Lần tối ưu gần nhất do watcher khởi động (điều kiện để auto-restore chạy).
    last_auto: Arc<AtomicBool>,
    /// Số lần tối ưu đã chạy: watcher dùng để không tối ưu lặp khi người dùng vừa bấm tay.
    opt_count: Arc<AtomicUsize>,
    /// Báo cáo trận đang ghi (None khi không có game nào chạy).
    live_report: Arc<Mutex<Option<Arc<Mutex<report::Live>>>>>,
    /// Host ping internet cho báo cáo (`report:ping_host`).
    ping_host: Arc<Mutex<String>>,
    /// Commit đã chạm trần (>= 90%): UI hiện banner, icon khay thêm cảnh báo.
    commit_alert: Arc<AtomicBool>,
    /// % commit gần nhất, chỉ để hiển thị trong cảnh báo.
    commit_pct: Arc<Mutex<f32>>,
}

impl Shared {
    fn save_config(&self) -> bool {
        let cfg = Config {
            keep: self.keep.lock().unwrap().clone(),
            svcs: self.svc.lock().unwrap().clone(),
            games: self.games.lock().unwrap().clone(),
            game_paths: self.game_paths.lock().unwrap().clone(),
            ping_host: self.ping_host.lock().unwrap().clone(),
            opts: *self.opts.lock().unwrap(),
        };
        write_config(&cfg)
    }

    fn log(&self, text: String, color: Color32) {
        self.log.lock().unwrap().push((text, color));
    }

    fn opts(&self) -> Opts {
        *self.opts.lock().unwrap()
    }

    /// Thêm một sự kiện vào báo cáo trận đang ghi (bỏ qua nếu đang không ghi trận).
    fn event(&self, kind: &str, text: impl Into<String>) {
        // Lấy Arc ra rồi nhả khoá `live_report` trước khi khoá báo cáo:
        // tránh giữ hai khoá cùng lúc không cần thiết.
        let live = self.live_report.lock().unwrap().clone();
        if let Some(l) = live {
            let text: String = text.into();
            report::add_event(&l, kind, &text);
        }
    }

    /// % commit gần nhất (0.0 nếu chưa đo lần nào).
    fn commit_pct(&self) -> f32 {
        *self.commit_pct.lock().unwrap()
    }
}

/// Bắt đầu tối ưu ở thread nền. Dùng chung cho nút trên UI và watcher.
fn optimize(sh: Shared, from_auto: bool) {
    if sh.busy.swap(true, Ordering::SeqCst) {
        return;
    }
    sh.last_auto.store(from_auto, Ordering::SeqCst);
    sh.opt_count.fetch_add(1, Ordering::SeqCst);
    sh.event(
        "optimize",
        if from_auto {
            "Tối ưu tự động khi game mở"
        } else {
            "Tối ưu bằng tay (nút Game Boost / menu khay)"
        },
    );
    std::thread::spawn(move || {
        do_optimize(&sh);
        sh.busy.store(false, Ordering::SeqCst);
    });
}

/// Bắt đầu Restore ở thread nền (UI và watcher dùng chung).
fn restore(sh: Shared) {
    if sh.busy.swap(true, Ordering::SeqCst) {
        return;
    }
    sh.event("restore", "Khôi phục hệ thống (app, service, Wi-Fi, Game DVR)");
    std::thread::spawn(move || {
        do_restore(&sh);
        sh.busy.store(false, Ordering::SeqCst);
    });
}

/// Thêm một app đang chạy vào danh sách game, lưu đường dẫn exe và (nếu bật) đặt GPU rời.
fn mark_as_game(sh: &Shared, app: &RunningApp) {
    {
        let mut games = sh.games.lock().unwrap();
        if !games.contains(&app.exe_lower) {
            games.push(app.exe_lower.clone());
        }
    }
    let path = app
        .exe_path
        .as_ref()
        .filter(|p| !p.trim().is_empty())
        .cloned();
    if let Some(p) = path.as_ref() {
        sh.game_paths
            .lock()
            .unwrap()
            .insert(app.exe_lower.clone(), p.clone());
    }
    let ok = sh.save_config();
    sh.log(
        format!(
            "  • [GAME] Đã thêm {} vào danh sách game{}.",
            app.exe_lower,
            if path.is_none() { " (chưa rõ đường dẫn exe)" } else { "" }
        ),
        Color32::from_rgb(0, 229, 255),
    );
    if !ok {
        sh.log(
            "  ⚠ [GAME] Không ghi được gamemode.cfg — danh sách game sẽ mất khi thoát.".into(),
            Color32::from_rgb(255, 120, 120),
        );
    }
    if let Some(p) = path {
        if sh.opts().gpu {
            match sys::ensure_gpu_high_perf(&p) {
                Ok(true) => sh.log(
                    format!("  ✔ [GPU] Đã đặt GPU rời cho {} (áp dụng từ lần mở game sau).", p),
                    Color32::from_rgb(0, 255, 163),
                ),
                Ok(false) => sh.log(
                    format!("  • [GPU] {} đã đặt GPU rời từ trước.", p),
                    Color32::from_rgb(130, 150, 170),
                ),
                Err(e) => sh.log(
                    format!("  ⚠ [GPU] Không ghi được GPU rời cho {}: {}", p, e),
                    Color32::from_rgb(255, 120, 120),
                ),
            }
        }
    }
}

/// Thân của tối ưu: tắt app / service / WLAN / Game DVR, xả standby, lưu session.
fn do_optimize(sh: &Shared) {
    let log = sh.log.clone();
    let push = |s: String, c: Color32| log.lock().unwrap().push((s, c));
    log.lock().unwrap().clear();

    let keep = sh.keep.lock().unwrap().clone();
    let svcs = sh.svc.lock().unwrap().clone();
    let games: Vec<String> = sh.games.lock().unwrap().clone();
    let opts = sh.opts();
    let is_admin = sh.is_admin;

    let before_mem = sys::memory();
    let before = before_mem.avail;
    push(
        format!("▶ [DEPLOY] Bắt đầu thanh lọc | RAM khả dụng ban đầu: {:.1} GB", before as f64 / GB),
        Color32::from_rgb(0, 229, 255),
    );

    let me = std::process::id();
    let my_session = sys::current_session();
    // Exe game + anti-cheat: chỉ lấy tên/đường dẫn, không mở quyền đọc bộ nhớ.
    let skip = skip_list(&games);
    // "Họ" process game (launcher / tổ tiên / con cháu) tính TRƯỚC khi quét sâu để con cháu
    // của game cũng chỉ được mở bằng quyền hạn chế, không đọc bộ nhớ.
    let family: HashSet<u32> = game_family_from_nodes(&sys::process_nodes(), &games)
        .into_keys()
        .collect();
    let snap = sys::snapshot_skip(&skip, &family);
    let parent = snap.iter().find(|p| p.pid == me).map(|p| p.ppid).unwrap_or(0);

    // 0. Ghi nhớ đường dẫn exe của game đang chạy (để ép GPU rời cả khi game đã tắt).
    {
        let mut paths = sh.game_paths.lock().unwrap();
        for p in snap.iter() {
            let exe = p.name.to_lowercase();
            if !games.contains(&exe) {
                continue;
            }
            if let Some(path) = p.exe_path.as_ref().filter(|x| !x.trim().is_empty()) {
                if paths.get(&exe).map(|v| v != path).unwrap_or(true) {
                    paths.insert(exe, path.clone());
                }
            }
        }
    }

    // 1. Quét app đang chạy -> nhóm theo tên exe, bỏ nhóm được tick "Giữ lại".
    //    Exe nằm trong danh sách game LUÔN được giữ lại, bất kể config ghi gì.
    let mut groups: HashMap<String, Vec<u32>> = HashMap::new();
    let mut labels: HashMap<String, String> = HashMap::new();
    let mut kept_exes: HashSet<String> = HashSet::new();
    for p in snap.iter() {
        if p.pid == 0 || p.pid == me || p.pid == parent || p.session != my_session {
            continue;
        }
        let exe = p.name.to_lowercase();
        if exe.is_empty() || is_never_kill(&exe, p.exe_path.as_deref()) {
            continue;
        }
        // Game + anti-cheat (ACE) + họ process game: tuyệt đối không tắt, không lưu session,
        // không mở quyền đọc bộ nhớ.
        if keep_decision(&keep, &exe) || is_ace_or_game(&exe, &skip) || family.contains(&p.pid) {
            kept_exes.insert(exe);
            continue;
        }
        groups.entry(exe.clone()).or_default().push(p.pid);
        labels
            .entry(exe)
            .or_insert_with(|| p.display_name.clone().unwrap_or_else(|| p.name.clone()));
    }
    // 1b. Exe được bảo vệ nhờ QUAN HỆ cha/con (tên exe không nằm trong danh sách game/ACE):
    //     báo một dòng để biết vì sao launcher / helper của game không bị tắt.
    {
        let mut names: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for p in snap.iter() {
            if !family.contains(&p.pid) {
                continue;
            }
            let exe = p.name.to_lowercase();
            if is_ace_or_game(&exe, &skip) || !seen.insert(exe) {
                continue;
            }
            names.push(p.name.clone());
        }
        if !names.is_empty() {
            push(
                format!("  • [GAME] Bảo vệ process của game: {}", names.join(", ")),
                Color32::from_rgb(0, 255, 163),
            );
        }
    }
    push(
        format!(
            "  • [SCAN] {} app sẽ tắt / {} app được giữ lại (session user hiện tại).",
            groups.len(),
            kept_exes.len()
        ),
        Color32::from_rgb(50, 215, 255),
    );

    // 2. Gom "process gốc" của từng app để lưu session (TRƯỚC khi tắt).
    //    Cửa sổ phải đọc trước khi gửi WM_CLOSE, sau đó app đã đóng nên không còn gì để đọc.
    let window_pids = sys::pids_with_user_window();
    let mut startup_lower: Option<Vec<String>> = None;
    let mut session_apps: Vec<SessionApp> = Vec::new();
    let mut skipped_children = 0usize;
    for pids in groups.values() {
        // Nhóm exe này có cửa sổ top-level thật không (điều kiện 3).
        let group_has_window = pids.iter().any(|p| window_pids.contains(p));
        for &pid in pids.iter() {
            let Some(info) = snap.iter().find(|p| p.pid == pid) else { continue };

            // (1) Không phải process con của app khác: parent phải là explorer.exe, hoặc
            //     không còn trong snapshot, hoặc là process Windows (svchost do Task
            //     Scheduler mở...). Giữ quy tắc cũ: cùng tên exe với parent -> không phải gốc.
            if let Some(parent) = snap.iter().find(|p| p.pid == info.ppid && p.pid != 0) {
                if parent.name.eq_ignore_ascii_case(&info.name) {
                    continue;
                }
                let windows_parent = parent.name.eq_ignore_ascii_case("explorer.exe")
                    || parent
                        .exe_path
                        .as_deref()
                        .is_some_and(|p| p.to_lowercase().starts_with(r"c:\windows\"));
                if !windows_parent {
                    skipped_children += 1;
                    continue;
                }
            }

            let path = info.exe_path.clone().unwrap_or_default();

            // App đóng gói (WindowsApps) không mở lại được bằng đường dẫn exe.
            if is_packaged_path(&path) {
                continue;
            }

            // (2) Command line không chứa dấu hiệu process phụ.
            let cmdline = info
                .cmdline
                .clone()
                .filter(|c| !c.trim().is_empty())
                .unwrap_or_else(|| format!("\"{}\"", path));
            if is_child_process_cmdline(&cmdline) || is_never_restore_exe(&info.name) {
                skipped_children += 1;
                continue;
            }

            // (3) App người dùng thật: có cửa sổ top-level, hoặc có trong lệnh tự khởi động
            //     (giữ được app chạy ẩn dưới khay như Zalo).
            let is_user_app = group_has_window || {
                let startup = startup_lower.get_or_insert_with(|| {
                    sys::startup_commands()
                        .iter()
                        .map(|s| s.to_lowercase())
                        .collect()
                });
                in_startup_entries(&path, startup)
            };
            if !is_user_app {
                skipped_children += 1;
                continue;
            }

            session_apps.push(SessionApp {
                exe: info.name.clone(),
                display: info.display_name.clone().unwrap_or_else(|| info.name.clone()),
                path,
                cmdline,
                cwd: sys::working_dir_of(pid),
            });
        }
    }
    if skipped_children > 0 {
        push(
            format!(
                "  • [SESSION] Bỏ qua {} process phụ / không phải app người dùng.",
                skipped_children
            ),
            Color32::from_rgb(130, 150, 170),
        );
    }

    // 3. Ép GPU rời cho các exe game đã biết đường dẫn (thiết lập lâu dài, không hoàn tác).
    if opts.gpu {
        let paths = sh.game_paths.lock().unwrap().clone();
        let mut written = 0usize;
        let mut already = 0usize;
        for exe in games.iter() {
            let Some(path) = paths.get(exe) else { continue };
            match sys::ensure_gpu_high_perf(path) {
                Ok(true) => {
                    written += 1;
                    push(
                        format!("  ✔ [GPU] Đặt GPU rời cho {} — áp dụng từ lần mở game sau.", exe),
                        Color32::from_rgb(0, 255, 163),
                    );
                }
                Ok(false) => already += 1,
                Err(e) => push(
                    format!("  ⚠ [GPU] Không đặt được GPU rời cho {}: {}", exe, e),
                    Color32::from_rgb(255, 120, 120),
                ),
            }
        }
        if already > 0 {
            push(
                format!("  • [GPU] {} game đã đặt GPU rời từ trước — áp dụng từ lần mở game sau.", already),
                Color32::from_rgb(130, 150, 170),
            );
        }
        if written == 0 && already == 0 && games.iter().all(|g| !paths.contains_key(g)) {
            push(
                "  • [GPU] Chưa biết đường dẫn exe game — mở game rồi bấm 'Là game' để lưu vị trí.".into(),
                Color32::from_rgb(130, 150, 170),
            );
        }
    }

    // 4. Tạm dừng các Services được chọn (nếu có quyền Admin)
    let mut newly_stopped = Vec::new();
    for ((svc_name, display_name, _, _), on) in SERVICE_TARGETS.iter().zip(&svcs) {
        if !on {
            continue;
        }
        if sys::is_service_running(svc_name) {
            if is_admin {
                match sys::stop_service(svc_name) {
                    Ok(_) => {
                        newly_stopped.push(svc_name.to_string());
                        push(
                            format!("  ✔ [STOP SVC] Tạm dừng service: {:<20} ({})", display_name, svc_name),
                            Color32::from_rgb(0, 255, 163),
                        );
                    }
                    Err(sys::SERVICE_TIMEOUT) => {
                        // Lệnh dừng đã gửi, service đang dừng chậm: vẫn lưu để Restore bật lại.
                        newly_stopped.push(svc_name.to_string());
                        push(
                            format!("  • [STOP SVC] {} phản hồi chậm, đang dừng ngầm ({})", display_name, svc_name),
                            Color32::from_rgb(255, 160, 40),
                        );
                    }
                    Err(e) => {
                        push(
                            format!("  ⚠ [WARN SVC] Lỗi dừng service {}: mã lỗi {}", svc_name, e),
                            Color32::from_rgb(255, 120, 120),
                        );
                    }
                }
            } else {
                push(
                    format!("  ⚠ [SKIP SVC] Bỏ qua {}: Cần quyền Administrator để dừng service.", display_name),
                    Color32::from_rgb(255, 160, 40),
                );
            }
        }
    }

    // 5. Tắt quét nền Wi-Fi (autoconf) của interface đang kết nối, nhớ GUID để Restore bật lại.
    let mut wlan_off: Vec<String> = Vec::new();
    if opts.wlan {
        match sys::wlan_interfaces() {
            Ok(list) => {
                for it in list.iter().filter(|i| i.connected) {
                    match it.autoconf {
                        Some(true) => match sys::set_wlan_autoconf(&it.guid, false) {
                            Ok(_) => {
                                wlan_off.push(it.guid.clone());
                                push(
                                    format!("  ✔ [WLAN] Đã tắt quét Wi-Fi nền: {}", it.description),
                                    Color32::from_rgb(0, 255, 163),
                                );
                            }
                            Err(e) => push(
                                format!("  ⚠ [WLAN] Không tắt được quét nền ({}): {}", it.description, e),
                                Color32::from_rgb(255, 120, 120),
                            ),
                        },
                        Some(false) => push(
                            format!("  • [WLAN] {} đã tắt quét nền từ trước.", it.description),
                            Color32::from_rgb(130, 150, 170),
                        ),
                        None => push(
                            format!("  ⚠ [WLAN] Không đọc được trạng thái quét nền: {}", it.description),
                            Color32::from_rgb(255, 160, 40),
                        ),
                    }
                }
            }
            Err(e) => push(
                format!("  ⚠ [WLAN] Không truy vấn được Wi-Fi: {}", e),
                Color32::from_rgb(255, 120, 120),
            ),
        }
    }

    // 6. Tắt Game DVR / App Capture, lưu giá trị cũ vào session để Restore ghi lại.
    let mut gamedvr_prev: Option<RegPrev> = None;
    let mut appcapture_prev: Option<RegPrev> = None;
    if opts.gamedvr {
        let existing = SessionData::load().unwrap_or_default();
        // Session đã có giá trị "trước đó" thì giữ nguyên: giá trị hiện tại có thể đã bị tool sửa.
        gamedvr_prev = existing
            .gamedvr_prev
            .clone()
            .or_else(|| Some(RegPrev { value: sys::gamedvr_enabled() }));
        appcapture_prev = existing
            .appcapture_prev
            .clone()
            .or_else(|| Some(RegPrev { value: sys::appcapture_enabled() }));
        match sys::set_gamedvr_enabled(Some(0)) {
            Ok(_) => push(
                "  ✔ [DVR] Đã tắt Game DVR (GameDVR_Enabled = 0).".into(),
                Color32::from_rgb(0, 255, 163),
            ),
            Err(e) => push(
                format!("  ⚠ [DVR] Không tắt được Game DVR: {}", e),
                Color32::from_rgb(255, 120, 120),
            ),
        }
        match sys::set_appcapture_enabled(Some(0)) {
            Ok(_) => push(
                "  ✔ [DVR] Đã tắt ghi lại clip nền (AppCaptureEnabled = 0).".into(),
                Color32::from_rgb(0, 255, 163),
            ),
            Err(e) => push(
                format!("  ⚠ [DVR] Không tắt được AppCaptureEnabled: {}", e),
                Color32::from_rgb(255, 120, 120),
            ),
        }
    }

    // 7. Ghi session.json (gộp với session cũ nếu còn) trước khi tắt app.
    let mut data = SessionData::load().unwrap_or_default();
    data.saved_at = sys::local_time_string();
    let apps_before = data.apps.len();
    data.merge(session_apps, newly_stopped);
    data.merge_wlan(&wlan_off);
    if let Some(p) = gamedvr_prev {
        data.gamedvr_prev = Some(p);
    }
    if let Some(p) = appcapture_prev {
        data.appcapture_prev = Some(p);
    }
    if data.save() {
        push(
            format!(
                "  💾 [SESSION] Đã lưu {} app{} + {} service vào session.json ({}).",
                data.apps.len(),
                if data.apps.len() > apps_before && apps_before > 0 {
                    " (gộp với phiên trước)"
                } else {
                    ""
                },
                data.services.len(),
                data.saved_at
            ),
            Color32::from_rgb(0, 229, 255),
        );
    } else {
        push(
            "  ⚠ [SESSION] Không ghi được session.json — Restore sẽ thiếu dữ liệu.".into(),
            Color32::from_rgb(255, 120, 120),
        );
    }
    *sh.session.lock().unwrap() = Some(data);

    // 8. Tắt app: WM_CLOSE trước (chờ tối đa 3s cho app kịp lưu), rồi TerminateProcess.
    let mut killed_any = false;
    let mut groups_sorted: Vec<(&String, &Vec<u32>)> = groups.iter().collect();
    groups_sorted.sort_by(|a, b| b.1.len().cmp(&a.1.len()));
    // Gửi WM_CLOSE cho tất cả app cùng lúc, chờ chung tối đa 3s (không chờ 3s mỗi app).
    let all_pids: Vec<u32> = groups.values().flatten().copied().collect();
    sys::close_gracefully(&all_pids, Duration::from_secs(3));
    for (exe, pids) in groups_sorted {
        let label = labels.get(exe).cloned().unwrap_or_else(|| exe.clone());
        let alive_after = pids.iter().filter(|&&p| sys::is_alive(p)).count();
        let mut freed = 0u64;
        let mut killed = 0usize;
        for &pid in pids.iter() {
            // Process đã tự thoát sau WM_CLOSE thì TerminateProcess trả về lỗi - bỏ qua.
            if let Some(ws) = sys::kill(pid) {
                freed += ws;
                killed += 1;
            }
        }
        killed_any = true;
        push(
            format!(
                "  ✖ [TERMINATE] {:<20} ({} tác vụ, {} còn sống sau WM_CLOSE, {} bị ép tắt) -> Giải phóng {}",
                label,
                pids.len(),
                alive_after,
                killed,
                fmt_ram(freed)
            ),
            Color32::from_rgb(255, 145, 45),
        );
    }
    if !killed_any {
        push(
            "  • [SKIP APPS] Không có app nền nào cần tắt (tất cả đang được giữ lại).".into(),
            Color32::from_rgb(130, 150, 170),
        );
    }

    // 9. Xả standby list một lần trước khi nén RAM (bỏ cache cũ, nhường chỗ cho trang vừa nén).
    if opts.standby {
        match sys::memory_lists() {
            Some(before_lists) => match sys::purge_standby() {
                Ok(_) => {
                    std::thread::sleep(Duration::from_millis(500));
                    let after_lists = sys::memory_lists().unwrap_or(before_lists);
                    push(
                        format!(
                            "  ✔ [STANDBY] Xả standby list: Free {:.2} GB / Standby {:.2} GB -> Free {:.2} GB / Standby {:.2} GB",
                            before_lists.free_bytes as f64 / GB,
                            before_lists.standby_bytes as f64 / GB,
                            after_lists.free_bytes as f64 / GB,
                            after_lists.standby_bytes as f64 / GB
                        ),
                        Color32::from_rgb(50, 215, 255),
                    );
                }
                Err(e) => push(
                    format!("  ⚠ [STANDBY] Không xả được standby list: {}", e),
                    Color32::from_rgb(255, 120, 120),
                ),
            },
            None => push(
                "  ⚠ [STANDBY] Không đọc được danh sách trang bộ nhớ của Windows.".into(),
                Color32::from_rgb(255, 120, 120),
            ),
        }
    }

    // 9b. Nén RAM: đẩy working set của các app ĐƯỢC GIỮ LẠI ra standby list.
    //     Làm sau bước 9 để trang cũ đã bị xả trước, còn trang vừa nén nằm lại trong standby
    //     làm cache (app cần thì lấy lại ngay) -> KHÔNG xả standby lần nữa sau bước này.
    //     Bỏ qua: chính tool, game + anti-cheat + họ process game (không mở handle),
    //     process hệ thống nhạy cảm, pid <= 4.
    if opts.trim {
        let mut targets = 0usize;
        let mut total = 0u64;
        let mut hits: Vec<(u64, String)> = Vec::new();
        for p in sys::list_processes() {
            if p.pid <= 4 || p.pid == me || family.contains(&p.pid) {
                continue;
            }
            let exe = p.name.to_lowercase();
            if exe.is_empty()
                || is_ace_or_game(&exe, &skip)
                || TRIM_SKIP_EXE.contains(&exe.as_str())
            {
                continue;
            }
            targets += 1;
            let freed = sys::trim(p.pid);
            if freed > 0 {
                total += freed;
                hits.push((freed, p.name.clone()));
            }
        }
        hits.sort_by(|a, b| b.0.cmp(&a.0));
        push(
            format!(
                "  ✔ [TRIM] Nén working set {} process -> thu hồi {} ({} process giảm)",
                targets,
                fmt_ram(total),
                hits.len()
            ),
            Color32::from_rgb(50, 215, 255),
        );
        let top: Vec<String> = hits
            .iter()
            .take(5)
            .map(|(bytes, name)| format!("{} {}", name, fmt_ram(*bytes)))
            .collect();
        if !top.is_empty() {
            push(
                format!("     • Giảm nhiều nhất: {}", top.join(", ")),
                Color32::from_rgb(130, 150, 170),
            );
        }

        // 9c. Còn gì đang giữ RAM: gom working set SAU khi nén theo tên exe, bỏ game + anti-cheat
        //     + họ process game.
        let mut left: HashMap<String, u64> = HashMap::new();
        for p in sys::list_processes() {
            if p.pid <= 4 || p.pid == me || family.contains(&p.pid) {
                continue;
            }
            let exe = p.name.to_lowercase();
            if exe.is_empty() || is_ace_or_game(&exe, &skip) {
                continue;
            }
            let ws = sys::working_set_of(p.pid);
            if ws > 0 {
                *left.entry(p.name.clone()).or_insert(0) += ws;
            }
        }
        let mut left_sorted: Vec<(&String, &u64)> = left.iter().collect();
        left_sorted.sort_by(|a, b| b.1.cmp(a.1));
        let remained: Vec<String> = left_sorted
            .iter()
            .take(5)
            .map(|(name, bytes)| format!("{} {}", name, fmt_ram(**bytes)))
            .collect();
        if !remained.is_empty() {
            push(
                format!("  • [CÒN LẠI] {}", remained.join(", ")),
                Color32::from_rgb(255, 200, 90),
            );
        }
    }

    // 10. Lưu lại config (đường dẫn exe game mới biết) rồi cập nhật danh sách app cho UI.
    if !sh.save_config() {
        push(
            "  ⚠ [CONFIG] Không ghi được gamemode.cfg.".into(),
            Color32::from_rgb(255, 120, 120),
        );
    }

    std::thread::sleep(Duration::from_millis(1000));
    let after_mem = sys::memory();
    let after = after_mem.avail;
    let net_freed = (after as f64 - before as f64) / GB;

    push(
        format!(
            "✔ [SUCCESS] HOÀN TẤT! RAM trống hiện tại: {:.1} GB (Tăng +{:.1} GB)",
            after as f64 / GB,
            net_freed.max(0.0)
        ),
        Color32::from_rgb(0, 255, 163),
    );
    push(
        "★ [READY] Toàn bộ tài nguyên CPU & RAM đã tối ưu cho DELTA FORCE!".into(),
        Color32::from_rgb(0, 255, 163),
    );

    *sh.running.lock().unwrap() = scan_running_apps(&skip, &games);
}

/// Thân của Restore: mở lại app, bật lại service, Wi-Fi autoconf, Game DVR.
fn do_restore(sh: &Shared) {
    log_clear(sh);
    let sel = sh.restore_sel.lock().unwrap().clone();
    sh.log(
        "▶ [RESTORE] Đang khôi phục lại các ứng dụng & dịch vụ hệ thống...".into(),
        Color32::from_rgb(0, 229, 255),
    );

    let Some(data) = SessionData::load() else {
        sh.log(
            format!("  ⚠ Không tìm thấy {} — chưa có phiên nào được lưu.", session_path().display()),
            Color32::from_rgb(255, 140, 140),
        );
        return;
    };

    // 1. Mở lại app đã tắt (bỏ qua app đang chạy sẵn), chạy bằng quyền user thường.
    let running: HashSet<String> = sys::list_processes().into_iter().map(|p| p.name.to_lowercase()).collect();
    let mut opened = 0usize;
    let mut failed = 0usize;
    let mut skipped = 0usize;
    for app in data.apps.iter() {
        let key = app_key(app);
        let path = if !app.path.trim().is_empty() {
            app.path.clone()
        } else {
            extract_exe_path(&app.cmdline).unwrap_or_default()
        };
        // Session cũ còn entry rác: process phụ (COM server, helper, renderer...) và app đóng gói.
        if is_child_process_cmdline(&app.cmdline) || is_packaged_path(&path) || is_never_restore_exe(&app.exe) {
            skipped += 1;
            sh.log(
                format!("  • Bỏ qua process phụ: {} ({})", app.display, app.exe),
                Color32::from_rgb(130, 150, 170),
            );
            continue;
        }
        if !sel.get(&key).copied().unwrap_or(true) {
            skipped += 1;
            sh.log(
                format!("  • Bỏ qua (không được tick): {}", app.display),
                Color32::from_rgb(130, 150, 170),
            );
            continue;
        }
        if running.contains(&app.exe.to_lowercase()) {
            sh.log(
                format!("  • {} đã đang chạy sẵn.", app.display),
                Color32::from_rgb(130, 150, 170),
            );
            continue;
        }
        if path.trim().is_empty() {
            failed += 1;
            sh.log(
                format!("  ⚠ {} — không có đường dẫn exe để mở.", app.display),
                Color32::from_rgb(255, 120, 120),
            );
            continue;
        }
        let cmd = if app.cmdline.trim().is_empty() {
            format!("\"{}\"", path)
        } else {
            app.cmdline.clone()
        };
        match sys::launch_as_standard_user(&cmd, &path, app.cwd.as_deref()) {
            Ok(()) => {
                opened += 1;
                sh.log(
                    format!("  ✔ Đã mở lại {} ({})", app.display, app.exe),
                    Color32::from_rgb(0, 255, 163),
                );
            }
            Err(e) => {
                failed += 1;
                sh.log(
                    format!("  ⚠ Không mở được {} ({}): {}", app.display, app.exe, e),
                    Color32::from_rgb(255, 120, 120),
                );
            }
        }
    }
    if skipped > 0 {
        sh.log(
            format!("  • Đã bỏ qua {} entry (process phụ / không được tick).", skipped),
            Color32::from_rgb(130, 150, 170),
        );
    }

    // 2. Bật lại các Services đã dừng trong phiên.
    for svc_name in data.services.iter() {
        match sys::start_service(svc_name) {
            Ok(_) => sh.log(
                format!("  ✔ Khởi động lại service: {}", svc_name),
                Color32::from_rgb(0, 255, 163),
            ),
            // 1056 = ERROR_SERVICE_ALREADY_RUNNING: service đã tự chạy lại, không phải lỗi.
            Err(sys::SERVICE_TIMEOUT) => sh.log(
                format!("  • Service {} khởi động chậm, đang chạy ngầm.", svc_name),
                Color32::from_rgb(255, 160, 40),
            ),
            Err(1056) => sh.log(
                format!("  • Service {} đã đang chạy sẵn.", svc_name),
                Color32::from_rgb(130, 150, 170),
            ),
            Err(e) => sh.log(
                format!("  ⚠ Lỗi khởi động service {}: mã lỗi {}", svc_name, e),
                Color32::from_rgb(255, 120, 120),
            ),
        }
    }

    // 3. Bật lại quét nền Wi-Fi cho các interface đã tắt (áp dụng cả khi không có app nào cần mở).
    for guid in data.wlan_autoconf_off.iter() {
        match sys::set_wlan_autoconf(guid, true) {
            Ok(_) => sh.log(
                format!("  ✔ Đã bật lại quét Wi-Fi nền cho interface {}.", guid),
                Color32::from_rgb(0, 255, 163),
            ),
            Err(e) => sh.log(
                format!("  ⚠ Không bật lại được quét Wi-Fi nền ({}): {}", guid, e),
                Color32::from_rgb(255, 120, 120),
            ),
        }
    }

    // 4. Ghi lại giá trị Game DVR / App Capture trước khi tool sửa (None = xoá value).
    if let Some(prev) = data.gamedvr_prev.as_ref() {
        match sys::set_gamedvr_enabled(prev.value) {
            Ok(_) => sh.log(
                format!("  ✔ Đã trả GameDVR_Enabled về {:?}.", prev.value),
                Color32::from_rgb(0, 255, 163),
            ),
            Err(e) => sh.log(
                format!("  ⚠ Không trả lại được GameDVR_Enabled: {}", e),
                Color32::from_rgb(255, 120, 120),
            ),
        }
    }
    if let Some(prev) = data.appcapture_prev.as_ref() {
        match sys::set_appcapture_enabled(prev.value) {
            Ok(_) => sh.log(
                format!("  ✔ Đã trả AppCaptureEnabled về {:?}.", prev.value),
                Color32::from_rgb(0, 255, 163),
            ),
            Err(e) => sh.log(
                format!("  ⚠ Không trả lại được AppCaptureEnabled: {}", e),
                Color32::from_rgb(255, 120, 120),
            ),
        }
    }

    // 5. Đã mở xong -> session.json chuyển thành session.last.json.
    let sp = session_path();
    let archived = sp.with_file_name("session.last.json");
    if std::fs::rename(&sp, &archived).is_err() {
        let _ = std::fs::remove_file(&sp);
    }
    *sh.session.lock().unwrap() = None;

    sh.log(
        format!(
            "✔ Đã gửi lệnh khôi phục xong: {} app mở được, {} lỗi, {} bỏ qua, {} service.",
            opened,
            failed,
            skipped,
            data.services.len()
        ),
        Color32::from_rgb(140, 165, 190),
    );
}

fn log_clear(sh: &Shared) {
    sh.log.lock().unwrap().clear();
}

/// % commit hiện tại (0.0 nếu không đọc được).
fn commit_pct() -> f32 {
    let (used, limit) = sys::commit();
    if limit == 0 {
        0.0
    } else {
        used as f32 * 100.0 / limit as f32
    }
}

/// Ping `times` lần tới một địa chỉ IPv4, trả về các dòng kết quả cho `--pingtest`.
fn ping_lines(label: &str, addr: u32, times: u32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut got = 0u32;
    let mut sum = 0.0f32;
    let mut max = 0.0f32;
    for i in 1..=times {
        match sys::ping(addr, 1000) {
            Some(rtt) => {
                got += 1;
                sum += rtt;
                max = max.max(rtt);
                lines.push(format!("  {} #{}: {:.0} ms", label, i, rtt));
            }
            None => lines.push(format!("  {} #{}: mất gói (chờ 1000 ms)", label, i)),
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    lines.push(format!(
        "  {}: {}/{} hồi âm, TB {:.0} ms, max {:.0} ms, mất {:.0}%",
        label,
        got,
        times,
        if got > 0 { sum / got as f32 } else { 0.0 },
        max,
        (times - got) as f32 * 100.0 / times as f32
    ));
    lines
}

/// Watcher chạy suốt đời app ở thread riêng: tự tối ưu khi game mở, tự Restore khi game tắt,
/// xả standby list khi RAM trống xuống thấp trong lúc chơi, cảnh báo commit gần trần,
/// và ghi báo cáo số liệu cho mỗi trận (RAM / commit / CPU / ping).
/// Không phụ thuộc `update()` nên vẫn hoạt động khi cửa sổ bị thu nhỏ.
fn start_watcher(sh: Shared) {
    std::thread::spawn(move || {
        const SCAN: Duration = Duration::from_secs(3);
        const GAME_STABLE: Duration = Duration::from_secs(5);
        const GAME_GONE: Duration = Duration::from_secs(15);
        const MEM_CHECK: Duration = Duration::from_secs(5);
        const PURGE_GAP: Duration = Duration::from_secs(60);
        const LOW_FREE: u64 = 1024 * 1024 * 1024;
        const HIGH_STANDBY: u64 = 1024 * 1024 * 1024;
        // Nhịp kiểm tra commit (chạy cả khi không có game).
        const COMMIT_CHECK: Duration = Duration::from_secs(5);
        // Dưới mức này thì tắt cờ cảnh báo commit.
        const COMMIT_CLEAR_PCT: f32 = 85.0;
        // Hai dòng cảnh báo commit cách nhau tối thiểu.
        const COMMIT_WARN_GAP: Duration = Duration::from_secs(120);

        let mut game_seen: Option<(Instant, usize)> = None;
        let mut game_gone: Option<Instant> = None;
        let mut optimized = false;
        let mut last_mem_check = Instant::now();
        let mut last_purge = Instant::now() - PURGE_GAP;
        // Báo cáo trận đang ghi (None khi không có game nào chạy).
        let mut live: Option<Arc<Mutex<report::Live>>> = None;
        let mut last_commit_check = Instant::now() - COMMIT_CHECK;
        let mut last_commit_warn = Instant::now() - COMMIT_WARN_GAP;
        // Đợt cảnh báo commit này đã xả standby một lần chưa.
        let mut commit_purged = false;

        loop {
            std::thread::sleep(SCAN);
            let opts = sh.opts();
            let games: Vec<String> = sh.games.lock().unwrap().clone();
            let (up, any) = if games.is_empty() {
                (Vec::new(), false)
            } else {
                let names: Vec<String> = sys::list_processes()
                    .into_iter()
                    .map(|p| p.name.to_lowercase())
                    .collect();
                let up: Vec<String> = games
                    .iter()
                    .filter(|g| names.iter().any(|n| n == *g))
                    .cloned()
                    .collect();
                let any = !up.is_empty();
                (up, any)
            };

            // Cảnh báo commit gần trần — chạy cả khi không có game nào.
            if last_commit_check.elapsed() >= COMMIT_CHECK {
                last_commit_check = Instant::now();
                let pct = commit_pct();
                *sh.commit_pct.lock().unwrap() = pct;
                if pct >= report::COMMIT_WARN_PCT {
                    let already = sh.commit_alert.swap(true, Ordering::SeqCst);
                    let mut announced = false;
                    if !already || last_commit_warn.elapsed() >= COMMIT_WARN_GAP {
                        last_commit_warn = Instant::now();
                        announced = true;
                        sh.log(
                            format!(
                                "  ⚠ [COMMIT] Commit {:.0}% — sắp chạm trần (RAM + pagefile). Game có thể văng: tăng pagefile (16–24 GB) hoặc thêm RAM.",
                                pct
                            ),
                            Color32::from_rgb(255, 59, 59),
                        );
                    }
                    if announced {
                        sh.event("commit_warn", format!("Commit {:.0}% — gần chạm trần", pct));
                    }
                    // Đang chơi mà commit chạm trần: xả standby ngay, bỏ qua khoảng cách 60 giây,
                    // nhưng chỉ MỘT lần cho mỗi đợt. Xả standby KHÔNG làm giảm commit nên vẫn giữ cảnh báo.
                    if any && opts.standby && !commit_purged && !sh.busy.load(Ordering::SeqCst) {
                        commit_purged = true;
                        last_purge = Instant::now();
                        match sys::purge_standby() {
                            Ok(_) => {
                                sh.log(
                                    format!(
                                        "  ✔ [COMMIT] Xả standby vì commit {:.0}% (xả standby không giảm commit — vẫn giữ cảnh báo).",
                                        pct
                                    ),
                                    Color32::from_rgb(50, 215, 255),
                                );
                                sh.event("purge", format!("Xả standby do commit {:.0}%", pct));
                            }
                            Err(e) => sh.log(
                                format!("  ⚠ [COMMIT] Không xả được standby: {}", e),
                                Color32::from_rgb(255, 120, 120),
                            ),
                        }
                    }
                } else if pct < COMMIT_CLEAR_PCT {
                    sh.commit_alert.store(false, Ordering::SeqCst);
                    commit_purged = false;
                }
            }

            // Không có game trong danh sách và cũng không ghi trận nào: hết việc.
            if games.is_empty() && live.is_none() {
                continue;
            }

            if any {
                game_gone = None;
                // Bắt đầu ghi báo cáo trận — ghi cả khi auto mode tắt.
                if live.is_none() && opts.report_enabled {
                    let host = sh.ping_host.lock().unwrap().clone();
                    let started = report::start(
                        &up.join(", "),
                        &host,
                        report::SAMPLE_SEC,
                        report::MIN_DURATION_SEC,
                    );
                    *sh.live_report.lock().unwrap() = Some(started.clone());
                    sh.log(
                        format!(
                            "  • [BÁO CÁO] Bắt đầu ghi trận {} (mẫu mỗi {} giây).",
                            up.join(", "),
                            report::SAMPLE_SEC
                        ),
                        Color32::from_rgb(0, 229, 255),
                    );
                    live = Some(started);
                }
                if opts.auto_enabled && !optimized {
                    let (since, count_at_seen) = *game_seen
                        .get_or_insert_with(|| (Instant::now(), sh.opt_count.load(Ordering::SeqCst)));
                    if since.elapsed() >= GAME_STABLE && !sh.busy.load(Ordering::SeqCst) {
                        // Người dùng vừa bấm tối ưu bằng tay trong lúc chờ -> không chạy lặp.
                        if sh.opt_count.load(Ordering::SeqCst) != count_at_seen {
                            optimized = true;
                            game_seen = None;
                            continue;
                        }
                        sh.log(
                            format!("[AUTO] Phát hiện {}", up.join(", ")),
                            Color32::from_rgb(0, 229, 255),
                        );
                        optimized = true;
                        game_seen = None;
                        optimize(sh.clone(), true);
                    }
                }
            } else {
                game_seen = None;
                let just_gone = game_gone.is_none();
                let since = *game_gone.get_or_insert_with(Instant::now);
                // Chốt mốc game tắt ngay lần đầu thấy vắng (không đợi hết 15 giây xác nhận).
                if just_gone {
                    if let Some(l) = live.as_ref() {
                        report::mark_game_end(l);
                    }
                }
                if since.elapsed() >= GAME_GONE {
                    // Cùng mốc 15 giây với auto-restore: đóng báo cáo trận rồi mới khôi phục.
                    let will_restore = opts.auto_restore
                        && optimized
                        && sh.last_auto.load(Ordering::SeqCst)
                        && !sh.busy.load(Ordering::SeqCst);
                    if let Some(l) = live.take() {
                        *sh.live_report.lock().unwrap() = None;
                        if will_restore {
                            report::add_event(&l, "restore", "Tự khôi phục sau khi game tắt 15 giây");
                        }
                        let out = report::finish(l);
                        sh.log(
                            out.line.clone(),
                            if out.saved {
                                Color32::from_rgb(0, 255, 163)
                            } else {
                                Color32::from_rgb(130, 150, 170)
                            },
                        );
                    }
                    // Chỉ tự Restore khi phiên game này do auto khởi động, và không chạy lặp.
                    if will_restore {
                        optimized = false;
                        sh.log(
                            "[AUTO] Game đã tắt 15 giây — tự khôi phục lại hệ thống.".into(),
                            Color32::from_rgb(0, 229, 255),
                        );
                        restore(sh.clone());
                    } else if !sh.busy.load(Ordering::SeqCst) {
                        optimized = false;
                    }
                }
            }

            // Trong phiên game: RAM trống thấp mà standby nhiều -> xả, tối thiểu 60 giây một lần.
            if any
                && opts.standby
                && !sh.busy.load(Ordering::SeqCst)
                && last_mem_check.elapsed() >= MEM_CHECK
                && last_purge.elapsed() >= PURGE_GAP
            {
                last_mem_check = Instant::now();
                if let Some(lists) = sys::memory_lists() {
                    if lists.free_bytes < LOW_FREE && lists.standby_bytes > HIGH_STANDBY {
                        match sys::purge_standby() {
                            Ok(_) => {
                                last_purge = Instant::now();
                                let after = sys::memory_lists().unwrap_or(lists);
                                sh.log(
                                    format!(
                                        "  ✔ [AUTO] Xả standby khi đang chơi: Free {:.2} GB / Standby {:.2} GB -> Free {:.2} GB / Standby {:.2} GB",
                                        lists.free_bytes as f64 / GB,
                                        lists.standby_bytes as f64 / GB,
                                        after.free_bytes as f64 / GB,
                                        after.standby_bytes as f64 / GB
                                    ),
                                    Color32::from_rgb(50, 215, 255),
                                );
                                sh.event(
                                    "purge",
                                    format!(
                                        "Xả standby khi RAM trống {:.1} GB / standby {:.1} GB",
                                        lists.free_bytes as f64 / GB,
                                        lists.standby_bytes as f64 / GB
                                    ),
                                );
                            }
                            Err(e) => {
                                last_purge = Instant::now();
                                sh.log(
                                    format!("  ⚠ [AUTO] Không xả được standby khi đang chơi: {}", e),
                                    Color32::from_rgb(255, 120, 120),
                                );
                            }
                        }
                    }
                }
            }
        }
    });
}

/// Cờ `--minimized`: vào thẳng khay hệ thống (cửa sổ ẩn) thay vì thu xuống taskbar.
static START_HIDDEN: AtomicBool = AtomicBool::new(false);

struct App {
    sh: Shared,
    tab: ConfigTab,
    mem: sys::Mem,
    /// Free / Standby đọc từ danh sách trang bộ nhớ; None nếu không đọc được.
    mem_lists: Option<sys::MemLists>,
    last_mem: Instant,
    last_ml: Instant,
    saved_at: Option<(Instant, bool)>,
    is_admin: bool,
    scanning: Arc<AtomicBool>,
    /// Ô nhập exe game mới.
    game_input: String,
    /// Task "GameMode" trong Task Scheduler (cache, hỏi lại định kỳ).
    task_on: bool,
    last_task_check: Instant,
    /// Còn phải ẩn cửa sổ ở frame đầu (cờ --minimized).
    start_hidden: bool,
    /// Icon khay hệ thống + menu; `None` nếu tạo lỗi (app vẫn dùng được như cũ).
    tray: Option<tray::Tray>,
    /// HWND của cửa sổ chính, lấy từ `CreationContext`; 0 nếu không lấy được.
    hwnd: isize,
    /// Cache danh sách báo cáo trên đĩa, không đọc đĩa mỗi frame.
    reports_cache: Option<Vec<report::ReportMeta>>,
    /// ID báo cáo đang chọn: Some("live") hoặc Some("YYYYMMDD-HHMMSS").
    selected_report_id: Option<String>,
    /// Chi tiết báo cáo đã nạp (cache tránh load_report mỗi frame).
    cached_report: Option<report::GameReport>,
    /// Xác nhận 2 bước khi xoá: (id_báo_cáo, thời_điểm_bấm_lần_1).
    delete_confirm: Option<(String, Instant)>,
    /// Chu kỳ đọc sys::commit() cho thẻ Pagefile (tối đa mỗi 2 giây).
    last_commit_sys: Instant,
    commit_sys: (u64, u64),
}

impl App {
    fn save(&mut self) {
        let ok = self.sh.save_config();
        self.saved_at = Some((Instant::now(), ok));
    }

    fn new(cc: &eframe::CreationContext) -> Self {
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "chakra_regular".into(),
            egui::FontData::from_static(include_bytes!("../assets/fonts/ChakraPetch-Regular.ttf")),
        );
        fonts.font_data.insert(
            "chakra_semibold".into(),
            egui::FontData::from_static(include_bytes!("../assets/fonts/ChakraPetch-SemiBold.ttf")),
        );
        fonts.font_data.insert(
            "chakra_bold".into(),
            egui::FontData::from_static(include_bytes!("../assets/fonts/ChakraPetch-Bold.ttf")),
        );
        fonts.font_data.insert(
            "jetbrains_mono".into(),
            egui::FontData::from_static(include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf")),
        );

        // Chakra Petch làm font chính tỉ lệ (giữ font mặc định của egui làm fallback)
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, "chakra_regular".into());

        // JetBrains Mono cho log và tên exe / monospace (giữ fallback)
        fonts
            .families
            .entry(egui::FontFamily::Monospace)
            .or_default()
            .insert(0, "jetbrains_mono".into());

        cc.egui_ctx.set_fonts(fonts);
        ui::setup_visuals(&cc.egui_ctx);

        let cfg = load_config();
        let is_admin = sys::is_admin();
        let session = SessionData::load();
        // Mặc định tick hết: người dùng bỏ tick app nào thì Restore không mở app đó.
        let restore_sel: HashMap<String, bool> = session
            .as_ref()
            .map(|s| s.apps.iter().map(|a| (app_key(a), true)).collect())
            .unwrap_or_default();

        let sh = Shared {
            log: Arc::new(Mutex::new(vec![
                ("[SYSTEM] DELTA FORCE TACTICAL RAM SUITE READY".into(), Color32::from_rgb(0, 229, 255)),
                (
                    if is_admin {
                        "[PERM] Quyền Administrator: ĐÃ KÍCH HOẠT (Đủ quyền quản lý Services)".into()
                    } else {
                        "[PERM] Chạy người dùng thường. Nhấp 'Cấp Quyền Admin' để tắt Services.".into()
                    },
                    if is_admin { Color32::from_rgb(0, 255, 163) } else { Color32::from_rgb(255, 170, 40) },
                ),
                (
                    "[SCAN] Đang quét các ứng dụng đang chạy trong session hiện tại...".into(),
                    Color32::from_rgb(50, 215, 255),
                ),
            ])),
            busy: Arc::new(AtomicBool::new(false)),
            running: Arc::new(Mutex::new(Vec::new())),
            session: Arc::new(Mutex::new(session)),
            keep: Arc::new(Mutex::new(cfg.keep)),
            svc: Arc::new(Mutex::new(cfg.svcs)),
            games: Arc::new(Mutex::new(cfg.games)),
            game_paths: Arc::new(Mutex::new(cfg.game_paths)),
            opts: Arc::new(Mutex::new(cfg.opts)),
            restore_sel: Arc::new(Mutex::new(restore_sel)),
            is_admin,
            last_auto: Arc::new(AtomicBool::new(false)),
            opt_count: Arc::new(AtomicUsize::new(0)),
            live_report: Arc::new(Mutex::new(None)),
            ping_host: Arc::new(Mutex::new(cfg.ping_host.clone())),
            commit_alert: Arc::new(AtomicBool::new(false)),
            commit_pct: Arc::new(Mutex::new(0.0)),
        };

        let mut app = Self {
            sh: sh.clone(),
            tab: ConfigTab::Apps,
            mem: sys::memory(),
            mem_lists: sys::memory_lists(),
            last_mem: Instant::now(),
            last_ml: Instant::now(),
            saved_at: None,
            is_admin,
            scanning: Arc::new(AtomicBool::new(false)),
            game_input: String::new(),
            task_on: sys::task_exists(),
            last_task_check: Instant::now(),
            start_hidden: START_HIDDEN.load(Ordering::SeqCst),
            tray: None,
            // HWND lấy ngay từ CreationContext: thread khay cần nó để ẩn/hiện cửa sổ
            // kể cả khi eframe không gọi update() nữa.
            hwnd: window_hwnd(cc),
            reports_cache: None,
            selected_report_id: None,
            cached_report: None,
            delete_confirm: None,
            last_commit_sys: Instant::now(),
            commit_sys: sys::commit(),
        };
        app.save();
        app.start_scan();
        // Watcher sống độc lập với UI: vẫn tự tối ưu khi cửa sổ bị thu nhỏ hoặc ẩn xuống khay.
        start_watcher(sh.clone());
        // Icon khay phải tạo trên main thread (Windows cần message loop của thread này);
        // `Tray` được giữ trong `App` để icon không bị bỏ khi hàm này kết thúc.
        match tray::Tray::new(&sh) {
            Ok(t) => {
                app.tray = Some(t);
                tray::start_event_thread(sh, cc.egui_ctx.clone(), app.hwnd);
            }
            Err(e) => sh.log(
                format!("  ⚠ [TRAY] Không tạo được icon khay: {}", e),
                ui::DANGER,
            ),
        }
        app
    }

    /// Quét lại danh sách app đang chạy ở thread nền.
    fn start_scan(&mut self) {
        if self.scanning.swap(true, Ordering::SeqCst) {
            return;
        }
        let running = self.sh.running.clone();
        let scanning = self.scanning.clone();
        let games = self.sh.games.lock().unwrap().clone();
        let skip = skip_list(&games);
        std::thread::spawn(move || {
            let list = scan_running_apps(&skip, &games);
            *running.lock().unwrap() = list;
            scanning.store(false, Ordering::SeqCst);
        });
    }

    /// Nạp lại danh sách báo cáo từ đĩa và cập nhật lựa chọn mặc định.
    fn refresh_reports(&mut self) {
        let list = report::list_reports();
        let live_exists = self.sh.live_report.lock().unwrap().is_some();
        if live_exists {
            if self.selected_report_id.is_none() || self.selected_report_id.as_deref() == Some("live") {
                self.selected_report_id = Some("live".into());
                self.cached_report = None;
            }
        } else if self.selected_report_id.as_deref() == Some("live") || self.selected_report_id.is_none() {
            self.selected_report_id = list.first().map(|m| m.id.clone());
            self.cached_report = self.selected_report_id.as_deref().and_then(report::load_report);
        } else if let Some(ref id) = self.selected_report_id {
            if !list.iter().any(|m| &m.id == id) {
                self.selected_report_id = list.first().map(|m| m.id.clone());
                self.cached_report = self.selected_report_id.as_deref().and_then(report::load_report);
            }
        }
        self.reports_cache = Some(list);
    }

    /// Chọn một báo cáo để xem chi tiết: "live" hoặc ID của file.
    fn select_report(&mut self, id: &str) {
        if id == "live" {
            self.selected_report_id = Some("live".into());
            self.cached_report = None;
        } else {
            if self.selected_report_id.as_deref() != Some(id) {
                self.cached_report = report::load_report(id);
                self.selected_report_id = Some(id.to_string());
            }
        }
        self.delete_confirm = None;
    }
}

fn render_stat_card(
    ui: &mut egui::Ui,
    title: &str,
    sub_text: &str,
    used_gb: f64,
    total_gb: f64,
    ratio: f32,
    _color: Color32,
    extra: Option<&str>,
) {
    ui::card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(title)
                    .size(11.0)
                    .strong()
                    .extra_letter_spacing(1.0)
                    .color(ui::ACCENT),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(sub_text)
                        .size(10.0)
                        .color(if ratio > 0.80 { ui::DANGER } else { ui::TEXT_MUTED }),
                );
            });
        });
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            // Số liệu lớn dùng font đậm
            ui.label(
                RichText::new(format!("{:.1} GB", used_gb))
                    .size(18.0)
                    .strong()
                    .color(ui::DATA_CYAN),
            );
            ui.label(
                RichText::new(format!("/ {:.1} GB", total_gb))
                    .size(11.0)
                    .color(ui::TEXT_MUTED),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(format!("{:.0}%", ratio * 100.0))
                        .size(11.5)
                        .strong()
                        .monospace()
                        .color(if ratio > 0.80 {
                            ui::DANGER
                        } else if ratio >= 0.60 {
                            ui::ACCENT
                        } else {
                            ui::DATA_CYAN
                        }),
                );
            });
        });
        ui.add_space(6.0);

        // Thanh RAM và Pagefile chia đoạn (~20 ô nhỏ, cách nhau 2px)
        ui::segmented_bar(ui, ratio);

        if let Some(text) = extra {
            ui.add_space(5.0);
            ui.label(
                RichText::new(text)
                    .monospace()
                    .size(9.5)
                    .color(ui::TEXT_MUTED),
            );
        } else {
            ui.add_space(18.0);
        }
    });
}

fn render_auto_card(
    ui: &mut egui::Ui,
    sh: &Shared,
    opts_view: &mut Opts,
    task_on: &mut bool,
    last_task_check: &mut Instant,
    games_view: &[String],
    game_input: &mut String,
    save_needed: &mut bool,
) {
    ui::card(ui, |ui| {
        ui::section_header(
            ui,
            "⚙",
            "Tự động khi chơi game",
            Some("Watcher quét tên process mỗi 3 giây (không mở handle vào game / ACE)"),
        );
        ui.add_space(ui::SPACE_HEADER_GAP);

        if ui::toggle_row(
            ui,
            &mut opts_view.auto_enabled,
            "Tự tối ưu khi game mở",
            "Chờ 5 giây cho game ổn định rồi dọn RAM & service",
        ) {
            *save_needed = true;
        }
        ui.add_space(2.0);
        if ui::toggle_row(
            ui,
            &mut opts_view.auto_restore,
            "Tự Restore khi game tắt",
            "Game tắt liên tục 15 giây thì khôi phục (phiên do auto tối ưu)",
        ) {
            *save_needed = true;
        }
        ui.add_space(2.0);
        if ui::toggle_row(
            ui,
            &mut opts_view.close_to_tray,
            "Đóng cửa sổ = thu xuống khay",
            "Bấm X sẽ ẩn app xuống khay hệ thống; thoát hẳn bằng menu khay",
        ) {
            *save_needed = true;
        }
        ui.add_space(2.0);

        // Chạy cùng Windows bằng Task Scheduler
        let mut cur_task = *task_on;
        if ui::toggle_row(
            ui,
            &mut cur_task,
            "Chạy cùng Windows (Task Scheduler, quyền cao nhất)",
            "Tạo task 'GameMode' chạy lúc đăng nhập với /RL HIGHEST — không hiện UAC",
        ) {
            let r = if cur_task { sys::task_create() } else { sys::task_delete() };
            match r {
                Ok(_) => {
                    *task_on = cur_task;
                    *last_task_check = Instant::now();
                    sh.log(
                        if cur_task {
                            "  ✔ [TASK] Đã tạo task GameMode: tự chạy cùng Windows với quyền cao nhất.".into()
                        } else {
                            "  • [TASK] Đã xoá task GameMode.".into()
                        },
                        ui::SUCCESS,
                    );
                }
                Err(e) => {
                    *last_task_check = Instant::now() - Duration::from_secs(20);
                    sh.log(
                        format!("  ⚠ [TASK] Không đổi được task GameMode: {}", e),
                        ui::DANGER,
                    );
                }
            }
        }

        ui.add_space(10.0);
        ui.label(
            RichText::new("DANH SÁCH GAME ĐƯỢC GIỮ LẠI:")
                .size(11.0)
                .strong()
                .color(ui::TEXT_MUTED),
        );
        ui.add_space(4.0);

        let mut remove: Option<String> = None;
        if games_view.is_empty() {
            ui.label(RichText::new("Chưa có game nào").size(11.0).color(ui::TEXT_DIM));
        } else {
            egui::ScrollArea::vertical()
                .id_salt("game_list_scroll")
                .max_height(100.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for exe in games_view.iter() {
                        let path = sh.game_paths.lock().unwrap().get(exe).cloned();
                        let row_h = 24.0_f32;
                        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), egui::Sense::hover());
                        let painter = ui.painter_at(rect);

                        // Nút X xoá game: 18x18, vẽ 2 đường chéo bằng painter (tránh ô vuông glyph)
                        let del_rect = Rect::from_center_size(
                            pos2(rect.right() - 10.0, rect.center().y),
                            vec2(18.0, 18.0),
                        );
                        let del_resp = ui.interact(del_rect, ui.id().with(exe), egui::Sense::click())
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text("Xoá khỏi danh sách game");
                        let del_color = if del_resp.hovered() { ui::DANGER } else { ui::TEXT_DIM };
                        let pad = 5.0_f32;
                        let stroke = Stroke::new(1.5_f32, del_color);
                        painter.line_segment(
                            [
                                pos2(del_rect.left() + pad, del_rect.top() + pad),
                                pos2(del_rect.right() - pad, del_rect.bottom() - pad),
                            ],
                            stroke,
                        );
                        painter.line_segment(
                            [
                                pos2(del_rect.right() - pad, del_rect.top() + pad),
                                pos2(del_rect.left() + pad, del_rect.bottom() - pad),
                            ],
                            stroke,
                        );
                        if del_resp.clicked() {
                            remove = Some(exe.clone());
                        }

                        let text_max_w = (rect.width() - 28.0).max(40.0);
                        let exe_galley = painter.layout_no_wrap(
                            exe.clone(),
                            FontId::monospace(11.0),
                            ui::TEXT_MAIN,
                        );
                        let exe_w = exe_galley.size().x;
                        painter.galley(
                            pos2(rect.left() + 2.0, rect.center().y - exe_galley.size().y / 2.0),
                            exe_galley,
                            ui::TEXT_MAIN,
                        );

                        let path_desc = match path {
                            Some(ref p) => format!(" — {}", p),
                            None => " — (chưa biết đường dẫn)".to_string(),
                        };
                        let path_max_w = (text_max_w - exe_w - 8.0).max(0.0);
                        if path_max_w > 20.0 {
                            let path_galley = painter.layout(
                                path_desc,
                                FontId::proportional(10.5),
                                ui::TEXT_DIM,
                                path_max_w,
                            );
                            painter.galley(
                                pos2(rect.left() + 2.0 + exe_w + 6.0, rect.center().y - path_galley.size().y / 2.0),
                                path_galley,
                                ui::TEXT_DIM,
                            );
                        }
                        ui.add_space(2.0);
                    }
                });
        }
        if let Some(exe) = remove {
            sh.games.lock().unwrap().retain(|e| e != &exe);
            *save_needed = true;
        }

        ui.add_space(8.0);

        // Ô nhập exe game (nền BG, viền BORDER, chữ TEXT_MAIN) + nút Thêm
        ui.horizontal(|ui| {
            let btn_w = 64.0_f32;
            let spacing = 6.0_f32;
            let edit_w = (ui.available_width() - btn_w - spacing).max(100.0_f32);

            egui::Frame::none()
                .fill(ui::BG)
                .stroke(Stroke::new(1.0_f32, ui::BORDER))
                .rounding(Rounding::same(6.0))
                .inner_margin(Margin::symmetric(8.0, 5.0))
                .show(ui, |ui| {
                    let edit = egui::TextEdit::singleline(game_input)
                        .hint_text("vd: game-client-win64-shipping.exe")
                        .text_color(ui::TEXT_MAIN)
                        .frame(false)
                        .desired_width(edit_w - 16.0);
                    ui.add(edit);
                });

            let add_btn = egui::Button::new(
                RichText::new("Thêm")
                    .size(12.0)
                    .strong()
                    .color(ui::ACCENT),
            )
            .fill(ui::CARD_BG)
            .stroke(Stroke::new(1.0_f32, ui::ACCENT))
            .min_size(vec2(btn_w, 30.0))
            .rounding(Rounding::same(6.0));

            if ui.add(add_btn).clicked() {
                let exe = game_input.trim().to_lowercase();
                if !exe.is_empty() {
                    let mut games = sh.games.lock().unwrap();
                    if !games.contains(&exe) {
                        games.push(exe.clone());
                    }
                    drop(games);
                    game_input.clear();
                    *save_needed = true;
                    sh.log(
                        format!("  • [GAME] Đã thêm {} vào danh sách game.", exe),
                        ui::ACCENT,
                    );
                }
            }
        });

        ui.add_space(4.0);
        ui.label(
            RichText::new("(hoặc bấm 'Là game' ở tab Ứng dụng đang chạy)")
                .size(10.5)
                .color(ui::TEXT_MUTED),
        );
    });
}

fn render_optimize_card(
    ui: &mut egui::Ui,
    opts_view: &mut Opts,
    save_needed: &mut bool,
) {
    ui::card(ui, |ui| {
        ui::section_header(
            ui,
            "⚙",
            "Tối ưu khi chơi",
            None,
        );
        ui.add_space(ui::SPACE_HEADER_GAP);

        if ui::toggle_row(
            ui,
            &mut opts_view.standby,
            "Xả standby list (RAM chờ) khi tối ưu",
            "Không đẩy app ra pagefile; lúc chơi tự xả nếu Free < 1 GB & Standby > 1 GB",
        ) {
            *save_needed = true;
        }
        ui.add_space(2.0);

        if ui::toggle_row(
            ui,
            &mut opts_view.trim,
            "Nén RAM các app được giữ lại",
            "Đẩy RAM của app nền (Discord, Steam, Claude...) ra cache; không đụng game và anti-cheat",
        ) {
            *save_needed = true;
        }
        ui.add_space(2.0);

        if ui::toggle_row(
            ui,
            &mut opts_view.gpu,
            "Ép game dùng GPU rời (NVIDIA)",
            "Ghi UserGpuPreferences = 2 trong DirectX registry cho game",
        ) {
            *save_needed = true;
        }
        ui.add_space(2.0);

        if ui::toggle_row(
            ui,
            &mut opts_view.wlan,
            "Tắt quét Wi-Fi nền khi chơi",
            "Tắt autoconf của card Wi-Fi đang kết nối để giảm giật lag đột ngột",
        ) {
            *save_needed = true;
        }

        // Callout ghi chú Wi-Fi ngay dưới toggle Wi-Fi
        ui.add_space(2.0);
        ui::callout_warning(
            ui,
            "Mạng vẫn giữ kết nối hiện tại nhưng KHÔNG tự kết nối lại nếu rớt; Restore sẽ bật lại.",
        );
        ui.add_space(4.0);

        if ui::toggle_row(
            ui,
            &mut opts_view.gamedvr,
            "Tắt Game DVR / quay màn hình nền của Windows",
            "Đặt GameDVR_Enabled và AppCaptureEnabled về 0; Restore khôi phục lại",
        ) {
            *save_needed = true;
        }
        ui.add_space(2.0);

        if ui::toggle_row(
            ui,
            &mut opts_view.report_enabled,
            "Ghi báo cáo trận đấu",
            "Ghi RAM, commit, CPU, ping mỗi 5 giây khi game chạy",
        ) {
            *save_needed = true;
        }
    });
}

/// Kết quả một dòng app đang chạy: người dùng vừa tick/bỏ tick hay vừa bấm "Là game".
#[derive(PartialEq, Clone, Copy)]
enum RowAction {
    None,
    ToggleKeep,
    MarkGame,
}

/// Một dòng app đang chạy: tick "Giữ lại" = không tắt app này; nút nhỏ "Là game" = thêm nhanh
/// exe vào danh sách game để watcher tự tối ưu.
fn running_app_row(
    ui: &mut egui::Ui,
    keep: &mut bool,
    app: &RunningApp,
    is_game: bool,
) -> RowAction {
    use egui::{pos2, vec2, Align2, CursorIcon, FontId, Rect, Sense};

    let row_h = ui::ROW_APP_HEIGHT;
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
    let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    let mut action = if resp.clicked() {
        *keep = !*keep;
        RowAction::ToggleKeep
    } else {
        RowAction::None
    };
    let hovered = resp.hovered();

    let painter = ui.painter_at(rect);

    let bg = if hovered {
        ui::CARD_HOVER
    } else {
        Color32::TRANSPARENT
    };
    painter.rect_filled(rect, Rounding::ZERO, bg);

    // Thanh màu dọc bên trái theo trạng thái (xanh lá = giữ, đỏ = sẽ tắt)
    let bar_color = if *keep { ui::SUCCESS } else { ui::DANGER };
    let bar = Rect::from_min_size(rect.min + vec2(0.0, 6.0), vec2(3.0, row_h - 12.0));
    painter.rect_filled(bar, Rounding::same(1.0), bar_color);

    let switch_rect = Rect::from_center_size(
        pos2(rect.left() + 20.0, rect.center().y),
        vec2(28.0, 16.0),
    );
    ui::draw_switch(&painter, switch_rect, *keep, hovered);

    let title = app.display.clone().unwrap_or_else(|| app.exe_name.clone());
    let name_color = if *keep {
        ui::SUCCESS
    } else {
        ui::TEXT_MAIN
    };

    let x0 = rect.left() + 40.0;
    let y_top = rect.top() + 16.0;
    let y_sub = rect.top() + 38.0;

    // Cột bên phải cố định rộng 76px (từ rect.right() - 76.0 đến rect.right() - 8.0)
    // Chừa khoảng đệm 8px: text chỉ được chiếm đến rect.right() - 84.0
    let text_right = rect.right() - 84.0;
    let max_text_w = (text_right - x0).max(40.0);

    // Dùng text_painter với clip_rect đảm bảo chữ tuyệt đối không tràn sang cột phải
    let text_clip = Rect::from_min_max(rect.min, pos2(text_right, rect.bottom()));
    let text_painter = painter.with_clip_rect(text_clip);

    // Dòng 1: Tên hiển thị (cắt bằng "…" nếu dài hơn chỗ trống, 1 dòng duy nhất)
    let mut title_job = egui::text::LayoutJob::simple(
        title,
        FontId::proportional(12.5),
        name_color,
        max_text_w,
    );
    title_job.wrap.max_rows = 1;
    title_job.wrap.break_anywhere = true;
    let title_galley = text_painter.layout_job(title_job);
    text_painter.galley(
        pos2(x0, y_top - title_galley.size().y / 2.0),
        title_galley,
        name_color,
    );

    // Dòng 2: Tên exe nhỏ (cắt bằng "…") + mini bar RAM + "N proc — X MB"
    let full_info_str = format!(" · {} proc — {}", app.count, fmt_ram(app.total_ws));
    let info_needed_w = text_painter.layout_no_wrap(
        full_info_str.clone(),
        FontId::proportional(10.5),
        ui::TEXT_MUTED,
    ).size().x;

    let mini_bar_w = 28.0_f32;
    let mini_bar_spacing = 6.0_f32;

    let exe_max_w = (max_text_w - info_needed_w - mini_bar_w - mini_bar_spacing).max(24.0);
    let mut exe_job = egui::text::LayoutJob::simple(
        app.exe_name.clone(),
        FontId::monospace(10.0),
        ui::TEXT_DIM,
        exe_max_w,
    );
    exe_job.wrap.max_rows = 1;
    exe_job.wrap.break_anywhere = true;
    let exe_galley = text_painter.layout_job(exe_job);

    let exe_w = exe_galley.size().x;
    text_painter.galley(
        pos2(x0, y_sub - exe_galley.size().y / 2.0),
        exe_galley,
        ui::TEXT_DIM,
    );

    // Mini bar RAM
    let mb_x = x0 + exe_w + 4.0;
    let mb_rect = Rect::from_min_size(pos2(mb_x, y_sub - 2.0), vec2(mini_bar_w, 4.0));
    let ram_ratio = (app.total_ws as f32 / (1024.0 * 1024.0 * 1024.0)).clamp(0.08, 1.0);
    text_painter.rect_filled(mb_rect, Rounding::ZERO, Color32::from_rgb(20, 27, 36));
    let filled_w = (mini_bar_w * ram_ratio).max(2.0);
    let mb_fill_color = if *keep { ui::DATA_CYAN } else { ui::DANGER };
    text_painter.rect_filled(
        Rect::from_min_size(mb_rect.min, vec2(filled_w, 4.0)),
        Rounding::ZERO,
        mb_fill_color,
    );

    let info_x = mb_x + mini_bar_w + 4.0;
    let info_rem_w = (text_right - info_x).max(10.0);
    let mut info_job = egui::text::LayoutJob::simple(
        full_info_str,
        FontId::proportional(10.5),
        ui::TEXT_MUTED,
        info_rem_w,
    );
    info_job.wrap.max_rows = 1;
    info_job.wrap.break_anywhere = true;
    let info_galley = text_painter.layout_job(info_job);
    text_painter.galley(
        pos2(info_x, y_sub - info_galley.size().y / 2.0),
        info_galley,
        ui::TEXT_MUTED,
    );

    // Cột bên phải: nhãn GIỮ LẠI (xanh lá) / SẼ TẮT (đỏ)
    let (badge, badge_color) = if *keep {
        ("GIỮ LẠI", ui::SUCCESS)
    } else {
        ("SẼ TẮT", ui::DANGER)
    };
    painter.text(
        pos2(rect.right() - 8.0, y_top),
        Align2::RIGHT_CENTER,
        badge,
        FontId::proportional(10.0),
        badge_color,
    );

    // Hover hiện đầy đủ tên hiển thị + tên exe + đường dẫn exe (nếu có)
    let hover_text = {
        let display_str = app.display.as_deref().unwrap_or(&app.exe_name);
        let path_str = app
            .exe_path
            .as_deref()
            .unwrap_or("Không đọc được đường dẫn exe của process này.");
        if app.display.as_deref().map_or(false, |d| !d.eq_ignore_ascii_case(&app.exe_name)) {
            format!("{}\n{}\n{}", display_str, app.exe_name, path_str)
        } else {
            format!("{}\n{}", app.exe_name, path_str)
        }
    };
    resp.on_hover_text(hover_text);

    // Cột bên phải: Nút nhỏ "Là game" (hoặc nhãn ✔ nếu exe đã nằm trong danh sách game).
    let btn_rect = Rect::from_center_size(
        pos2(rect.right() - 42.0, y_sub),
        vec2(68.0, 20.0),
    );
    if is_game {
        painter.text(
            btn_rect.center(),
            Align2::CENTER_CENTER,
            "✔ LÀ GAME",
            FontId::proportional(9.5),
            ui::SUCCESS,
        );
    } else {
        let btn = egui::Button::new(
            RichText::new("Là game")
                .size(9.5)
                .color(ui::ACCENT),
        )
        .fill(ui::CARD_BG)
        .stroke(Stroke::new(1.0_f32, ui::BORDER))
        .rounding(Rounding::ZERO);
        if ui.put(btn_rect, btn).on_hover_text("Thêm exe này vào danh sách game để tự tối ưu khi mở game").clicked() {
            action = RowAction::MarkGame;
        }
    }

    action
}

/// "2026-05-05 18:50:12" -> "18:50"
fn time_part(saved_at: &str) -> String {
    if saved_at.len() >= 16 {
        saved_at[11..16].to_string()
    } else {
        saved_at.to_string()
    }
}

/// Rút gọn tên exe game cho gọn gàng trên HUD.
fn clean_game_name(raw: &str) -> String {
    let lower = raw.trim();
    let no_ext = if let Some(stripped) = lower.strip_suffix(".exe") {
        stripped
    } else if let Some(stripped) = lower.strip_suffix(".EXE") {
        stripped
    } else {
        lower
    };
    if no_ext.chars().count() > 16 {
        format!("{}…", no_ext.chars().take(15).collect::<String>())
    } else {
        no_ext.to_string()
    }
}

/// "2026-10-22 01:12:29" -> "10-22 01:12"
fn format_short_date(s: &str) -> String {
    if s.len() >= 16 {
        s[5..16].to_string()
    } else if !s.is_empty() {
        s.to_string()
    } else {
        "—".to_string()
    }
}

/// Định dạng thời lượng theo giây -> chuỗi hiển thị gọn.
fn format_duration(sec: u64) -> String {
    if sec < 60 {
        format!("{}s", sec)
    } else if sec < 3600 {
        format!("{}m {:02}s", sec / 60, sec % 60)
    } else {
        format!("{}h {:02}m", sec / 3600, (sec % 3600) / 60)
    }
}

/// Tính nhanh summary khi trận đang diễn ra (chưa kết thúc để có summary đầy đủ từ watcher).
fn synthesize_live_summary(rep: &mut report::GameReport) {
    if rep.samples.is_empty() {
        if rep.summary.verdict.is_empty() {
            rep.summary.verdict = vec!["Đang thu thập mẫu số liệu trận đấu...".to_string()];
        }
        return;
    }
    let last_t = rep.samples.last().map(|s| s.t).unwrap_or(0.0);
    rep.summary.duration_sec = last_t as u64;

    let mut min_avail = f32::MAX;
    let mut sum_avail = 0.0_f32;
    let mut max_commit_pct = 0.0_f32;
    let mut max_commit_gb = 0.0_f32;
    let mut sum_cpu = 0.0_f32;
    let mut max_cpu = 0.0_f32;
    let mut net_pings = Vec::new();
    let mut sum_gw = 0.0_f32;
    let mut count_gw = 0;
    let mut sum_loss_net = 0.0_f32;
    let mut sum_loss_gw = 0.0_f32;

    for s in &rep.samples {
        if s.avail_gb < min_avail { min_avail = s.avail_gb; }
        sum_avail += s.avail_gb;
        if s.commit_pct > max_commit_pct { max_commit_pct = s.commit_pct; }
        if s.commit_gb > max_commit_gb { max_commit_gb = s.commit_gb; }
        sum_cpu += s.cpu_pct;
        if s.cpu_pct > max_cpu { max_cpu = s.cpu_pct; }
        if let Some(p) = s.ping_net_ms {
            net_pings.push(p);
        }
        if let Some(p) = s.ping_gw_ms {
            sum_gw += p;
            count_gw += 1;
        }
        sum_loss_net += s.loss_net_pct;
        sum_loss_gw += s.loss_gw_pct;
    }

    let n = rep.samples.len() as f32;
    rep.summary.ram_avail_min_gb = if min_avail == f32::MAX { 0.0 } else { min_avail };
    rep.summary.ram_avail_avg_gb = sum_avail / n;
    rep.summary.commit_peak_pct = max_commit_pct;
    rep.summary.commit_peak_gb = max_commit_gb;
    rep.summary.cpu_avg_pct = sum_cpu / n;
    rep.summary.cpu_peak_pct = max_cpu;
    rep.summary.loss_net_pct = sum_loss_net / n;
    rep.summary.loss_gw_pct = sum_loss_gw / n;
    if count_gw > 0 {
        rep.summary.ping_gw_avg_ms = Some(sum_gw / count_gw as f32);
    }
    if !net_pings.is_empty() {
        let sum_p: f32 = net_pings.iter().sum();
        rep.summary.ping_net_avg_ms = Some(sum_p / net_pings.len() as f32);
        net_pings.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let p95_idx = ((net_pings.len() as f32 * 0.95).round() as usize).min(net_pings.len() - 1);
        rep.summary.ping_net_p95_ms = Some(net_pings[p95_idx]);
        rep.summary.ping_net_max_ms = net_pings.last().copied();

        let mut sum_diff = 0.0_f32;
        let mut diff_count = 0;
        for i in 1..net_pings.len() {
            sum_diff += (net_pings[i] - net_pings[i - 1]).abs();
            diff_count += 1;
        }
        if diff_count > 0 {
            rep.summary.jitter_net_ms = Some(sum_diff / diff_count as f32);
        }
    }
    rep.summary.standby_purges = rep.events.iter().filter(|e| e.kind == "purge").count() as u32;
    if rep.summary.verdict.is_empty() {
        rep.summary.verdict = vec!["Trận đang diễn ra — số liệu được cập nhật theo thời gian thực mỗi 5 giây.".to_string()];
    }
}

/// Một ô hiển thị chỉ số nhỏ trong tổng kết trận đấu.
fn render_metric_box(
    ui: &mut egui::Ui,
    title: &str,
    primary_value: &str,
    secondary_value: Option<&str>,
    value_color: Color32,
) {
    ui::card(ui, |ui| {
        ui.label(
            RichText::new(title.to_uppercase())
                .size(9.5)
                .extra_letter_spacing(0.8)
                .color(ui::TEXT_MUTED),
        );
        ui.add_space(2.0);
        ui.label(
            RichText::new(primary_value)
                .size(15.0)
                .strong()
                .color(value_color),
        );
        if let Some(sub) = secondary_value {
            ui.label(
                RichText::new(sub)
                    .size(9.5)
                    .color(ui::TEXT_DIM),
            );
        } else {
            ui.add_space(12.0);
        }
    });
}

/// Vẽ các vạch dọc mờ thể hiện sự kiện trên các biểu đồ.
fn render_plot_events(plot_ui: &mut egui_plot::PlotUi, events: &[report::Event]) {
    for ev in events {
        let x = (ev.t / 60.0) as f64;
        let color = match ev.kind.as_str() {
            "optimize" => ui::ACCENT,
            "purge" => ui::DATA_CYAN,
            "commit_warn" => ui::DANGER,
            "ping_spike" => ui::WARNING,
            "restore" => ui::SUCCESS,
            _ => ui::TEXT_MUTED,
        };
        let stroke = Stroke::new(
            1.0_f32,
            Color32::from_rgba_premultiplied(color.r(), color.g(), color.b(), 110),
        );
        let vline = VLine::new(x)
            .name(format!("{}: {}", ev.kind, ev.text))
            .stroke(stroke)
            .style(LineStyle::Dashed { length: 4.0 });
        plot_ui.vline(vline);
    }
}

/// Biểu đồ (a): RAM Khả dụng và Standby (GB).
fn render_plot_ram(ui: &mut egui::Ui, report: &report::GameReport) {
    ui::card(ui, |ui| {
        ui::section_header(ui, "// RAM", "RAM Khả dụng & Standby", Some("GB theo thời gian (phút)"));
        ui.add_space(6.0);

        let avail_pts: Vec<[f64; 2]> = report.samples.iter()
            .map(|s| [(s.t / 60.0) as f64, s.avail_gb as f64])
            .collect();
        let standby_pts: Vec<[f64; 2]> = report.samples.iter()
            .map(|s| [(s.t / 60.0) as f64, s.standby_gb.unwrap_or(0.0) as f64])
            .collect();

        let l_avail = Line::new(PlotPoints::new(avail_pts))
            .name("Khả dụng (GB)")
            .color(ui::DATA_CYAN)
            .width(1.5_f32);
        let l_standby = Line::new(PlotPoints::new(standby_pts))
            .name("Standby (GB)")
            .color(ui::ACCENT)
            .width(1.5_f32);

        let plot = Plot::new("plot_ram")
            .height(175.0)
            .show_background(false)
            .show_axes([true, true])
            .show_grid([true, true])
            .allow_scroll(false)
            .allow_drag(false)
            .allow_zoom(false)
            .include_y(0.0)
            .legend(egui_plot::Legend::default().position(egui_plot::Corner::RightTop))
            .x_axis_formatter(|mark, _| format!("{:.1}m", mark.value))
            .y_axis_formatter(|mark, _| format!("{:.1} GB", mark.value))
            .label_formatter(|name, val| {
                if !name.is_empty() {
                    format!("{}: {:.2} GB · {:.1}m", name, val.y, val.x)
                } else {
                    format!("{:.2} GB · {:.1}m", val.y, val.x)
                }
            });

        plot.show(ui, |plot_ui| {
            plot_ui.line(l_avail);
            plot_ui.line(l_standby);
            render_plot_events(plot_ui, &report.events);
        });
    });
}

/// Biểu đồ (b): Bộ nhớ ảo Commit % kèm vạch ngang 90% màu đỏ.
fn render_plot_commit(ui: &mut egui::Ui, report: &report::GameReport) {
    ui::card(ui, |ui| {
        ui::section_header(ui, "// COMMIT", "Bộ nhớ ảo (Commit %)", Some("Vạch đỏ nguy hiểm 90%"));
        ui.add_space(6.0);

        let commit_pts: Vec<[f64; 2]> = report.samples.iter()
            .map(|s| [(s.t / 60.0) as f64, s.commit_pct as f64])
            .collect();

        let l_commit = Line::new(PlotPoints::new(commit_pts))
            .name("Commit (%)")
            .color(ui::ACCENT)
            .width(1.5_f32);

        let h_warn = HLine::new(90.0)
            .name("Cảnh báo 90%")
            .color(ui::DANGER)
            .stroke(Stroke::new(1.2_f32, ui::DANGER))
            .style(LineStyle::Dashed { length: 5.0 });

        let plot = Plot::new("plot_commit")
            .height(175.0)
            .show_background(false)
            .show_axes([true, true])
            .show_grid([true, true])
            .allow_scroll(false)
            .allow_drag(false)
            .allow_zoom(false)
            .include_y(0.0)
            .include_y(100.0)
            .legend(egui_plot::Legend::default().position(egui_plot::Corner::RightTop))
            .x_axis_formatter(|mark, _| format!("{:.1}m", mark.value))
            .y_axis_formatter(|mark, _| format!("{:.0}%", mark.value))
            .label_formatter(|name, val| {
                if !name.is_empty() {
                    format!("{}: {:.1}% · {:.1}m", name, val.y, val.x)
                } else {
                    format!("{:.1}% · {:.1}m", val.y, val.x)
                }
            });

        plot.show(ui, |plot_ui| {
            plot_ui.line(l_commit);
            plot_ui.hline(h_warn);
            render_plot_events(plot_ui, &report.events);
        });
    });
}

/// Biểu đồ (c): CPU % toàn hệ thống.
fn render_plot_cpu(ui: &mut egui::Ui, report: &report::GameReport) {
    ui::card(ui, |ui| {
        ui::section_header(ui, "// CPU", "Tải CPU (%)", Some("Toàn hệ thống theo thời gian (phút)"));
        ui.add_space(6.0);

        let cpu_pts: Vec<[f64; 2]> = report.samples.iter()
            .map(|s| [(s.t / 60.0) as f64, s.cpu_pct as f64])
            .collect();

        let l_cpu = Line::new(PlotPoints::new(cpu_pts))
            .name("CPU (%)")
            .color(Color32::from_rgb(130, 180, 255))
            .width(1.5_f32);

        let plot = Plot::new("plot_cpu")
            .height(175.0)
            .show_background(false)
            .show_axes([true, true])
            .show_grid([true, true])
            .allow_scroll(false)
            .allow_drag(false)
            .allow_zoom(false)
            .include_y(0.0)
            .include_y(100.0)
            .legend(egui_plot::Legend::default().position(egui_plot::Corner::RightTop))
            .x_axis_formatter(|mark, _| format!("{:.1}m", mark.value))
            .y_axis_formatter(|mark, _| format!("{:.0}%", mark.value))
            .label_formatter(|name, val| {
                if !name.is_empty() {
                    format!("{}: {:.1}% · {:.1}m", name, val.y, val.x)
                } else {
                    format!("{:.1}% · {:.1}m", val.y, val.x)
                }
            });

        plot.show(ui, |plot_ui| {
            plot_ui.line(l_cpu);
            render_plot_events(plot_ui, &report.events);
        });
    });
}

/// Biểu đồ (d): Ping Internet và Router (ms), các điểm mất gói đánh dấu đỏ.
fn render_plot_ping(ui: &mut egui::Ui, report: &report::GameReport) {
    ui::card(ui, |ui| {
        ui::section_header(ui, "// NET", "Độ trễ Ping & Mất gói", Some("Internet & Router (ms)"));
        ui.add_space(6.0);

        let mut net_pts = Vec::new();
        let mut gw_pts = Vec::new();
        let mut loss_pts = Vec::new();

        for s in &report.samples {
            let t_min = (s.t / 60.0) as f64;
            if let Some(p) = s.ping_net_ms {
                net_pts.push([t_min, p as f64]);
            }
            if let Some(p) = s.ping_gw_ms {
                gw_pts.push([t_min, p as f64]);
            }
            if s.loss_net_pct > 0.0 || s.loss_gw_pct > 0.0 || s.ping_net_ms.is_none() {
                let y = s.ping_net_ms.or(s.ping_gw_ms).unwrap_or(0.0) as f64;
                loss_pts.push([t_min, y]);
            }
        }

        let l_net = Line::new(PlotPoints::new(net_pts))
            .name("Internet (ms)")
            .color(ui::DATA_CYAN)
            .width(1.5_f32);
        let l_gw = Line::new(PlotPoints::new(gw_pts))
            .name("Router (ms)")
            .color(ui::SUCCESS)
            .width(1.2_f32);

        let plot = Plot::new("plot_ping")
            .height(175.0)
            .show_background(false)
            .show_axes([true, true])
            .show_grid([true, true])
            .allow_scroll(false)
            .allow_drag(false)
            .allow_zoom(false)
            .include_y(0.0)
            .legend(egui_plot::Legend::default().position(egui_plot::Corner::RightTop))
            .x_axis_formatter(|mark, _| format!("{:.1}m", mark.value))
            .y_axis_formatter(|mark, _| format!("{:.0} ms", mark.value))
            .label_formatter(|name, val| {
                if !name.is_empty() {
                    format!("{}: {:.1} ms · {:.1}m", name, val.y, val.x)
                } else {
                    format!("{:.1} ms · {:.1}m", val.y, val.x)
                }
            });

        plot.show(ui, |plot_ui| {
            plot_ui.line(l_net);
            plot_ui.line(l_gw);
            if !loss_pts.is_empty() {
                let p_loss = Points::new(PlotPoints::new(loss_pts))
                    .name("Mất gói")
                    .color(ui::DANGER)
                    .radius(4.0_f32)
                    .shape(MarkerShape::Cross);
                plot_ui.points(p_loss);
            }
            render_plot_events(plot_ui, &report.events);
        });
    });
}

fn render_report_detail(
    ui: &mut egui::Ui,
    report: &report::GameReport,
    is_live: bool,
    delete_confirm: &Option<(String, Instant)>,
    delete_req: &mut Option<String>,
    confirm_req: &mut Option<String>,
) {
    // 1. Header chi tiết: Tên game, ngày bắt đầu, nút xoá
    ui::card(ui, |ui| {
        ui.horizontal(|ui| {
            let game_title = clean_game_name(&report.game);
            ui.label(
                RichText::new(game_title.to_uppercase())
                    .size(15.0)
                    .strong()
                    .extra_letter_spacing(1.0)
                    .color(ui::TEXT_MAIN),
            );
            ui.add_space(8.0);

            if is_live {
                ui.label(
                    RichText::new("● ĐANG GHI SỐ LIỆU")
                        .size(11.0)
                        .strong()
                        .color(ui::DANGER),
                );
            } else {
                ui.label(
                    RichText::new(format!("{}  ·  ID: {}", report.started_at, report.id))
                        .size(10.5)
                        .monospace()
                        .color(ui::TEXT_DIM),
                );
            }

            if !is_live {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let is_confirming = match delete_confirm {
                        Some((ref id, inst)) if id == &report.id && inst.elapsed() < Duration::from_secs(3) => {
                            let rem = (3.0 - inst.elapsed().as_secs_f32()).max(0.1);
                            Some(rem)
                        }
                        _ => None,
                    };

                    if let Some(rem_sec) = is_confirming {
                        let btn_text = format!("⚠ Bấm lại để xoá ({:.0}s)", rem_sec.ceil());
                        if ui.add(ui::danger_button_sized(btn_text, 11.0)).clicked() {
                            *delete_req = Some(report.id.clone());
                        }
                    } else if ui.add(ui::ghost_button_sized("🗑 Xoá báo cáo", 11.0)).clicked() {
                        *confirm_req = Some(report.id.clone());
                    }
                });
            }
        });
    });

    ui.add_space(ui::SPACE_CARD_GAP);

    // 2. Hàng thẻ chỉ số (9 chỉ số)
    ui::card(ui, |ui| {
        ui::section_header(ui, "// STATS", "Chỉ số tổng kết trận", None);
        ui.add_space(8.0);

        let sum = &report.summary;
        let dur_text = format_duration(sum.duration_sec);
        let ram_min_text = format!("{:.2} GB", sum.ram_avail_min_gb);
        let ram_color = if sum.ram_avail_min_gb < 1.0 {
            ui::DANGER
        } else if sum.ram_avail_min_gb < 2.0 {
            ui::WARNING
        } else {
            ui::SUCCESS
        };

        let commit_color = if sum.commit_peak_pct >= 90.0 {
            ui::DANGER
        } else if sum.commit_peak_pct >= 80.0 {
            ui::ACCENT
        } else {
            ui::SUCCESS
        };
        let commit_text = format!("{:.1}%", sum.commit_peak_pct);
        let commit_sub = format!("{:.1} GB đỉnh", sum.commit_peak_gb);

        let cpu_text = format!("{:.0}% / {:.0}%", sum.cpu_avg_pct, sum.cpu_peak_pct);

        let ping_text = match (sum.ping_net_avg_ms, sum.ping_net_p95_ms, sum.ping_net_max_ms) {
            (Some(a), Some(p), Some(m)) => format!("{:.0}/{:.0}/{:.0} ms", a, p, m),
            (Some(a), _, _) => format!("{:.0} ms", a),
            _ => "—".to_string(),
        };

        let jitter_text = sum.jitter_net_ms.map(|j| format!("{:.1} ms", j)).unwrap_or_else(|| "—".to_string());

        let loss_text = format!("{:.1}%", sum.loss_net_pct);
        let loss_color = if sum.loss_net_pct > 0.0 { ui::DANGER } else { ui::SUCCESS };

        let gw_text = sum.ping_gw_avg_ms.map(|g| format!("{:.1} ms", g)).unwrap_or_else(|| "—".to_string());
        let gw_sub = format!("Loss: {:.1}%", sum.loss_gw_pct);

        let purge_text = format!("{} lần", sum.standby_purges);

        let is_wide = ui.available_width() >= 540.0;
        if is_wide {
            ui.columns(3, |cols| {
                render_metric_box(&mut cols[0], "Thời lượng", &dur_text, Some(&format!("Bắt đầu: {}", format_short_date(&report.started_at))), ui::TEXT_MAIN);
                cols[0].add_space(8.0);
                render_metric_box(&mut cols[0], "RAM trống thấp nhất", &ram_min_text, Some(&format!("TB: {:.2} GB", sum.ram_avail_avg_gb)), ram_color);
                cols[0].add_space(8.0);
                render_metric_box(&mut cols[0], "Commit đỉnh", &commit_text, Some(&commit_sub), commit_color);

                render_metric_box(&mut cols[1], "CPU TB / Đỉnh", &cpu_text, Some("Toàn hệ thống"), ui::DATA_CYAN);
                cols[1].add_space(8.0);
                render_metric_box(&mut cols[1], "Ping Net TB/P95/Max", &ping_text, Some("Internet"), ui::DATA_CYAN);
                cols[1].add_space(8.0);
                render_metric_box(&mut cols[1], "Jitter độ trễ", &jitter_text, Some("Biên độ dao động"), ui::TEXT_MAIN);

                render_metric_box(&mut cols[2], "Mất gói Net", &loss_text, Some("Tỉ lệ rớt gói"), loss_color);
                cols[2].add_space(8.0);
                render_metric_box(&mut cols[2], "Ping Router TB", &gw_text, Some(&gw_sub), ui::SUCCESS);
                cols[2].add_space(8.0);
                render_metric_box(&mut cols[2], "Xả Standby", &purge_text, Some("Giải phóng RAM"), ui::ACCENT);
            });
        } else {
            ui.columns(2, |cols| {
                render_metric_box(&mut cols[0], "Thời lượng", &dur_text, None, ui::TEXT_MAIN);
                cols[0].add_space(8.0);
                render_metric_box(&mut cols[0], "RAM trống thấp nhất", &ram_min_text, None, ram_color);
                cols[0].add_space(8.0);
                render_metric_box(&mut cols[0], "Commit đỉnh", &commit_text, Some(&commit_sub), commit_color);
                cols[0].add_space(8.0);
                render_metric_box(&mut cols[0], "CPU TB / Đỉnh", &cpu_text, None, ui::DATA_CYAN);
                cols[0].add_space(8.0);
                render_metric_box(&mut cols[0], "Xả Standby", &purge_text, None, ui::ACCENT);

                render_metric_box(&mut cols[1], "Ping Net TB/P95/Max", &ping_text, None, ui::DATA_CYAN);
                cols[1].add_space(8.0);
                render_metric_box(&mut cols[1], "Jitter độ trễ", &jitter_text, None, ui::TEXT_MAIN);
                cols[1].add_space(8.0);
                render_metric_box(&mut cols[1], "Mất gói Net", &loss_text, None, loss_color);
                cols[1].add_space(8.0);
                render_metric_box(&mut cols[1], "Ping Router TB", &gw_text, Some(&gw_sub), ui::SUCCESS);
            });
        }
    });

    ui.add_space(ui::SPACE_CARD_GAP);

    // 3. Khối nhận xét (verdict)
    ui::card(ui, |ui| {
        ui::section_header(ui, "// VERDICT", "Đánh giá & Nhận xét", None);
        ui.add_space(8.0);

        if report.summary.verdict.is_empty() {
            ui.label(
                RichText::new("Chưa có nhận xét cho trận này.")
                    .size(11.0)
                    .color(ui::TEXT_DIM),
            );
        } else {
            for line in &report.summary.verdict {
                ui::verdict_callout(ui, line);
                ui.add_space(4.0);
            }
        }
    });

    ui.add_space(ui::SPACE_CARD_GAP);

    // 4. 4 biểu đồ đường
    if report.samples.is_empty() {
        ui::card(ui, |ui| {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new("⏳ Đang thu thập mẫu số liệu trận đấu...")
                        .size(12.0)
                        .color(ui::TEXT_MUTED),
                );
            });
            ui.add_space(20.0);
        });
    } else {
        let is_wide = ui.available_width() >= 620.0;
        if is_wide {
            ui.columns(2, |cols| {
                render_plot_ram(&mut cols[0], report);
                render_plot_commit(&mut cols[1], report);
            });
            ui.add_space(ui::SPACE_CARD_GAP);
            ui.columns(2, |cols| {
                render_plot_cpu(&mut cols[0], report);
                render_plot_ping(&mut cols[1], report);
            });
        } else {
            render_plot_ram(ui, report);
            ui.add_space(ui::SPACE_CARD_GAP);
            render_plot_commit(ui, report);
            ui.add_space(ui::SPACE_CARD_GAP);
            render_plot_cpu(ui, report);
            ui.add_space(ui::SPACE_CARD_GAP);
            render_plot_ping(ui, report);
        }
    }
}

impl App {
    fn render_reports_tab(&mut self, ui: &mut egui::Ui) {
        if self.reports_cache.is_none() {
            self.refresh_reports();
        }

        let live_report_arc = self.sh.live_report.lock().unwrap().clone();
        let reports_list = self.reports_cache.as_ref().cloned().unwrap_or_default();

        // Mặc định chọn nếu chưa có lựa chọn
        if self.selected_report_id.is_none() {
            if live_report_arc.is_some() {
                self.selected_report_id = Some("live".into());
            } else if let Some(first) = reports_list.first() {
                self.selected_report_id = Some(first.id.clone());
                self.cached_report = report::load_report(&first.id);
            }
        }

        let total_w = ui.available_width();
        let list_w = 260.0_f32.min(total_w * 0.45).max(180.0);
        let detail_w = (total_w - list_w - ui::SPACE_CARD_GAP).max(180.0);

        let mut refresh_requested = false;
        let mut select_action: Option<String> = None;
        let mut delete_req: Option<String> = None;
        let mut confirm_req: Option<String> = None;

        ui.horizontal_top(|ui| {
            // CỘT TRÁI TAB: DANH SÁCH BÁO CÁO (~260px)
            ui.allocate_ui_with_layout(
                vec2(list_w, ui.available_height()),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui::card(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui::section_header(ui, "// LIST", "Danh sách", None);
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.add(ui::ghost_button_sized("🔄 Làm mới", 10.5)).clicked() {
                                    refresh_requested = true;
                                }
                            });
                        });
                        ui.add_space(8.0);

                        egui::ScrollArea::vertical()
                            .id_salt("reports_list_scroll")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                // 1. Trận đang ghi (nếu có)
                                if let Some(ref l_arc) = live_report_arc {
                                    let (game_name, dur_sec) = {
                                        let l = l_arc.lock().unwrap();
                                        (clean_game_name(&l.report.game), l.report.samples.last().map(|s| s.t as u64).unwrap_or(0))
                                    };
                                    let is_sel = self.selected_report_id.as_deref() == Some("live");
                                    if ui::report_list_row(
                                        ui,
                                        is_sel,
                                        "● ĐANG GHI",
                                        &game_name,
                                        &format_duration(dur_sec),
                                        ui::DANGER,
                                        true,
                                    ).clicked() {
                                        select_action = Some("live".to_string());
                                    }
                                    ui.add_space(4.0);
                                }

                                // 2. Các file báo cáo trên đĩa
                                if reports_list.is_empty() && live_report_arc.is_none() {
                                    ui.add_space(20.0);
                                    ui.label(
                                        RichText::new("Chưa có báo cáo nào.")
                                            .size(11.0)
                                            .color(ui::TEXT_DIM),
                                    );
                                } else {
                                    for meta in &reports_list {
                                        let is_sel = self.selected_report_id.as_deref() == Some(&meta.id);
                                        let status_col = if meta.commit_peak_pct >= 90.0 {
                                            ui::DANGER
                                        } else if meta.commit_peak_pct >= 80.0 {
                                            ui::ACCENT
                                        } else {
                                            ui::SUCCESS
                                        };
                                        let time_str = format_short_date(&meta.started_at);
                                        let game_short = clean_game_name(&meta.game);
                                        let dur_str = format_duration(meta.duration_sec);

                                        if ui::report_list_row(
                                            ui,
                                            is_sel,
                                            &time_str,
                                            &game_short,
                                            &dur_str,
                                            status_col,
                                            false,
                                        ).clicked() {
                                            select_action = Some(meta.id.clone());
                                        }
                                        ui.add_space(4.0);
                                    }
                                }
                            });
                    });
                },
            );

            ui.add_space(ui::SPACE_CARD_GAP);

            // CỘT PHẢI TAB: CHI TIẾT BÁO CÁO ĐANG CHỌN
            ui.allocate_ui_with_layout(
                vec2(detail_w, ui.available_height()),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    let is_live_sel = self.selected_report_id.as_deref() == Some("live");
                    let report_opt: Option<report::GameReport> = if is_live_sel {
                        live_report_arc.as_ref().map(|l| {
                            let live = l.lock().unwrap();
                            let mut rep = live.report.clone();
                            synthesize_live_summary(&mut rep);
                            rep
                        })
                    } else if let Some(ref rep) = self.cached_report {
                        Some(rep.clone())
                    } else if let Some(ref id) = self.selected_report_id {
                        let loaded = report::load_report(id);
                        self.cached_report = loaded.clone();
                        loaded
                    } else {
                        None
                    };

                    if let Some(rep) = report_opt {
                        egui::ScrollArea::vertical()
                            .id_salt("report_detail_scroll")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                render_report_detail(
                                    ui,
                                    &rep,
                                    is_live_sel,
                                    &self.delete_confirm,
                                    &mut delete_req,
                                    &mut confirm_req,
                                );
                            });
                    } else {
                        ui::card(ui, |ui| {
                            ui.add_space(40.0);
                            ui.vertical_centered(|ui| {
                                ui.label(
                                    RichText::new("📊")
                                        .size(36.0)
                                        .color(ui::ACCENT),
                                );
                                ui.add_space(10.0);
                                ui.label(
                                    RichText::new("CHƯA CÓ BÁO CÁO TRẬN")
                                        .size(14.0)
                                        .strong()
                                        .extra_letter_spacing(1.0)
                                        .color(ui::TEXT_MAIN),
                                );
                                ui.add_space(8.0);
                                ui.label(
                                    RichText::new("Mở game trong danh sách, GameMode sẽ tự ghi số liệu và lưu báo cáo khi bạn tắt game.")
                                        .size(11.5)
                                        .color(ui::TEXT_MUTED),
                                );
                            });
                            ui.add_space(40.0);
                        });
                    }
                },
            );
        });

        // Xử lý các action sau closure layout
        if refresh_requested {
            self.refresh_reports();
        }
        if let Some(id) = select_action {
            self.select_report(&id);
        }
        if let Some(id) = delete_req {
            report::delete_report(&id);
            self.delete_confirm = None;
            self.cached_report = None;
            self.refresh_reports();
        } else if let Some(id) = confirm_req {
            self.delete_confirm = Some((id, Instant::now()));
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.start_hidden {
            self.start_hidden = false;
            // --minimized: vào thẳng khay (cửa sổ ẩn) chứ không thu xuống taskbar.
            // eframe tự hiện cửa sổ sau frame đầu, nên ẩn lại thêm vài lần trong giây đầu.
            let hwnd = self.hwnd;
            sys::hide_main_window(hwnd);
            std::thread::spawn(move || {
                for ms in [150u64, 400, 900] {
                    std::thread::sleep(Duration::from_millis(ms));
                    sys::hide_main_window(hwnd);
                }
            });
        }
        if self.last_mem.elapsed() > Duration::from_millis(800) {
            self.mem = sys::memory();
            self.last_mem = Instant::now();
        }
        // Danh sách trang bộ nhớ (free / standby) — rẻ, chỉ đọc khi thẻ RAM cần hiển thị.
        if self.last_ml.elapsed() > Duration::from_millis(2000) {
            self.mem_lists = sys::memory_lists();
            self.last_ml = Instant::now();
        }
        // Trạng thái task "GameMode" hỏi lại định kỳ (schtasks là tiến trình riêng, khá tốn).
        if self.last_task_check.elapsed() > Duration::from_secs(20) {
            self.task_on = sys::task_exists();
            self.last_task_check = Instant::now();
        }
        // Commit từ sys::commit() cho thẻ Pagefile (tối đa mỗi 2 giây).
        if self.last_commit_sys.elapsed() > Duration::from_millis(2000) {
            self.commit_sys = sys::commit();
            self.last_commit_sys = Instant::now();
        }
        // Với trận đang ghi: repaint mỗi ~2 giây khi tab Báo cáo đang mở
        if self.tab == ConfigTab::Reports && self.sh.live_report.lock().unwrap().is_some() {
            ctx.request_repaint_after(Duration::from_secs(2));
        } else {
            ctx.request_repaint_after(Duration::from_millis(400));
        }
        let sh = self.sh.clone();

        // Đóng cửa sổ (nút X trên title bar, Alt+F4, ...): mặc định thu xuống khay hệ thống,
        // chỉ thoát hẳn khi chọn "Thoát" ở menu khay hoặc khi tắt ui:close_to_tray.
        if ctx.input(|i| i.viewport().close_requested()) {
            // Không có icon khay hoặc không lấy được HWND thì ẩn đi sẽ không hiện lại được
            // -> cứ đóng như cũ.
            let can_tray = self.tray.is_some() && self.hwnd != 0;
            if !tray::QUIT.load(Ordering::SeqCst) && sh.opts().close_to_tray && can_tray {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                sys::hide_main_window(self.hwnd);
            } else {
                // Bỏ icon khay trước khi đóng app.
                if let Some(mut t) = self.tray.take() {
                    t.remove();
                }
            }
        }
        // Tooltip / trạng thái disable / dấu tick của menu khay bám theo trạng thái thật.
        if let Some(t) = self.tray.as_mut() {
            t.sync(&sh);
        }

        let busy = sh.busy.load(Ordering::SeqCst);
        let scanning = self.scanning.load(Ordering::SeqCst);
        let apps_snapshot: Vec<RunningApp> = sh.running.lock().unwrap().clone();
        let session_view: Option<SessionData> = sh.session.lock().unwrap().clone();
        let games_view: Vec<String> = sh.games.lock().unwrap().clone();
        let mut opts_view = sh.opts();
        let ram_extra = self.mem_lists.map(|l| {
            format!(
                "Free {:.1} GB · Standby {:.1} GB",
                l.free_bytes as f64 / GB,
                l.standby_bytes as f64 / GB
            )
        });

        let kept = {
            let keep = sh.keep.lock().unwrap();
            apps_snapshot
                .iter()
                .filter(|a| keep_decision(&keep, &a.exe_lower))
                .count()
        };
        let svc_sel = sh.svc.lock().unwrap().iter().filter(|&&x| x).count();

        // 0. Title bar tuỳ biến ở trên cùng (phủ hết chiều rộng 2 cột)
        ui::render_title_bar(ctx, busy);

        // =========================================================
        // CỘT TRÁI: SidePanel (rộng 420, khoảng 380-480, resizable)
        // =========================================================
        egui::SidePanel::left("left_side_panel")
            .default_width(420.0)
            .width_range(380.0..=480.0)
            .resizable(true)
            .frame(egui::Frame::none().fill(ui::BG).inner_margin(Margin::same(ui::SPACE_PANEL_PAD)))
            .show(ctx, |ui| {
                ui::paint_background_grid(ui);

                // Banner cảnh báo commit ở đầu cột trái khi commit_alert = true
                if sh.commit_alert.load(Ordering::SeqCst) {
                    ui::render_commit_alert_banner(ui, sh.commit_pct());
                    ui.add_space(ui::SPACE_CARD_GAP);
                }

                // 1. Hai thẻ RAM vật lý và Pagefile nằm cạnh nhau
                let m = self.mem;
                let ram_used = m.total - m.avail;
                let ram_ratio = if m.total > 0 { ram_used as f32 / m.total as f32 } else { 0.0 };
                let ram_color = if ram_ratio > 0.85 {
                    ui::DANGER
                } else if ram_ratio > 0.70 {
                    ui::WARNING
                } else {
                    ui::SUCCESS
                };

                let commit_ratio = if m.commit_limit > 0 {
                    m.commit_used as f32 / m.commit_limit as f32
                } else {
                    0.0
                };
                let commit_color = if commit_ratio > 0.85 {
                    ui::DANGER
                } else if commit_ratio > 0.70 {
                    ui::WARNING
                } else {
                    ui::ACCENT
                };

                let commit_extra = format!(
                    "Commit {:.1} / {:.1} GB",
                    self.commit_sys.0 as f64 / GB,
                    self.commit_sys.1 as f64 / GB,
                );

                ui.columns(2, |cols| {
                    render_stat_card(
                        &mut cols[0],
                        "RAM VẬT LÝ",
                        &format!("Trống: {:.1} GB", m.avail as f64 / GB),
                        ram_used as f64 / GB,
                        m.total as f64 / GB,
                        ram_ratio,
                        ram_color,
                        ram_extra.as_deref(),
                    );
                    render_stat_card(
                        &mut cols[1],
                        "PAGEFILE",
                        if commit_ratio > 0.85 { "⚠ NGUY HIỂM" } else { "An toàn" },
                        m.commit_used as f64 / GB,
                        m.commit_limit as f64 / GB,
                        commit_ratio,
                        commit_color,
                        Some(&commit_extra),
                    );
                });

                ui.add_space(ui::SPACE_CARD_GAP);

                // 2. Nút chính BẬT GAME BOOST rộng hết cột + hàng Khôi phục / Xoá nhật ký
                let boost_btn_text = if busy {
                    "ĐANG THANH LỌC…"
                } else {
                    "🚀  BẬT GAME BOOST"
                };

                let boost_btn = ui::primary_button(boost_btn_text, busy);
                if ui.add_sized([ui.available_width(), ui::BTN_BOOST_HEIGHT], boost_btn).clicked() && !busy {
                    optimize(sh.clone(), false);
                }

                ui.add_space(8.0);

                ui.columns(2, |cols| {
                    let restore_btn = ui::ghost_button("↩  Khôi phục");
                    if cols[0].add_sized([cols[0].available_width(), ui::BTN_SUB_HEIGHT], restore_btn).clicked() && !busy {
                        restore(sh.clone());
                    }

                    let clear_btn = ui::ghost_button("Xoá nhật ký");
                    if cols[1].add_sized([cols[1].available_width(), ui::BTN_SUB_HEIGHT], clear_btn).clicked() {
                        sh.log.lock().unwrap().clear();
                    }
                });

                ui.add_space(ui::SPACE_CARD_GAP);

                // 3. Card session: trạng thái session đã lưu, danh sách app gói trong CollapsingHeader, mặc định đóng
                ui::card(ui, |ui| {
                    match session_view.as_ref() {
                        Some(s) => {
                            let title = format!(
                                "💾 Session đã lưu: {} app, {} service — lúc {}",
                                s.apps.len(),
                                s.services.len(),
                                time_part(&s.saved_at)
                            );
                            egui::CollapsingHeader::new(
                                RichText::new(title)
                                    .size(11.0)
                                    .strong()
                                    .color(ui::ACCENT),
                            )
                            .default_open(false)
                            .show(ui, |ui| {
                                ui.add_space(4.0);
                                ui.label(
                                    RichText::new("Tick app nào để Restore mở lại app đó:")
                                        .size(10.5)
                                        .color(ui::TEXT_MUTED),
                                );
                                ui.add_space(4.0);
                                egui::ScrollArea::vertical()
                                    .id_salt("session_apps_scroll")
                                    .max_height(120.0)
                                    .auto_shrink([false, false])
                                    .show(ui, |ui| {
                                        let mut sel = sh.restore_sel.lock().unwrap();
                                        for a in s.apps.iter() {
                                            let cwd = a.cwd.clone().unwrap_or_else(|| "(không rõ)".into());
                                            let checked = sel.entry(app_key(a)).or_insert(true);
                                            ui.horizontal(|ui| {
                                                ui::compact_switch(ui, checked);
                                                ui.label(
                                                    RichText::new(format!(
                                                        "{} [{}] — cwd: {}",
                                                        a.display, a.exe, cwd
                                                    ))
                                                    .monospace()
                                                    .size(10.0)
                                                    .color(ui::TEXT_MAIN),
                                                );
                                            });
                                            ui.add_space(2.0);
                                        }
                                        for svc in s.services.iter() {
                                            ui.label(
                                                RichText::new(format!("  • service: {}", svc))
                                                    .monospace()
                                                    .size(10.0)
                                                    .color(ui::SUCCESS),
                                            );
                                        }
                                    });
                            });
                        }
                        None => {
                            ui.label(
                                RichText::new("💾 Chưa có session nào được lưu — bấm Game Boost để chụp phiên hiện tại.")
                                    .size(10.5)
                                    .color(ui::TEXT_MUTED),
                            );
                        }
                    }
                });

                ui.add_space(ui::SPACE_CARD_GAP);

                // 4. Terminal log chiếm toàn bộ chiều cao còn lại của cột, scroll riêng, auto-scroll xuống cuối
                ui::card(ui, |ui| {
                    ui::section_header(ui, "AUTO-SCROLL", "Terminal Log", None);
                    ui.add_space(ui::SPACE_HEADER_GAP);

                    let log_height = ui.available_height().max(80.0);
                    let log_frame = egui::Frame::none()
                        .fill(Color32::from_rgb(5, 7, 9))
                        .stroke(Stroke::new(1.0_f32, ui::BORDER))
                        .inner_margin(Margin::same(8.0));

                    log_frame.show(ui, |ui| {
                        // Hiệu ứng scanline rất nhẹ (các đường ngang alpha thấp cách nhau 3px)
                        let log_rect = ui.max_rect();
                        if log_rect.is_positive() {
                            let scan_painter = ui.painter().with_clip_rect(log_rect);
                            let scan_stroke = Stroke::new(
                                1.0_f32,
                                Color32::from_rgba_premultiplied(0, 0, 0, 45),
                            );
                            let mut y = log_rect.top();
                            while y <= log_rect.bottom() {
                                scan_painter.line_segment(
                                    [pos2(log_rect.left(), y), pos2(log_rect.right(), y)],
                                    scan_stroke,
                                );
                                y += 3.0;
                            }
                        }

                        egui::ScrollArea::vertical()
                            .id_salt("terminal_log_scroll")
                            .stick_to_bottom(true)
                            .auto_shrink([false, false])
                            .max_height((log_height - 16.0).max(60.0))
                            .show(ui, |ui| {
                                for (line, color) in sh.log.lock().unwrap().iter() {
                                    let text_color = if line.contains('✔') {
                                        ui::SUCCESS
                                    } else if line.contains('⚠') {
                                        ui::ACCENT
                                    } else if line.contains("[ERR]")
                                        || line.to_lowercase().contains("lỗi")
                                        || line.to_lowercase().contains("error")
                                    {
                                        ui::DANGER
                                    } else {
                                        *color
                                    };
                                    ui.label(
                                        RichText::new(line)
                                            .monospace()
                                            .size(10.5)
                                            .color(text_color),
                                    );
                                    ui.add_space(ui::SPACE_LOG_LINE_GAP);
                                }
                            });
                    });
                });
            });

        // =========================================================
        // CỘT PHẢI: CentralPanel (Tabs trên cùng, ScrollArea riêng)
        // =========================================================
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(ui::BG).inner_margin(Margin::same(ui::SPACE_PANEL_PAD)))
            .show(ctx, |ui| {
                ui::paint_background_grid(ui);

                // Thanh tab ở trên cùng
                ui.horizontal(|ui| {
                    let is_app = self.tab == ConfigTab::Apps;
                    let app_tab_btn = ui::tab_button(
                        format!("💻 Ứng dụng đang chạy ({}/{})", kept, apps_snapshot.len()),
                        is_app,
                        ui::ACCENT,
                    );
                    if ui.add(app_tab_btn).clicked() {
                        self.tab = ConfigTab::Apps;
                    }

                    let is_svc = self.tab == ConfigTab::Services;
                    let svc_tab_btn = ui::tab_button(
                        format!("🛠 Services ({}/{})", svc_sel, SERVICE_TARGETS.len()),
                        is_svc,
                        ui::ACCENT,
                    );
                    if ui.add(svc_tab_btn).clicked() {
                        self.tab = ConfigTab::Services;
                    }

                    let is_auto = self.tab == ConfigTab::AutoOpt;
                    let auto_tab_btn = ui::tab_button(
                        "⚙ Tự động & Tối ưu",
                        is_auto,
                        ui::ACCENT,
                    );
                    if ui.add(auto_tab_btn).clicked() {
                        self.tab = ConfigTab::AutoOpt;
                    }

                    let is_rep = self.tab == ConfigTab::Reports;
                    let rep_tab_btn = ui::tab_button(
                        "📊 Báo cáo trận",
                        is_rep,
                        ui::ACCENT,
                    );
                    if ui.add(rep_tab_btn).clicked() {
                        self.tab = ConfigTab::Reports;
                        if self.reports_cache.is_none() {
                            self.refresh_reports();
                        }
                    }
                });

                ui.add_space(ui::SPACE_CARD_GAP);

                // Nội dung tab bên dưới chiếm hết chiều cao, mỗi tab một ScrollArea riêng
                match self.tab {
                    ConfigTab::Apps => {
                        // Thanh công cụ: đếm số app + các nút, dùng horizontal_wrapped không bao giờ chồng chữ
                        ui::card(ui, |ui| {
                            ui.spacing_mut().item_spacing.x = ui::SPACE_TOOLBAR_GAP;
                            ui.horizontal_wrapped(|ui| {
                                let (status, color) = match self.saved_at {
                                    Some((t, true)) if t.elapsed() < Duration::from_millis(2500) => {
                                        ("✔ Đã lưu lựa chọn", ui::SUCCESS)
                                    }
                                    Some((t, false)) if t.elapsed() < Duration::from_millis(4000) => {
                                        ("⚠ Không lưu được cấu hình", ui::DANGER)
                                    }
                                    _ => ("💾 Tự động ghi nhớ", ui::TEXT_MUTED),
                                };
                                ui.label(RichText::new(status).size(11.0).color(color));
                                ui.add_space(ui::SPACE_TOOLBAR_GAP);

                                if ui.add(ui::ghost_button_sized("Mặc định", 11.0)).clicked() {
                                    let mut keep = sh.keep.lock().unwrap();
                                    for a in apps_snapshot.iter() {
                                        keep.insert(a.exe_lower.clone(), default_keep(&a.exe_lower));
                                    }
                                    drop(keep);
                                    self.save();
                                }
                                if ui.add(ui::ghost_button_sized("Bỏ giữ tất cả", 11.0)).clicked() {
                                    let mut keep = sh.keep.lock().unwrap();
                                    for a in apps_snapshot.iter() {
                                        keep.insert(a.exe_lower.clone(), false);
                                    }
                                    drop(keep);
                                    self.save();
                                }
                                if ui.add(ui::ghost_button_sized("Giữ tất cả", 11.0)).clicked() {
                                    let mut keep = sh.keep.lock().unwrap();
                                    for a in apps_snapshot.iter() {
                                        keep.insert(a.exe_lower.clone(), true);
                                    }
                                    drop(keep);
                                    self.save();
                                }
                                if ui.add_enabled(
                                    !scanning,
                                    ui::ghost_button_sized(if scanning { "Đang quét..." } else { "🔄 Làm mới" }, 11.0),
                                ).clicked() {
                                    self.start_scan();
                                }
                            });
                        });

                        ui.add_space(ui::SPACE_TOOLBAR_MARGIN_BOTTOM);

                        egui::ScrollArea::vertical()
                            .id_salt("apps_scroll_area")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui::card(ui, |ui| {
                                    if apps_snapshot.is_empty() {
                                        ui.add_space(16.0);
                                        ui.label(
                                            RichText::new(if scanning {
                                                "Đang quét các app đang chạy..."
                                             } else {
                                                "Không thấy app nào đang chạy ngoài các process hệ thống được loại trừ."
                                            })
                                            .size(11.5)
                                            .color(ui::TEXT_MUTED),
                                        );
                                        ui.add_space(16.0);
                                    } else {
                                        let keep_view = sh.keep.lock().unwrap().clone();
                                        let is_grid = ui.available_width() >= 560.0;
                                        let mut changed = false;

                                        if is_grid {
                                            ui.spacing_mut().item_spacing.x = ui::SPACE_GRID_GAP;
                                            let half = (apps_snapshot.len() + 1) / 2;
                                            ui.columns(2, |cols| {
                                                for a in &apps_snapshot[..half] {
                                                    let mut k = keep_decision(&keep_view, &a.exe_lower);
                                                    let is_game = games_view.contains(&a.exe_lower);
                                                    match running_app_row(&mut cols[0], &mut k, a, is_game) {
                                                        RowAction::ToggleKeep => {
                                                            sh.keep.lock().unwrap().insert(a.exe_lower.clone(), k);
                                                            changed = true;
                                                        }
                                                        RowAction::MarkGame => mark_as_game(&sh, a),
                                                        RowAction::None => {}
                                                    }
                                                    cols[0].add_space(ui::SPACE_GRID_GAP);
                                                }
                                                for a in &apps_snapshot[half..] {
                                                    let mut k = keep_decision(&keep_view, &a.exe_lower);
                                                    let is_game = games_view.contains(&a.exe_lower);
                                                    match running_app_row(&mut cols[1], &mut k, a, is_game) {
                                                        RowAction::ToggleKeep => {
                                                            sh.keep.lock().unwrap().insert(a.exe_lower.clone(), k);
                                                            changed = true;
                                                        }
                                                        RowAction::MarkGame => mark_as_game(&sh, a),
                                                        RowAction::None => {}
                                                    }
                                                    cols[1].add_space(ui::SPACE_GRID_GAP);
                                                }
                                            });
                                        } else {
                                            for a in apps_snapshot.iter() {
                                                let mut k = keep_decision(&keep_view, &a.exe_lower);
                                                let is_game = games_view.contains(&a.exe_lower);
                                                match running_app_row(ui, &mut k, a, is_game) {
                                                    RowAction::ToggleKeep => {
                                                        sh.keep.lock().unwrap().insert(a.exe_lower.clone(), k);
                                                        changed = true;
                                                    }
                                                    RowAction::MarkGame => mark_as_game(&sh, a),
                                                    RowAction::None => {}
                                                }
                                                ui.add_space(ui::SPACE_GRID_GAP);
                                            }
                                        }

                                        if changed {
                                            self.save();
                                        }
                                    }
                                });
                            });
                    }
                    ConfigTab::Services => {
                        // Thanh công cụ dịch vụ
                        ui::card(ui, |ui| {
                            ui.spacing_mut().item_spacing.x = ui::SPACE_TOOLBAR_GAP;
                            ui.horizontal_wrapped(|ui| {
                                ui.label(
                                    RichText::new(format!("Đã chọn tắt: {}/{} service", svc_sel, SERVICE_TARGETS.len()))
                                        .size(11.5)
                                        .color(ui::TEXT_MUTED),
                                );
                                ui.add_space(ui::SPACE_TOOLBAR_GAP);

                                if ui.add(ui::ghost_button_sized("Mặc định", 11.0)).clicked() {
                                    let mut svc = sh.svc.lock().unwrap();
                                    for (i, (_, _, _, def)) in SERVICE_TARGETS.iter().enumerate() {
                                        svc[i] = *def;
                                    }
                                    drop(svc);
                                    self.save();
                                }
                                if ui.add(ui::ghost_button_sized("Chọn hết", 11.0)).clicked() {
                                    for x in sh.svc.lock().unwrap().iter_mut() {
                                        *x = true;
                                    }
                                    self.save();
                                }
                                ui.add_space(ui::SPACE_TOOLBAR_GAP);

                                if self.is_admin {
                                    ui.label(RichText::new("✔ QUYỀN ADMIN").size(10.5).strong().color(ui::SUCCESS));
                                } else {
                                    let btn = egui::Button::new(
                                        RichText::new("🛡 Cấp Quyền Admin").size(10.5).strong().color(ui::WARNING),
                                    )
                                    .fill(ui::CARD_HOVER)
                                    .stroke(Stroke::new(1.0_f32, ui::WARNING))
                                    .rounding(Rounding::same(4.0));
                                    if ui.add(btn).clicked() {
                                        restart_as_admin();
                                    }
                                }
                            });
                        });

                        ui.add_space(ui::SPACE_TOOLBAR_MARGIN_BOTTOM);

                        egui::ScrollArea::vertical()
                            .id_salt("services_scroll_area")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui::card(ui, |ui| {
                                    let is_grid = ui.available_width() >= 560.0;
                                    let mut changed = false;
                                    let mut svc = sh.svc.lock().unwrap();

                                    if is_grid {
                                        ui.spacing_mut().item_spacing.x = ui::SPACE_GRID_GAP;
                                        let half = (SERVICE_TARGETS.len() + 1) / 2;
                                        ui.columns(2, |cols| {
                                            for (i, (svc_name, display_name, desc, _)) in SERVICE_TARGETS[..half].iter().enumerate() {
                                                let is_running = sys::is_service_running(svc_name);
                                                if ui::service_row(&mut cols[0], &mut svc[i], svc_name, display_name, desc, is_running) {
                                                    changed = true;
                                                }
                                                cols[0].add_space(ui::SPACE_GRID_GAP);
                                            }
                                            for (j, (svc_name, display_name, desc, _)) in SERVICE_TARGETS[half..].iter().enumerate() {
                                                let i = half + j;
                                                let is_running = sys::is_service_running(svc_name);
                                                if ui::service_row(&mut cols[1], &mut svc[i], svc_name, display_name, desc, is_running) {
                                                    changed = true;
                                                }
                                                cols[1].add_space(ui::SPACE_GRID_GAP);
                                            }
                                        });
                                    } else {
                                        for (i, (svc_name, display_name, desc, _)) in SERVICE_TARGETS.iter().enumerate() {
                                            let is_running = sys::is_service_running(svc_name);
                                            if ui::service_row(ui, &mut svc[i], svc_name, display_name, desc, is_running) {
                                                changed = true;
                                            }
                                            ui.add_space(ui::SPACE_GRID_GAP);
                                        }
                                    }

                                    if changed {
                                        self.save();
                                    }
                                });
                            });
                    }
                    ConfigTab::AutoOpt => {
                        let is_wide = ui.available_width() >= 560.0;
                        egui::ScrollArea::vertical()
                            .id_salt("auto_opt_scroll_area")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                let mut save_needed = false;
                                if is_wide {
                                    ui.spacing_mut().item_spacing.x = ui::SPACE_GRID_GAP;
                                    ui.columns(2, |cols| {
                                        render_auto_card(
                                            &mut cols[0],
                                            &sh,
                                            &mut opts_view,
                                            &mut self.task_on,
                                            &mut self.last_task_check,
                                            &games_view,
                                            &mut self.game_input,
                                            &mut save_needed,
                                        );
                                        render_optimize_card(
                                            &mut cols[1],
                                            &mut opts_view,
                                            &mut save_needed,
                                        );
                                    });
                                } else {
                                    render_auto_card(
                                        ui,
                                        &sh,
                                        &mut opts_view,
                                        &mut self.task_on,
                                        &mut self.last_task_check,
                                        &games_view,
                                        &mut self.game_input,
                                        &mut save_needed,
                                    );
                                    ui.add_space(ui::SPACE_CARD_GAP);
                                    render_optimize_card(
                                        ui,
                                        &mut opts_view,
                                        &mut save_needed,
                                    );
                                }

                                if save_needed {
                                    *sh.opts.lock().unwrap() = opts_view;
                                    self.save();
                                }
                            });
                    }
                    ConfigTab::Reports => {
                        self.render_reports_tab(ui);
                    }
                }
            });

        ui::handle_window_resize_and_border(ctx);
    }
}

fn main() -> eframe::Result<()> {
    // Bản release là panic=abort + không có console: ghi lý do panic ra file cạnh exe.
    std::panic::set_hook(Box::new(|info| {
        let msg = format!("[{}] {}
", sys::local_time_string(), info);
        let path = config_path().with_file_name("gamemode-crash.log");
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            use std::io::Write;
            let _ = f.write_all(msg.as_bytes());
        }
    }));

    // --launchtest: thử mở notepad bằng quyền user thường, ghi kết quả ra launchtest.txt.
    if std::env::args().any(|a| a == "--launchtest") {
        let r = sys::launch_as_standard_user(
            "notepad.exe",
            r"C:\Windows\System32\notepad.exe",
            None,
        );
        let msg = format!("admin={} result={:?}", sys::is_admin(), r);
        let _ = std::fs::write(config_path().with_file_name("launchtest.txt"), msg);
        return Ok(());
    }

    // --selftest: đọc mọi process như khi lưu session (cmdline, cwd), KHÔNG tắt gì.
    // Ghi thêm trạng thái standby/Wi-Fi/Game DVR/GPU/task để kiểm tra trên máy thật.
    if std::env::args().any(|a| a == "--selftest") {
        let cfg = load_config();
        let skip = skip_list(&cfg.games);
        let protect: HashSet<u32> = game_family_from_nodes(&sys::process_nodes(), &cfg.games)
            .into_keys()
            .collect();
        let snap = sys::snapshot_skip(&skip, &protect);
        let mut lines = Vec::new();
        let mut cwd_ok = 0;
        for p in snap.iter() {
            // Không đọc bộ nhớ của game / anti-cheat / họ process game kể cả khi chạy selftest.
            if is_ace_or_game(&p.name.to_lowercase(), &skip) || protect.contains(&p.pid) {
                lines.push(format!("{}\t{}\tskip(cmdline/cwd)\t{:?}", p.pid, p.name, p.exe_path));
                continue;
            }
            let cwd = sys::working_dir_of(p.pid);
            cwd_ok += cwd.is_some() as usize;
            lines.push(format!("{}	{}	{}	{:?}", p.pid, p.name, p.cmdline.is_some(), cwd));
        }
        let apps = scan_running_apps(&skip, &cfg.games);
        lines.insert(0, format!("processes={} cwd_ok={} apps_listed={}", snap.len(), cwd_ok, apps.len()));
        for a in apps.iter() {
            lines.push(format!("APP	{}	{}	{}", a.exe_name, a.count, fmt_ram(a.total_ws)));
        }

        lines.push(String::new());
        lines.push("=== GAME FAMILY ===".into());
        let reasons = game_family_reasons(&snap, &cfg.games);
        let mut fam: Vec<(FamilyReason, u32, String)> = snap
            .iter()
            .filter_map(|p| reasons.get(&p.pid).map(|r| (*r, p.pid, p.name.clone())))
            .collect();
        fam.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        if fam.is_empty() {
            lines.push("(không có process nào thuộc họ game)".into());
        }
        for (reason, pid, name) in fam.iter() {
            lines.push(format!("{}	{}	{}", pid, name, reason.label()));
        }
        lines.push(format!("protected_pids={}", fam.len()));

        lines.push(String::new());
        lines.push("=== MEMORY LISTS (standby) ===".into());
        match sys::memory_lists() {
            Some(l) => lines.push(format!(
                "free_bytes={} ({:.2} GB) standby_bytes={} ({:.2} GB)",
                l.free_bytes,
                l.free_bytes as f64 / GB,
                l.standby_bytes,
                l.standby_bytes as f64 / GB
            )),
            None => lines.push("memory_lists() = None (không gọi được NtQuerySystemInformation class 80)".into()),
        }

        lines.push(String::new());
        lines.push("=== WLAN INTERFACES ===".into());
        match sys::wlan_interfaces() {
            Ok(list) => {
                if list.is_empty() {
                    lines.push("(không có interface Wi-Fi nào)".into());
                }
                for it in list.iter() {
                    lines.push(format!(
                        "{}	state={}	connected={}	autoconf={:?}",
                        it.guid, it.state, it.connected, it.autoconf
                    ));
                }
            }
            Err(e) => lines.push(format!("lỗi: {}", e)),
        }

        lines.push(String::new());
        lines.push("=== GAME DVR ===".into());
        lines.push(format!("GameDVR_Enabled = {:?}", sys::gamedvr_enabled()));
        lines.push(format!("AppCaptureEnabled = {:?}", sys::appcapture_enabled()));

        lines.push(String::new());
        lines.push("=== USER GPU PREFERENCES ===".into());
        let prefs = sys::gpu_prefs();
        if prefs.is_empty() {
            lines.push("(chưa có value nào)".into());
        }
        for p in prefs.iter() {
            lines.push(format!("{}	=	{}", p.path, p.value));
        }

        lines.push(String::new());
        lines.push("=== TASK SCHEDULER ===".into());
        lines.push(format!("task \"GameMode\" tồn tại = {}", sys::task_exists()));

        lines.push(String::new());
        lines.push("=== CONFIG ĐANG ĐỌC ===".into());
        lines.push(format!(
            "auto:enabled={} auto:restore={} opt:standby={} opt:trim={} opt:gpu={} opt:wlan={} opt:gamedvr={}",
            cfg.opts.auto_enabled as u8,
            cfg.opts.auto_restore as u8,
            cfg.opts.standby as u8,
            cfg.opts.trim as u8,
            cfg.opts.gpu as u8,
            cfg.opts.wlan as u8,
            cfg.opts.gamedvr as u8
        ));
        lines.push(format!("game list = {:?}", cfg.games));
        lines.push(format!("game paths = {:?}", cfg.game_paths));
        lines.push(format!(
            "report:enabled={} report:ping_host={} gateway={:?} reports_dir={}",
            cfg.opts.report_enabled as u8,
            cfg.ping_host,
            sys::gateway_ip().map(sys::ip_to_string),
            report::reports_dir().display()
        ));

        let _ = std::fs::write(config_path().with_file_name("selftest.txt"), lines.join("
"));
        return Ok(());
    }

    // --pingtest: ping gateway và host internet 5 lần mỗi đích, ghi IP / RTT / mất gói ra pingtest.txt.
    if std::env::args().any(|a| a == "--pingtest") {
        let cfg = load_config();
        let mut lines = vec![format!("admin={}", sys::is_admin())];

        let gw = sys::gateway_ip();
        match gw {
            Some(addr) => {
                lines.push(format!("gateway = {}", sys::ip_to_string(addr)));
                lines.extend(ping_lines("gateway", addr, 5));
            }
            None => lines.push("gateway = KHÔNG lấy được route mặc định (GetBestRoute)".into()),
        }

        match sys::resolve_ipv4(&cfg.ping_host) {
            Some((addr, name)) => {
                lines.push(format!("internet = {} (host {})", sys::ip_to_string(addr), name));
                lines.extend(ping_lines("internet", addr, 5));
            }
            None => lines.push(format!("internet = KHÔNG phân giải được host {:?}", cfg.ping_host)),
        }
        let _ = std::fs::write(config_path().with_file_name("pingtest.txt"), lines.join("\n"));
        return Ok(());
    }

    // --reporttest: recorder giả lập 20 giây (mẫu mỗi 2 giây, không cần game, cho phép trận ngắn),
    // lưu báo cáo game="test" vào reports/ rồi ghi đường dẫn ra reporttest.txt.
    if std::env::args().any(|a| a == "--reporttest") {
        let cfg = load_config();
        let live = report::start("test", &cfg.ping_host, 2, 0);
        for _ in 0..10 {
            std::thread::sleep(Duration::from_secs(2));
        }
        let out = report::finish(live);
        let mut lines = vec![
            format!("admin={}", sys::is_admin()),
            format!("id={} samples={} saved={}", out.id, out.samples, out.saved),
            format!("thư mục reports = {}", report::reports_dir().display()),
            format!(
                "file = {}",
                out.path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "KHÔNG ghi được".to_string())
            ),
            format!("tóm tắt: {}", out.line),
        ];
        // Đọc lại file vừa ghi: đúng đường mà tab báo cáo (task sau) sẽ dùng.
        match report::load_report(&out.id) {
            Some(r) => {
                lines.push(format!(
                    "đọc lại: game={} duration={}s samples={} events={} kết thúc={}",
                    r.game,
                    r.summary.duration_sec,
                    r.samples.len(),
                    r.events.len(),
                    r.ended_at
                ));
                if let Some(s) = r.samples.first() {
                    lines.push(format!(
                        "mẫu đầu: t={:.1}s ram={:.2} GB ({:.0}%) commit={:.0}% cpu={:.0}% ping_gw={:?} ping_net={:?}",
                        s.t, s.avail_gb, s.ram_used_pct, s.commit_pct, s.cpu_pct, s.ping_gw_ms, s.ping_net_ms
                    ));
                }
                for v in r.summary.verdict.iter() {
                    lines.push(format!("nhận xét: {v}"));
                }
            }
            None => lines.push("đọc lại: LỖI (load_report trả None)".into()),
        }
        let ids: Vec<String> = report::list_reports()
            .into_iter()
            .take(5)
            .map(|m| m.id)
            .collect();
        lines.push(format!("list_reports (5 mới nhất) = {:?}", ids));
        let _ = std::fs::write(config_path().with_file_name("reporttest.txt"), lines.join("\n"));
        return Ok(());
    }

    // --purgetest: xả standby list một lần, ghi free/standby trước & sau ra purgetest.txt.
    // --tasktest: tạo task "GameMode", đọc lại XML, rồi xoá (chỉ để kiểm tra cách quote đường dẫn).
    if std::env::args().any(|a| a == "--tasktest") {
        let created = sys::task_create();
        use std::os::windows::process::CommandExt;
        let xml = std::process::Command::new("schtasks")
            .args(["/Query", "/TN", "GameMode", "/XML"])
            .creation_flags(0x0800_0000)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();
        let deleted = sys::task_delete();
        let _ = std::fs::write(
            config_path().with_file_name("tasktest.txt"),
            format!("create={:?}
delete={:?}
{}", created, deleted, xml),
        );
        return Ok(());
    }

    if std::env::args().any(|a| a == "--purgetest") {
        let before = sys::memory_lists();
        let result = sys::purge_standby();
        let after = sys::memory_lists();
        let fmt = |m: Option<sys::MemLists>| match m {
            Some(l) => format!(
                "free={} ({:.2} GB) standby={} ({:.2} GB)",
                l.free_bytes,
                l.free_bytes as f64 / GB,
                l.standby_bytes,
                l.standby_bytes as f64 / GB
            ),
            None => "không đọc được".to_string(),
        };
        let text = format!(
            "admin={}\ntrước: {}\nkết quả purge: {:?}\nsau: {}\n",
            sys::is_admin(),
            fmt(before),
            result,
            fmt(after)
        );
        let _ = std::fs::write(config_path().with_file_name("purgetest.txt"), text);
        return Ok(());
    }

    // --traytest: tạo icon khay + menu trong 5 giây rồi thoát, ghi kết quả ra traytest.txt.
    if std::env::args().any(|a| a == "--traytest") {
        let text = format!(
            "admin={}\n{}\n",
            sys::is_admin(),
            tray::test_tray(5000)
        );
        let _ = std::fs::write(config_path().with_file_name("traytest.txt"), text);
        return Ok(());
    }

    // Một bản duy nhất: bản chạy thứ hai chỉ hiện cửa sổ của bản cũ rồi thoát ngay
    // (không chặn các cờ kiểm tra ở trên). Giữ handle mutex tới hết hàm main.
    let _instance = match sys::acquire_single_instance("Local\\GameModeTacticalRamPurge") {
        Some(handle) => handle,
        None => {
            let old = sys::find_window(WINDOW_TITLE);
            if old != 0 {
                sys::show_main_window(old);
            }
            return Ok(());
        }
    };

    if std::env::args().any(|a| a == "--minimized") {
        START_HIDDEN.store(true, Ordering::SeqCst);
    }
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(WINDOW_TITLE)
        .with_inner_size([1240.0, 820.0])
        .with_min_inner_size([980.0, 640.0])
        .with_resizable(true)
        .with_decorations(false);

    if let Some(icon) = load_app_icon() {
        viewport = viewport.with_icon(icon);
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "DeltaForceRamOptimizer",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}
