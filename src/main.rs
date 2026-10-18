#![windows_subsystem = "windows"]

mod sys;
mod ui;

use eframe::egui::{self, Align2, Color32, CursorIcon, FontId, Margin, Rect, RichText, Rounding, Stroke, pos2, vec2};
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
}

impl Default for Opts {
    fn default() -> Self {
        Self {
            standby: true,
            gpu: true,
            wlan: true,
            gamedvr: true,
            auto_enabled: false,
            auto_restore: true,
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
    opts: Opts,
}

fn is_on(v: &str) -> bool {
    v.trim() == "1"
}

/// Đọc cấu hình: `keep:<exe>=1/0`, `svc:<tên>=1/0`, `game:<exe>=1`, `gamepath:<exe>=<path>`,
/// `auto:enabled/restore=1/0`, `opt:standby/gpu/wlan/gamedvr=1/0`.
/// Các dòng `app:...` của bản cũ được bỏ qua (không còn danh sách app cố định).
fn load_config() -> Config {
    let text = std::fs::read_to_string(config_path()).unwrap_or_default();
    let mut keep_saved: HashMap<String, bool> = HashMap::new();
    let mut svc_saved: HashMap<String, bool> = HashMap::new();
    let mut games: Vec<String> = Vec::new();
    let mut game_paths: HashMap<String, String> = HashMap::new();
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
    lines.push(format!("opt:gpu={}", if o.gpu { 1 } else { 0 }));
    lines.push(format!("opt:wlan={}", if o.wlan { 1 } else { 0 }));
    lines.push(format!("opt:gamedvr={}", if o.gamedvr { 1 } else { 0 }));
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
/// (game luôn được giữ lại nên không có gì để chọn).
fn scan_running_apps(skip: &[String]) -> Vec<RunningApp> {
    let me = std::process::id();
    let my_session = sys::current_session();
    let all = sys::snapshot_skip(skip);
    let parent = all.iter().find(|p| p.pid == me).map(|p| p.ppid).unwrap_or(0);

    let mut map: HashMap<String, RunningApp> = HashMap::new();
    for p in all.iter() {
        if p.pid == 0 || p.pid == me || p.pid == parent || p.session != my_session {
            continue;
        }
        let exe_lower = p.name.to_lowercase();
        if exe_lower.is_empty() || is_never_kill(&exe_lower, p.exe_path.as_deref()) {
            continue;
        }
        // Game + anti-cheat: luôn được giữ, không hiện trong danh sách để chọn.
        if is_ace_or_game(&exe_lower, skip) {
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
}

impl Shared {
    fn save_config(&self) -> bool {
        let cfg = Config {
            keep: self.keep.lock().unwrap().clone(),
            svcs: self.svc.lock().unwrap().clone(),
            games: self.games.lock().unwrap().clone(),
            game_paths: self.game_paths.lock().unwrap().clone(),
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
}

/// Bắt đầu tối ưu ở thread nền. Dùng chung cho nút trên UI và watcher.
fn optimize(sh: Shared, from_auto: bool) {
    if sh.busy.swap(true, Ordering::SeqCst) {
        return;
    }
    sh.last_auto.store(from_auto, Ordering::SeqCst);
    sh.opt_count.fetch_add(1, Ordering::SeqCst);
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
    let snap = sys::snapshot_skip(&skip);
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
        // Game + anti-cheat (ACE): tuyệt đối không tắt, không mở quyền đọc bộ nhớ.
        if keep_decision(&keep, &exe) || is_ace_or_game(&exe, &skip) {
            kept_exes.insert(exe);
            continue;
        }
        groups.entry(exe.clone()).or_default().push(p.pid);
        labels
            .entry(exe)
            .or_insert_with(|| p.display_name.clone().unwrap_or_else(|| p.name.clone()));
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

    // 9. Xả standby list một lần sau khi đã tắt app (thay cho trim working-set cũ).
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

    *sh.running.lock().unwrap() = scan_running_apps(&skip);
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

/// Watcher chạy suốt đời app ở thread riêng: tự tối ưu khi game mở, tự Restore khi game tắt,
/// và xả standby list khi RAM trống xuống thấp trong lúc chơi.
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

        let mut game_seen: Option<(Instant, usize)> = None;
        let mut game_gone: Option<Instant> = None;
        let mut optimized = false;
        let mut last_mem_check = Instant::now();
        let mut last_purge = Instant::now() - PURGE_GAP;

        loop {
            std::thread::sleep(SCAN);
            let opts = sh.opts();
            let games: Vec<String> = sh.games.lock().unwrap().clone();
            if games.is_empty() {
                continue;
            }
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

            if any {
                game_gone = None;
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
                let since = *game_gone.get_or_insert_with(Instant::now);
                if since.elapsed() >= GAME_GONE {
                    // Chỉ tự Restore khi phiên game này do auto khởi động, và không chạy lặp.
                    if opts.auto_restore
                        && optimized
                        && sh.last_auto.load(Ordering::SeqCst)
                        && !sh.busy.load(Ordering::SeqCst)
                    {
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

/// Cờ `--minimized`: egui 0.29 không có `ViewportBuilder::with_minimized`, phải gửi
/// `ViewportCommand::Minimized(true)` sau khi cửa sổ đã tạo (xem `App::update`).
static START_MINIMIZED: AtomicBool = AtomicBool::new(false);

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
    show_session: bool,
    /// Ô nhập exe game mới.
    game_input: String,
    /// Task "GameMode" trong Task Scheduler (cache, hỏi lại định kỳ).
    task_on: bool,
    last_task_check: Instant,
    /// Còn phải thu nhỏ cửa sổ lần đầu (cờ --minimized).
    minimize_pending: bool,
}

impl App {
    fn save(&mut self) {
        let ok = self.sh.save_config();
        self.saved_at = Some((Instant::now(), ok));
    }

    fn new(cc: &eframe::CreationContext) -> Self {
        let mut fonts = egui::FontDefinitions::default();
        if let Ok(bytes) = std::fs::read(r"C:\Windows\Fonts\segoeui.ttf") {
            fonts.font_data.insert("segoe".into(), egui::FontData::from_owned(bytes));
            fonts.families.entry(egui::FontFamily::Proportional).or_default().insert(0, "segoe".into());
        }
        if let Ok(bytes) = std::fs::read(r"C:\Windows\Fonts\consola.ttf") {
            fonts.font_data.insert("consolas".into(), egui::FontData::from_owned(bytes));
            fonts.families.entry(egui::FontFamily::Monospace).or_default().insert(0, "consolas".into());
        }
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
            show_session: false,
            game_input: String::new(),
            task_on: sys::task_exists(),
            last_task_check: Instant::now(),
            minimize_pending: START_MINIMIZED.load(Ordering::SeqCst),
        };
        app.save();
        app.start_scan();
        // Watcher sống độc lập với UI: vẫn tự tối ưu khi cửa sổ bị thu nhỏ.
        start_watcher(sh);
        app
    }

    /// Quét lại danh sách app đang chạy ở thread nền.
    fn start_scan(&mut self) {
        if self.scanning.swap(true, Ordering::SeqCst) {
            return;
        }
        let running = self.sh.running.clone();
        let scanning = self.scanning.clone();
        let skip = skip_list(&self.sh.games.lock().unwrap().clone());
        std::thread::spawn(move || {
            let list = scan_running_apps(&skip);
            *running.lock().unwrap() = list;
            scanning.store(false, Ordering::SeqCst);
        });
    }
}

fn render_stat_card(
    ui: &mut egui::Ui,
    title: &str,
    sub_text: &str,
    used_gb: f64,
    total_gb: f64,
    ratio: f32,
    color: Color32,
    extra: Option<&str>,
) {
    ui::card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(title).size(12.0).strong().color(ui::ACCENT));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(sub_text).size(11.0).color(ui::TEXT_MUTED));
            });
        });
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{:.1} GB", used_gb)).size(20.0).strong().color(ui::TEXT_MAIN));
            ui.label(RichText::new(format!("/ {:.1} GB", total_gb)).size(12.0).color(ui::TEXT_MUTED));
        });
        ui.add_space(6.0);

        let progress = egui::ProgressBar::new(ratio)
            .fill(color)
            .show_percentage()
            .animate(false);
        ui.add(progress);

        if let Some(text) = extra {
            ui.add_space(4.0);
            ui.label(
                RichText::new(text)
                    .size(10.5)
                    .color(ui::TEXT_MUTED),
            );
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

    let row_h = 42.0_f32;
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
    painter.rect_filled(rect, Rounding::same(6.0), bg);
    if *keep {
        let bar = Rect::from_min_size(rect.min + vec2(0.0, 6.0), vec2(3.0, row_h - 12.0));
        painter.rect_filled(bar, Rounding::same(1.5), ui::SUCCESS);
    }

    let switch_rect = Rect::from_center_size(
        pos2(rect.left() + 22.0, rect.center().y),
        vec2(28.0, 16.0),
    );
    ui::draw_switch(&painter, switch_rect, *keep, hovered);

    let title = app.display.clone().unwrap_or_else(|| app.exe_name.clone());
    let name_color = if *keep {
        ui::SUCCESS
    } else {
        ui::TEXT_MAIN
    };

    let x0 = rect.left() + 44.0;
    let y_top = rect.top() + 13.0;
    let name = painter.layout_no_wrap(title, FontId::proportional(13.0), name_color);
    let name_w = name.size().x;
    painter.galley(pos2(x0, y_top - name.size().y / 2.0), name, name_color);

    let tag = painter.layout_no_wrap(app.exe_name.clone(), FontId::monospace(10.5), ui::TEXT_DIM);
    painter.galley(pos2(x0 + name_w + 8.0, y_top - tag.size().y / 2.0), tag, ui::TEXT_DIM);

    let (badge, badge_color) = if *keep {
        ("GIỮ LẠI", ui::SUCCESS)
    } else {
        ("SẼ TẮT", ui::WARNING)
    };
    painter.text(
        pos2(rect.right() - 10.0, y_top),
        Align2::RIGHT_CENTER,
        badge,
        FontId::proportional(10.5),
        badge_color,
    );

    painter.text(
        pos2(x0, rect.top() + 30.0),
        Align2::LEFT_CENTER,
        format!("{} process — tổng {}", app.count, fmt_ram(app.total_ws)),
        FontId::proportional(11.0),
        ui::TEXT_MUTED,
    );

    match app.exe_path.as_ref() {
        Some(p) => {
            resp.on_hover_text(p.clone());
        }
        None => {
            resp.on_hover_text("Không đọc được đường dẫn exe của process này.");
        }
    }

    // Nút nhỏ "Là game" (hoặc nhãn ✓ nếu exe đã nằm trong danh sách game).
    let btn_rect = Rect::from_min_max(
        pos2(rect.right() - 80.0, rect.top() + 22.0),
        pos2(rect.right() - 8.0, rect.top() + 38.0),
    );
    if is_game {
        painter.text(
            btn_rect.center(),
            Align2::CENTER_CENTER,
            "✓ LÀ GAME",
            FontId::proportional(10.0),
            ui::SUCCESS,
        );
    } else {
        let btn = egui::Button::new(
            RichText::new("Là game")
                .size(10.0)
                .color(ui::ACCENT),
        )
        .fill(ui::CARD_BG)
        .stroke(Stroke::new(1.0_f32, ui::BORDER))
        .rounding(Rounding::same(4.0));
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

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.minimize_pending {
            self.minimize_pending = false;
            // Khởi động cùng Windows: thu nhỏ ngay, watcher vẫn chạy bình thường.
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
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
        ctx.request_repaint_after(Duration::from_millis(400));
        let sh = self.sh.clone();
        let busy = sh.busy.load(Ordering::SeqCst);
        let scanning = self.scanning.load(Ordering::SeqCst);
        let apps_snapshot: Vec<RunningApp> = sh.running.lock().unwrap().clone();
        let session_view: Option<SessionData> = sh.session.lock().unwrap().clone();
        let games_view: Vec<String> = sh.games.lock().unwrap().clone();
        let mut opts_view = sh.opts();
        let mut opts_changed = false;
        let ram_extra = self.mem_lists.map(|l| {
            format!(
                "Free {:.1} GB · Standby {:.1} GB",
                l.free_bytes as f64 / GB,
                l.standby_bytes as f64 / GB
            )
        });

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(ui::BG).inner_margin(Margin::same(10.0)))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        // 1. TOP HEADER BANNER
                        ui::card(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("⚡").size(24.0).color(ui::ACCENT));
                                ui.vertical(|ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            RichText::new("DELTA FORCE")
                                                .size(18.0)
                                                .strong()
                                                .color(ui::SUCCESS),
                                        );
                                        ui.label(
                                            RichText::new("RAM PURGE")
                                                .size(18.0)
                                                .strong()
                                                .color(ui::ACCENT),
                                        );
                                    });
                                    ui.label(
                                        RichText::new("TACTICAL MEMORY BOOSTER // PRO EDITION")
                                            .size(9.5)
                                            .strong()
                                            .color(ui::TEXT_MUTED),
                                    );
                                });

                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if busy {
                                        egui::Frame::none()
                                            .fill(ui::CARD_HOVER)
                                            .stroke(Stroke::new(1.0_f32, ui::WARNING))
                                            .rounding(Rounding::same(6.0))
                                            .inner_margin(Margin::symmetric(8.0, 4.0))
                                            .show(ui, |ui| {
                                                ui.label(RichText::new("● PURGING...").size(11.0).strong().color(ui::WARNING));
                                            });
                                    } else {
                                        egui::Frame::none()
                                            .fill(ui::CARD_HOVER)
                                            .stroke(Stroke::new(1.0_f32, ui::SUCCESS))
                                            .rounding(Rounding::same(6.0))
                                            .inner_margin(Margin::symmetric(8.0, 4.0))
                                            .show(ui, |ui| {
                                                ui.label(RichText::new("● SYSTEM READY").size(11.0).strong().color(ui::SUCCESS));
                                            });
                                    }
                                });
                            });
                        });

                        ui.add_space(10.0);

                        // 2. HUD METRICS (2 CỘT CÂN ĐỐI 50/50)
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
                                "BỘ NHỚ ẢO (PAGEFILE)",
                                if commit_ratio > 0.85 { "⚠ NGUY HIỂM" } else { "An toàn" },
                                m.commit_used as f64 / GB,
                                m.commit_limit as f64 / GB,
                                commit_ratio,
                                commit_color,
                                None,
                            );
                        });

                        ui.add_space(10.0);

                        // 3. ACTION CONTROL DECK
                        let boost_btn_text = if busy {
                            "⏳  ĐANG THANH LỌC BỘ NHỚ & DỊCH VỤ..."
                        } else {
                            "🚀  BẬT GAME BOOST (DỌN RAM & SERVICES)"
                        };

                        let boost_btn = ui::primary_button(boost_btn_text, busy);
                        if ui.add_sized([ui.available_width(), 44.0], boost_btn).clicked() && !busy {
                            optimize(sh.clone(), false);
                        }

                        ui.add_space(6.0);

                        ui.columns(2, |cols| {
                            let restore_btn = ui::ghost_button("↩  Khôi Phục App & Services");
                            if cols[0].add_sized([cols[0].available_width(), 34.0], restore_btn).clicked() && !busy {
                                restore(sh.clone());
                            }

                            let clear_btn = ui::ghost_button("🧹  Xoá Nhật Ký");
                            if cols[1].add_sized([cols[1].available_width(), 34.0], clear_btn).clicked() {
                                sh.log.lock().unwrap().clear();
                            }
                        });

                        ui.add_space(10.0);

                        // 3b. TRẠNG THÁI SESSION ĐÃ LƯU
                        ui::card(ui, |ui| {
                            ui.horizontal(|ui| {
                                match session_view.as_ref() {
                                    Some(s) => {
                                        ui.label(
                                            RichText::new(format!(
                                                "💾 Session đã lưu: {} app, {} service, {} Wi-Fi, DVR {} — lúc {}",
                                                s.apps.len(),
                                                s.services.len(),
                                                s.wlan_autoconf_off.len(),
                                                if s.gamedvr_prev.is_some() { "có" } else { "không" },
                                                time_part(&s.saved_at)
                                            ))
                                            .size(11.0)
                                            .color(ui::ACCENT),
                                        );
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            if ui.add(ui::ghost_button_sized(if self.show_session { "Ẩn" } else { "Xem danh sách" }, 10.5)).clicked() {
                                                self.show_session = !self.show_session;
                                            }
                                        });
                                    }
                                    None => {
                                        ui.label(
                                            RichText::new("💾 Chưa có session nào được lưu — bấm Game Boost để chụp phiên hiện tại.")
                                                .size(11.0)
                                                .color(ui::TEXT_MUTED),
                                        );
                                    }
                                }
                            });

                            if self.show_session {
                                if let Some(s) = session_view.as_ref() {
                                    ui.add_space(6.0);
                                    ui.label(
                                        RichText::new("Tick app nào để Restore mở lại app đó:")
                                            .size(10.5)
                                            .color(ui::TEXT_MUTED),
                                    );
                                    ui.add_space(4.0);
                                    egui::ScrollArea::vertical()
                                        .id_salt("session_list")
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
                                                        .size(10.5)
                                                        .color(ui::TEXT_MAIN),
                                                    );
                                                });
                                                ui.add_space(2.0);
                                            }
                                            for svc in s.services.iter() {
                                                ui.label(
                                                    RichText::new(format!("  • service: {}", svc))
                                                        .monospace()
                                                        .size(10.5)
                                                        .color(ui::SUCCESS),
                                                );
                                            }
                                        });
                                }
                            }
                        });

                        ui.add_space(10.0);

                        // 3c. TỰ ĐỘNG KHI CHƠI GAME (watcher + danh sách exe game)
                        ui::card(ui, |ui| {
                            ui::section_header(
                                ui,
                                "🤖",
                                "Tự động khi chơi game",
                                Some("Watcher quét tên process mỗi 3 giây (không mở handle vào game / ACE)"),
                            );
                            ui.add_space(6.0);

                            let mut changed = false;
                            if ui::toggle_row(
                                ui,
                                &mut opts_view.auto_enabled,
                                "Tự tối ưu khi game mở",
                                "Chờ 5 giây cho game ổn định rồi dọn RAM & service",
                            ) {
                                changed = true;
                            }
                            ui.add_space(2.0);
                            if ui::toggle_row(
                                ui,
                                &mut opts_view.auto_restore,
                                "Tự Restore khi game tắt",
                                "Game tắt liên tục 15 giây thì khôi phục (phiên do auto tối ưu)",
                            ) {
                                changed = true;
                            }
                            ui.add_space(2.0);

                            // Chạy cùng Windows bằng Task Scheduler
                            let mut task_on = self.task_on;
                            if ui::toggle_row(
                                ui,
                                &mut task_on,
                                "Chạy cùng Windows (Task Scheduler, quyền cao nhất)",
                                "Tạo task 'GameMode' chạy lúc đăng nhập với /RL HIGHEST — không hiện UAC",
                            ) {
                                let r = if task_on { sys::task_create() } else { sys::task_delete() };
                                match r {
                                    Ok(_) => {
                                        self.task_on = task_on;
                                        self.last_task_check = Instant::now();
                                        sh.log(
                                            if task_on {
                                                "  ✔ [TASK] Đã tạo task GameMode: tự chạy cùng Windows với quyền cao nhất.".into()
                                            } else {
                                                "  • [TASK] Đã xoá task GameMode.".into()
                                            },
                                            ui::SUCCESS,
                                        );
                                    }
                                    Err(e) => {
                                        self.last_task_check = Instant::now() - Duration::from_secs(20);
                                        sh.log(
                                            format!("  ⚠ [TASK] Không đổi được task GameMode: {}", e),
                                            ui::DANGER,
                                        );
                                    }
                                }
                            }

                            ui.add_space(8.0);
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
                                    .id_salt("game_list")
                                    .max_height(84.0)
                                    .auto_shrink([false, true])
                                    .show(ui, |ui| {
                                        for exe in games_view.iter() {
                                            let path = sh.game_paths.lock().unwrap().get(exe).cloned();
                                            let row_h = 24.0_f32;
                                            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), egui::Sense::hover());
                                            let painter = ui.painter_at(rect);

                                            let del_rect = Rect::from_center_size(
                                                pos2(rect.right() - 10.0, rect.center().y),
                                                vec2(18.0, 18.0),
                                            );
                                            let del_resp = ui.interact(del_rect, ui.id().with(exe), egui::Sense::click())
                                                .on_hover_cursor(CursorIcon::PointingHand)
                                                .on_hover_text("Xoá khỏi danh sách game");
                                            let del_color = if del_resp.hovered() { ui::DANGER } else { ui::TEXT_DIM };
                                            painter.text(
                                                del_rect.center(),
                                                Align2::CENTER_CENTER,
                                                "✕",
                                                FontId::proportional(12.0),
                                                del_color,
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
                                self.save();
                            }

                            ui.add_space(6.0);

                            ui.horizontal(|ui| {
                                let btn_w = 64.0_f32;
                                let spacing = 6.0_f32;
                                let edit_w = (ui.available_width() - btn_w - spacing).max(100.0_f32);

                                let edit = egui::TextEdit::singleline(&mut self.game_input)
                                    .hint_text("vd: game-client-win64-shipping.exe")
                                    .desired_width(edit_w)
                                    .min_size(vec2(edit_w, 30.0));
                                ui.add(edit);

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
                                    let exe = self.game_input.trim().to_lowercase();
                                    if !exe.is_empty() {
                                        let mut games = sh.games.lock().unwrap();
                                        if !games.contains(&exe) {
                                            games.push(exe.clone());
                                        }
                                        drop(games);
                                        self.game_input.clear();
                                        self.save();
                                        sh.log(
                                            format!("  • [GAME] Đã thêm {} vào danh sách game.", exe),
                                            ui::ACCENT,
                                        );
                                    }
                                }
                            });

                            ui.add_space(4.0);
                            ui.label(
                                RichText::new("(hoặc bấm 'Là game' ở tab ỨNG DỤNG ĐANG CHẠY)")
                                    .size(11.0)
                                    .color(ui::TEXT_MUTED),
                            );

                            if changed {
                                *sh.opts.lock().unwrap() = opts_view;
                                self.save();
                            }
                        });

                        ui.add_space(10.0);

                        // 3d. TỐI ƯU KHI CHƠI (4 tuỳ chọn)
                        ui::card(ui, |ui| {
                            ui::section_header(
                                ui,
                                "⚙",
                                "Tối ưu khi chơi",
                                None,
                            );
                            ui.add_space(6.0);

                            if ui::toggle_row(
                                ui,
                                &mut opts_view.standby,
                                "Xả standby list (RAM chờ) khi tối ưu",
                                "Không đẩy app ra pagefile; lúc chơi tự xả nếu Free < 1 GB & Standby > 1 GB",
                            ) {
                                opts_changed = true;
                            }
                            ui.add_space(2.0);

                            if ui::toggle_row(
                                ui,
                                &mut opts_view.gpu,
                                "Ép game dùng GPU rời (NVIDIA)",
                                "Ghi UserGpuPreferences = 2 trong DirectX registry cho game",
                            ) {
                                opts_changed = true;
                            }
                            ui.add_space(2.0);

                            if ui::toggle_row(
                                ui,
                                &mut opts_view.wlan,
                                "Tắt quét Wi-Fi nền khi chơi",
                                "Tắt autoconf của card Wi-Fi đang kết nối để giảm giật lag đột ngột",
                            ) {
                                opts_changed = true;
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
                                opts_changed = true;
                            }

                            if opts_changed {
                                *sh.opts.lock().unwrap() = opts_view;
                                self.save();
                            }
                        });

                        ui.add_space(10.0);

                        // 4. DANH SÁCH CẤU HÌNH: TABS CHO ỨNG DỤNG VÀ SERVICES
                        ui::card(ui, |ui| {
                            let kept = {
                                let keep = sh.keep.lock().unwrap();
                                apps_snapshot
                                    .iter()
                                    .filter(|a| keep_decision(&keep, &a.exe_lower))
                                    .count()
                            };
                            let svc_sel = sh.svc.lock().unwrap().iter().filter(|&&x| x).count();

                            // Tab Switcher
                            ui.horizontal(|ui| {
                                let is_app = self.tab == ConfigTab::Apps;
                                let app_tab_btn = ui::tab_button(
                                    &format!("💻 ỨNG DỤNG ĐANG CHẠY ({}/{})", kept, apps_snapshot.len()),
                                    is_app,
                                    ui::SUCCESS,
                                );
                                if ui.add(app_tab_btn).clicked() {
                                    self.tab = ConfigTab::Apps;
                                }

                                let is_svc = self.tab == ConfigTab::Services;
                                let svc_tab_btn = ui::tab_button(
                                    &format!("🛠 SERVICES HỆ THỐNG ({}/{})", svc_sel, SERVICE_TARGETS.len()),
                                    is_svc,
                                    ui::ACCENT,
                                );
                                if ui.add(svc_tab_btn).clicked() {
                                    self.tab = ConfigTab::Services;
                                }

                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if self.tab == ConfigTab::Apps {
                                        if ui.add_enabled(
                                            !scanning,
                                            ui::ghost_button_sized(if scanning { "Đang quét..." } else { "🔄 Làm mới" }, 10.5),
                                        ).clicked() {
                                            self.start_scan();
                                        }
                                        if ui.add(ui::ghost_button_sized("Giữ tất cả", 10.5)).clicked() {
                                            let mut keep = sh.keep.lock().unwrap();
                                            for a in apps_snapshot.iter() {
                                                keep.insert(a.exe_lower.clone(), true);
                                            }
                                            drop(keep);
                                            self.save();
                                        }
                                        if ui.add(ui::ghost_button_sized("Bỏ giữ tất cả", 10.5)).clicked() {
                                            let mut keep = sh.keep.lock().unwrap();
                                            for a in apps_snapshot.iter() {
                                                keep.insert(a.exe_lower.clone(), false);
                                            }
                                            drop(keep);
                                            self.save();
                                        }
                                        if ui.add(ui::ghost_button_sized("Mặc định", 10.5)).clicked() {
                                            let mut keep = sh.keep.lock().unwrap();
                                            for a in apps_snapshot.iter() {
                                                keep.insert(a.exe_lower.clone(), default_keep(&a.exe_lower));
                                            }
                                            drop(keep);
                                            self.save();
                                        }
                                    } else {
                                        if ui.add(ui::ghost_button_sized("Mặc định", 10.5)).clicked() {
                                            let mut svc = sh.svc.lock().unwrap();
                                            for (i, (_, _, _, def)) in SERVICE_TARGETS.iter().enumerate() {
                                                svc[i] = *def;
                                            }
                                            drop(svc);
                                            self.save();
                                        }
                                        if ui.add(ui::ghost_button_sized("Chọn hết", 10.5)).clicked() {
                                            for x in sh.svc.lock().unwrap().iter_mut() {
                                                *x = true;
                                            }
                                            self.save();
                                        }
                                    }
                                });
                            });

                            ui.add_space(4.0);

                            // Banner trạng thái Admin & Lưu cấu hình
                            ui.horizontal(|ui| {
                                let (status, color) = match self.saved_at {
                                    Some((t, true)) if t.elapsed() < Duration::from_millis(2500) => {
                                        ("✔ Đã lưu lựa chọn", ui::SUCCESS)
                                    }
                                    Some((t, false)) if t.elapsed() < Duration::from_millis(4000) => {
                                        ("⚠ Không lưu được cấu hình", ui::DANGER)
                                    }
                                    _ => ("💾 Tự động ghi nhớ cho lần sau", ui::TEXT_MUTED),
                                };
                                ui.label(RichText::new(status).size(10.5).color(color));

                                if self.tab == ConfigTab::Services {
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if self.is_admin {
                                            ui.label(RichText::new("✔ QUYỀN ADMIN").size(10.0).strong().color(ui::SUCCESS));
                                        } else {
                                            let btn = egui::Button::new(
                                                RichText::new("🛡 Cấp Quyền Admin").size(10.0).strong().color(ui::WARNING),
                                            )
                                            .fill(ui::CARD_HOVER)
                                            .stroke(Stroke::new(1.0_f32, ui::WARNING))
                                            .rounding(Rounding::same(4.0));
                                            if ui.add(btn).clicked() {
                                                restart_as_admin();
                                            }
                                        }
                                    });
                                }
                            });

                            ui.add_space(6.0);

                            // ScrollArea danh sách
                            egui::ScrollArea::vertical()
                                .id_salt(if self.tab == ConfigTab::Apps { "app_list" } else { "svc_list" })
                                .max_height(175.0)
                                .auto_shrink([false, false])
                                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                                .show(ui, |ui| {
                                    let mut changed = false;
                                    match self.tab {
                                        ConfigTab::Apps => {
                                            if apps_snapshot.is_empty() {
                                                ui.add_space(8.0);
                                                ui.label(
                                                    RichText::new(if scanning {
                                                        "Đang quét các app đang chạy..."
                                                    } else {
                                                        "Không thấy app nào đang chạy ngoài các process hệ thống được loại trừ."
                                                    })
                                                    .size(11.5)
                                                    .color(ui::TEXT_MUTED),
                                                );
                                                ui.add_space(8.0);
                                            }
                                            let keep_view = sh.keep.lock().unwrap().clone();
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
                                                ui.add_space(2.0);
                                            }
                                        }
                                        ConfigTab::Services => {
                                            let mut svc = sh.svc.lock().unwrap();
                                            for (i, (svc_name, display_name, desc, _)) in SERVICE_TARGETS.iter().enumerate() {
                                                let is_running = sys::is_service_running(svc_name);
                                                if ui::service_row(ui, &mut svc[i], svc_name, display_name, desc, is_running) {
                                                    changed = true;
                                                }
                                                ui.add_space(2.0);
                                            }
                                        }
                                    }
                                    if changed {
                                        self.save();
                                    }
                                });
                        });

                        ui.add_space(10.0);

                        // 5. TERMINAL LOG
                        ui::card(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new("TERMINAL LOG // NHẬT KÝ HỆ THỐNG")
                                        .size(11.5)
                                        .strong()
                                        .color(ui::ACCENT),
                                );
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.label(
                                        RichText::new("AUTO-SCROLL")
                                            .size(10.0)
                                            .color(ui::TEXT_DIM),
                                    );
                                });
                            });
                            ui.add_space(6.0);

                            let log_height = ui.available_height().max(100.0);
                            egui::ScrollArea::vertical()
                                .stick_to_bottom(true)
                                .max_height(log_height)
                                .show(ui, |ui| {
                                    for (line, color) in sh.log.lock().unwrap().iter() {
                                        ui.label(RichText::new(line).monospace().size(11.0).color(*color));
                                    }
                                });
                        });
                    });
            });
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
        let snap = sys::snapshot_skip(&skip);
        let mut lines = Vec::new();
        let mut cwd_ok = 0;
        for p in snap.iter() {
            // Không đọc bộ nhớ của game / anti-cheat kể cả khi chạy selftest.
            if is_ace_or_game(&p.name.to_lowercase(), &skip) {
                lines.push(format!("{}\t{}\tskip(cmdline/cwd)\t{:?}", p.pid, p.name, p.exe_path));
                continue;
            }
            let cwd = sys::working_dir_of(p.pid);
            cwd_ok += cwd.is_some() as usize;
            lines.push(format!("{}	{}	{}	{:?}", p.pid, p.name, p.cmdline.is_some(), cwd));
        }
        let apps = scan_running_apps(&skip);
        lines.insert(0, format!("processes={} cwd_ok={} apps_listed={}", snap.len(), cwd_ok, apps.len()));
        for a in apps.iter() {
            lines.push(format!("APP	{}	{}	{}", a.exe_name, a.count, fmt_ram(a.total_ws)));
        }

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
            "auto:enabled={} auto:restore={} opt:standby={} opt:gpu={} opt:wlan={} opt:gamedvr={}",
            cfg.opts.auto_enabled as u8,
            cfg.opts.auto_restore as u8,
            cfg.opts.standby as u8,
            cfg.opts.gpu as u8,
            cfg.opts.wlan as u8,
            cfg.opts.gamedvr as u8
        ));
        lines.push(format!("game list = {:?}", cfg.games));
        lines.push(format!("game paths = {:?}", cfg.game_paths));

        let _ = std::fs::write(config_path().with_file_name("selftest.txt"), lines.join("
"));
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

    let minimized = std::env::args().any(|a| a == "--minimized");
    if minimized {
        START_MINIMIZED.store(true, Ordering::SeqCst);
    }
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("DELTA FORCE - TACTICAL RAM PURGE")
        .with_inner_size([500.0, 760.0])
        .with_min_inner_size([460.0, 620.0])
        .with_resizable(true);

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
