#![windows_subsystem = "windows"]

mod sys;

use eframe::egui::{self, Color32, Margin, RichText, Rounding, Stroke};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
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
];
/// Tiền tố tên exe mặc định được giữ (Tailscale: tailscale.exe, tailscaled.exe...).
const DEFAULT_KEEP_PREFIX: &[&str] = &["tailscale", "deltaforce", "df_garena", "sguard", "unrealcef", "ace-", "gamebar"];
/// Chuỗi con trong tên exe mặc định được giữ (JBL Quantum).
const DEFAULT_KEEP_CONTAINS: &[&str] = &["quantum", "-win64-shipping", "无极画质"];

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

/// Đọc cấu hình: `keep:<exe>=1/0` cho app, `svc:<tên>=1/0` cho service.
/// Các dòng `app:...` của bản cũ được bỏ qua (không còn danh sách app cố định).
fn load_config() -> (HashMap<String, bool>, Vec<bool>) {
    let text = std::fs::read_to_string(config_path()).unwrap_or_default();
    let mut keep_saved: HashMap<String, bool> = HashMap::new();
    let mut svc_saved: HashMap<String, bool> = HashMap::new();

    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(rest) = line.strip_prefix("keep:") {
            if let Some((k, v)) = rest.rsplit_once('=') {
                keep_saved.insert(k.trim().to_lowercase(), v.trim() == "1");
            }
        } else if let Some(rest) = line.strip_prefix("svc:") {
            if let Some((k, v)) = rest.rsplit_once('=') {
                svc_saved.insert(k.trim().to_string(), v.trim() == "1");
            }
        }
        // dòng "app:..." / dòng rác của bản cũ: bỏ qua
    }

    let svcs = SERVICE_TARGETS
        .iter()
        .map(|(name, _, _, def)| svc_saved.get(*name).copied().unwrap_or(*def))
        .collect();

    (keep_saved, svcs)
}

fn save_config(keep: &HashMap<String, bool>, svcs: &[bool]) -> bool {
    let mut lines: Vec<String> = Vec::new();
    let mut keys: Vec<&String> = keep.keys().collect();
    keys.sort();
    for k in keys {
        lines.push(format!("keep:{}={}", k, if keep[k] { 1 } else { 0 }));
    }
    for ((name, _, _, _), on) in SERVICE_TARGETS.iter().zip(svcs) {
        lines.push(format!("svc:{}={}", name, if *on { 1 } else { 0 }));
    }
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

#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
struct SessionData {
    #[serde(default)]
    saved_at: String,
    #[serde(default)]
    apps: Vec<SessionApp>,
    #[serde(default)]
    services: Vec<String>,
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
fn scan_running_apps() -> Vec<RunningApp> {
    let me = std::process::id();
    let my_session = sys::current_session();
    let all = sys::snapshot();
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

struct App {
    /// Lựa chọn "Giữ lại" theo tên exe (viết thường) — lưu vào config.
    keep: HashMap<String, bool>,
    svc_enabled: Vec<bool>,
    tab: ConfigTab,
    log: Arc<Mutex<Vec<(String, Color32)>>>,
    busy: Arc<AtomicBool>,
    mem: sys::Mem,
    last_mem: Instant,
    saved_at: Option<(Instant, bool)>,
    is_admin: bool,
    /// App đang chạy thực tế (quét ở thread nền).
    running: Arc<Mutex<Vec<RunningApp>>>,
    scanning: Arc<AtomicBool>,
    /// Session đã lưu (đọc từ session.json) để hiển thị.
    session: Arc<Mutex<Option<SessionData>>>,
    show_session: bool,
    /// App nào của session được tick để Restore mở lại (mặc định bật, không lưu ra file).
    restore_sel: HashMap<String, bool>,
}

impl App {
    fn save(&mut self) {
        let ok = save_config(&self.keep, &self.svc_enabled);
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

        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = Color32::from_rgb(10, 14, 21);
        visuals.window_fill = Color32::from_rgb(10, 14, 21);
        visuals.faint_bg_color = Color32::from_rgb(15, 21, 32);
        visuals.extreme_bg_color = Color32::from_rgb(8, 11, 16);

        visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(15, 22, 33);
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(26, 38, 56));
        visuals.widgets.noninteractive.rounding = Rounding::same(6.0);

        visuals.widgets.inactive.bg_fill = Color32::from_rgb(19, 28, 42);
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(34, 48, 70));
        visuals.widgets.inactive.rounding = Rounding::same(6.0);

        visuals.widgets.hovered.bg_fill = Color32::from_rgb(28, 42, 65);
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0, 229, 255));
        visuals.widgets.hovered.rounding = Rounding::same(6.0);

        visuals.widgets.active.bg_fill = Color32::from_rgb(0, 200, 120);
        visuals.widgets.active.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0, 255, 163));
        visuals.widgets.active.rounding = Rounding::same(6.0);

        cc.egui_ctx.set_visuals(visuals);
        cc.egui_ctx.style_mut(|s| {
            s.spacing.scroll = egui::style::ScrollStyle::solid();
            s.spacing.scroll.bar_width = 8.0;
            s.spacing.scroll.bar_inner_margin = 4.0;
        });

        let (keep, svcs) = load_config();
        let is_admin = sys::is_admin();
        let session = SessionData::load();
        // Mặc định tick hết: người dùng bỏ tick app nào thì Restore không mở app đó.
        let restore_sel: HashMap<String, bool> = session
            .as_ref()
            .map(|s| s.apps.iter().map(|a| (app_key(a), true)).collect())
            .unwrap_or_default();

        let mut app = Self {
            keep,
            svc_enabled: svcs,
            tab: ConfigTab::Apps,
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
            mem: sys::memory(),
            last_mem: Instant::now(),
            saved_at: None,
            is_admin,
            running: Arc::new(Mutex::new(Vec::new())),
            scanning: Arc::new(AtomicBool::new(false)),
            session: Arc::new(Mutex::new(session)),
            show_session: false,
            restore_sel,
        };
        app.save();
        app.start_scan();
        app
    }

    /// Quét lại danh sách app đang chạy ở thread nền.
    fn start_scan(&mut self) {
        if self.scanning.swap(true, Ordering::SeqCst) {
            return;
        }
        let running = self.running.clone();
        let scanning = self.scanning.clone();
        std::thread::spawn(move || {
            let list = scan_running_apps();
            *running.lock().unwrap() = list;
            scanning.store(false, Ordering::SeqCst);
        });
    }

    fn run_optimize(&mut self) {
        let keep = self.keep.clone();
        let svcs = self.svc_enabled.clone();
        let log = self.log.clone();
        let busy = self.busy.clone();
        let is_admin = self.is_admin;
        let running_arc = self.running.clone();
        let session_arc = self.session.clone();
        busy.store(true, Ordering::SeqCst);

        std::thread::spawn(move || {
            let push = |s: String, c: Color32| log.lock().unwrap().push((s, c));
            log.lock().unwrap().clear();

            let before_mem = sys::memory();
            let before = before_mem.avail;
            push(
                format!("▶ [DEPLOY] Bắt đầu thanh lọc | RAM khả dụng ban đầu: {:.1} GB", before as f64 / GB),
                Color32::from_rgb(0, 229, 255),
            );

            // 1. Quét app đang chạy -> nhóm theo tên exe, bỏ nhóm được tick "Giữ lại".
            let me = std::process::id();
            let my_session = sys::current_session();
            let snap = sys::snapshot();
            let parent = snap.iter().find(|p| p.pid == me).map(|p| p.ppid).unwrap_or(0);

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
                if keep_decision(&keep, &exe) {
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
                        let startup = startup_lower
                            .get_or_insert_with(|| {
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

            // 3. Tạm dừng các Services được chọn (nếu có quyền Admin)
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

            // 4. Ghi session.json (gộp với session cũ nếu còn) trước khi tắt app.
            let mut data = SessionData::load().unwrap_or_default();
            data.saved_at = sys::local_time_string();
            let apps_before = data.apps.len();
            data.merge(session_apps, newly_stopped);
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
            *session_arc.lock().unwrap() = Some(data);

            // 5. Tắt app: WM_CLOSE trước (chờ tối đa 3s cho app kịp lưu), rồi TerminateProcess.
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

            // 6. Ép Working-set nhả RAM
            let mut trimmed = 0u64;
            for p in sys::list_processes().iter().filter(|p| p.pid > 4 && p.pid != me) {
                trimmed += sys::trim(p.pid);
            }
            push(
                format!("  〰 [TRIM WORKING-SET] Đã thu hồi cache toàn hệ thống -> Giảm {}", fmt_ram(trimmed)),
                Color32::from_rgb(50, 215, 255),
            );

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

            // 7. Danh sách app đã thay đổi -> quét lại cho UI.
            *running_arc.lock().unwrap() = scan_running_apps();
            busy.store(false, Ordering::SeqCst);
        });
    }

    fn run_restore(&self) {
        let log = self.log.clone();
        let session_arc = self.session.clone();
        let restore_sel = self.restore_sel.clone();
        let busy = self.busy.clone();
        busy.store(true, Ordering::SeqCst);
        log.lock().unwrap().clear();
        // Chạy nền: mở app / bật service có thể chậm, không được làm đứng giao diện.
        std::thread::spawn(move || {
            (|| {
                log.lock().unwrap().push(("▶ [RESTORE] Đang khôi phục lại các ứng dụng & dịch vụ hệ thống...".into(), Color32::from_rgb(0, 229, 255)));

                let Some(data) = SessionData::load() else {
                    log.lock().unwrap().push((
                        format!("  ⚠ Không tìm thấy {} — chưa có phiên nào được lưu.", session_path().display()),
                        Color32::from_rgb(255, 140, 140),
                    ));
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
                        log.lock().unwrap().push((
                            format!("  • Bỏ qua process phụ: {} ({})", app.display, app.exe),
                            Color32::from_rgb(130, 150, 170),
                        ));
                        continue;
                    }
                    if !restore_sel.get(&key).copied().unwrap_or(true) {
                        skipped += 1;
                        log.lock().unwrap().push((
                            format!("  • Bỏ qua (không được tick): {}", app.display),
                            Color32::from_rgb(130, 150, 170),
                        ));
                        continue;
                    }
                    if running.contains(&app.exe.to_lowercase()) {
                        log.lock().unwrap().push((format!("  • {} đã đang chạy sẵn.", app.display), Color32::from_rgb(130, 150, 170)));
                        continue;
                    }
                    if path.trim().is_empty() {
                        failed += 1;
                        log.lock().unwrap().push((format!("  ⚠ {} — không có đường dẫn exe để mở.", app.display), Color32::from_rgb(255, 120, 120)));
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
                            log.lock().unwrap().push((format!("  ✔ Đã mở lại {} ({})", app.display, app.exe), Color32::from_rgb(0, 255, 163)));
                        }
                        Err(e) => {
                            failed += 1;
                            log.lock().unwrap().push((
                                format!("  ⚠ Không mở được {} ({}): {}", app.display, app.exe, e),
                                Color32::from_rgb(255, 120, 120),
                            ));
                        }
                    }
                }
                if skipped > 0 {
                    log.lock().unwrap().push((
                        format!("  • Đã bỏ qua {} entry (process phụ / không được tick).", skipped),
                        Color32::from_rgb(130, 150, 170),
                    ));
                }

                // 2. Bật lại các Services đã dừng trong phiên.
                for svc_name in data.services.iter() {
                    match sys::start_service(svc_name) {
                        Ok(_) => log.lock().unwrap().push((format!("  ✔ Khởi động lại service: {}", svc_name), Color32::from_rgb(0, 255, 163))),
                        // 1056 = ERROR_SERVICE_ALREADY_RUNNING: service đã tự chạy lại, không phải lỗi.
                        Err(sys::SERVICE_TIMEOUT) => log.lock().unwrap().push((format!("  • Service {} khởi động chậm, đang chạy ngầm.", svc_name), Color32::from_rgb(255, 160, 40))),
                        Err(1056) => log.lock().unwrap().push((format!("  • Service {} đã đang chạy sẵn.", svc_name), Color32::from_rgb(130, 150, 170))),
                        Err(e) => log.lock().unwrap().push((format!("  ⚠ Lỗi khởi động service {}: mã lỗi {}", svc_name, e), Color32::from_rgb(255, 120, 120))),
                    }
                }

                // 3. Đã mở xong -> session.json chuyển thành session.last.json.
                let sp = session_path();
                let archived = sp.with_file_name("session.last.json");
                if std::fs::rename(&sp, &archived).is_err() {
                    let _ = std::fs::remove_file(&sp);
                }
                *session_arc.lock().unwrap() = None;

                log.lock().unwrap().push((
                    format!(
                        "✔ Đã gửi lệnh khôi phục xong: {} app mở được, {} lỗi, {} bỏ qua, {} service.",
                        opened,
                        failed,
                        skipped,
                        data.services.len()
                    ),
                    Color32::from_rgb(140, 165, 190),
                ));
            })();
            busy.store(false, Ordering::SeqCst);
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
) {
    egui::Frame::none()
        .fill(Color32::from_rgb(14, 20, 30))
        .stroke(Stroke::new(1.0, Color32::from_rgb(28, 42, 62)))
        .rounding(Rounding::same(8.0))
        .inner_margin(Margin::same(10.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(title).size(11.0).strong().color(Color32::from_rgb(0, 229, 255)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(sub_text).size(11.0).color(Color32::from_rgb(130, 155, 180)));
                });
            });
            ui.add_space(3.0);

            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{:.1} GB", used_gb)).size(20.0).strong().color(Color32::WHITE));
                ui.label(RichText::new(format!("/ {:.1} GB", total_gb)).size(12.0).color(Color32::from_rgb(150, 175, 200)));
            });
            ui.add_space(4.0);

            let progress = egui::ProgressBar::new(ratio)
                .fill(color)
                .show_percentage()
                .animate(false);
            ui.add(progress);
        });
}

/// Một dòng app đang chạy: tick "Giữ lại" = không tắt app này.
fn running_app_row(ui: &mut egui::Ui, keep: &mut bool, app: &RunningApp) -> bool {
    use egui::{pos2, vec2, Align2, CursorIcon, FontId, Rect, Sense, Shape};

    let row_h = 42.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
    let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    let clicked = resp.clicked();
    let hovered = resp.hovered();
    if clicked {
        *keep = !*keep;
    }

    let painter = ui.painter_at(rect);

    let bg = if hovered {
        Color32::from_rgb(22, 34, 52)
    } else if *keep {
        Color32::from_rgb(15, 24, 36)
    } else {
        Color32::TRANSPARENT
    };
    painter.rect_filled(rect, Rounding::same(6.0), bg);
    if *keep {
        let bar = Rect::from_min_size(rect.min + vec2(0.0, 6.0), vec2(3.0, row_h - 12.0));
        painter.rect_filled(bar, Rounding::same(2.0), Color32::from_rgb(0, 255, 163));
    }

    let box_rect = Rect::from_center_size(pos2(rect.left() + 20.0, rect.center().y), vec2(18.0, 18.0));
    let (fill, stroke) = if *keep {
        (Color32::from_rgb(0, 255, 163), Color32::from_rgb(0, 255, 163))
    } else if hovered {
        (Color32::from_rgb(20, 30, 45), Color32::from_rgb(0, 229, 255))
    } else {
        (Color32::from_rgb(18, 26, 38), Color32::from_rgb(50, 70, 100))
    };
    painter.rect(box_rect, Rounding::same(4.0), fill, Stroke::new(1.5, stroke));
    if *keep {
        let c = box_rect.center();
        painter.add(Shape::line(
            vec![c + vec2(-5.0, 0.0), c + vec2(-1.5, 3.5), c + vec2(5.0, -4.0)],
            Stroke::new(2.4, Color32::from_rgb(8, 12, 18)),
        ));
    }

    let title = app.display.clone().unwrap_or_else(|| app.exe_name.clone());
    let name_color = if *keep {
        Color32::from_rgb(90, 210, 255)
    } else {
        Color32::WHITE
    };

    let x0 = rect.left() + 40.0;
    let y_top = rect.top() + 13.0;
    let name = painter.layout_no_wrap(title, FontId::proportional(13.0), name_color);
    let name_w = name.size().x;
    painter.galley(pos2(x0, y_top - name.size().y / 2.0), name, name_color);

    let tag = painter.layout_no_wrap(app.exe_name.clone(), FontId::proportional(10.5), Color32::from_rgb(130, 150, 175));
    painter.galley(pos2(x0 + name_w + 8.0, y_top - tag.size().y / 2.0), tag, Color32::WHITE);

    let (badge, badge_color) = if *keep {
        ("GIỮ LẠI", Color32::from_rgb(0, 255, 163))
    } else {
        ("SẼ TẮT", Color32::from_rgb(255, 150, 50))
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
        Color32::from_rgb(115, 135, 155),
    );

    match app.exe_path.as_ref() {
        Some(p) => {
            resp.on_hover_text(p.clone());
        }
        None => {
            resp.on_hover_text("Không đọc được đường dẫn exe của process này.");
        }
    }

    clicked
}

fn service_row(ui: &mut egui::Ui, on: &mut bool, svc_name: &str, display_name: &str, desc: &str) -> bool {
    use egui::{pos2, vec2, Align2, CursorIcon, FontId, Rect, Sense, Shape};

    let row_h = 40.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
    let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    let clicked = resp.clicked();
    if clicked {
        *on = !*on;
    }

    let painter = ui.painter_at(rect);
    let hovered = resp.hovered();

    let bg = if hovered {
        Color32::from_rgb(22, 34, 52)
    } else if *on {
        Color32::from_rgb(15, 24, 36)
    } else {
        Color32::TRANSPARENT
    };
    painter.rect_filled(rect, Rounding::same(6.0), bg);
    if *on {
        let bar = Rect::from_min_size(rect.min + vec2(0.0, 6.0), vec2(3.0, row_h - 12.0));
        painter.rect_filled(bar, Rounding::same(2.0), Color32::from_rgb(0, 229, 255));
    }

    let box_rect = Rect::from_center_size(pos2(rect.left() + 20.0, rect.center().y), vec2(18.0, 18.0));
    let (fill, stroke) = if *on {
        (Color32::from_rgb(0, 229, 255), Color32::from_rgb(0, 229, 255))
    } else if hovered {
        (Color32::from_rgb(20, 30, 45), Color32::from_rgb(0, 229, 255))
    } else {
        (Color32::from_rgb(18, 26, 38), Color32::from_rgb(50, 70, 100))
    };
    painter.rect(box_rect, Rounding::same(4.0), fill, Stroke::new(1.5, stroke));
    if *on {
        let c = box_rect.center();
        painter.add(Shape::line(
            vec![c + vec2(-5.0, 0.0), c + vec2(-1.5, 3.5), c + vec2(5.0, -4.0)],
            Stroke::new(2.4, Color32::from_rgb(8, 12, 18)),
        ));
    }

    let is_running = sys::is_service_running(svc_name);
    let name_color = if *on { Color32::WHITE } else { Color32::from_rgb(125, 135, 148) };

    let x0 = rect.left() + 40.0;
    let y_top = rect.top() + 12.0;
    let name = painter.layout_no_wrap(display_name.to_string(), FontId::proportional(13.5), name_color);
    let name_w = name.size().x;
    painter.galley(pos2(x0, y_top - name.size().y / 2.0), name, name_color);

    let tag = painter.layout_no_wrap(format!("({})", svc_name), FontId::proportional(10.5), Color32::from_rgb(130, 150, 175));
    let tag_w = tag.size().x;
    painter.galley(pos2(x0 + name_w + 6.0, y_top - tag.size().y / 2.0), tag, Color32::WHITE);

    let (badge_txt, badge_color) = if is_running {
        ("ĐANG CHẠY", Color32::from_rgb(0, 255, 163))
    } else {
        ("ĐÃ DỪNG", Color32::from_rgb(120, 130, 140))
    };
    painter.text(
        pos2(x0 + name_w + tag_w + 12.0, y_top),
        Align2::LEFT_CENTER,
        badge_txt,
        FontId::proportional(10.0),
        badge_color,
    );

    painter.text(
        pos2(x0, rect.top() + 29.0),
        Align2::LEFT_CENTER,
        desc,
        FontId::proportional(11.0),
        Color32::from_rgb(115, 135, 155),
    );

    clicked
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
        if self.last_mem.elapsed() > Duration::from_millis(800) {
            self.mem = sys::memory();
            self.last_mem = Instant::now();
        }
        ctx.request_repaint_after(Duration::from_millis(400));
        let busy = self.busy.load(Ordering::SeqCst);
        let scanning = self.scanning.load(Ordering::SeqCst);
        let apps_snapshot: Vec<RunningApp> = self.running.lock().unwrap().clone();
        let session_view: Option<SessionData> = self.session.lock().unwrap().clone();

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(Color32::from_rgb(10, 14, 21)).inner_margin(Margin::same(14.0)))
            .show(ctx, |ui| {
                // 1. TOP HEADER BANNER
                egui::Frame::none()
                    .fill(Color32::from_rgb(15, 22, 33))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(32, 48, 70)))
                    .rounding(Rounding::same(8.0))
                    .inner_margin(Margin::symmetric(14.0, 10.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("⚡").size(24.0).color(Color32::from_rgb(0, 255, 163)));
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new("DELTA FORCE")
                                            .size(19.0)
                                            .strong()
                                            .color(Color32::from_rgb(0, 255, 163)),
                                    );
                                    ui.label(
                                        RichText::new("RAM PURGE")
                                            .size(19.0)
                                            .strong()
                                            .color(Color32::from_rgb(0, 229, 255)),
                                    );
                                });
                                ui.label(
                                    RichText::new("TACTICAL MEMORY BOOSTER // PRO EDITION")
                                        .size(9.5)
                                        .strong()
                                        .color(Color32::from_rgb(120, 145, 175)),
                                );
                            });

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if busy {
                                    egui::Frame::none()
                                        .fill(Color32::from_rgb(45, 28, 12))
                                        .stroke(Stroke::new(1.0, Color32::from_rgb(255, 145, 45)))
                                        .rounding(Rounding::same(6.0))
                                        .inner_margin(Margin::symmetric(8.0, 4.0))
                                        .show(ui, |ui| {
                                            ui.label(RichText::new("● PURGING...").size(11.0).strong().color(Color32::from_rgb(255, 165, 60)));
                                        });
                                } else {
                                    egui::Frame::none()
                                        .fill(Color32::from_rgb(12, 38, 26))
                                        .stroke(Stroke::new(1.0, Color32::from_rgb(0, 255, 163)))
                                        .rounding(Rounding::same(6.0))
                                        .inner_margin(Margin::symmetric(8.0, 4.0))
                                        .show(ui, |ui| {
                                            ui.label(RichText::new("● SYSTEM READY").size(11.0).strong().color(Color32::from_rgb(0, 255, 163)));
                                        });
                                }
                            });
                        });
                    });

                ui.add_space(8.0);

                // 2. HUD METRICS (2 CỘT CÂN ĐỐI 50/50)
                let m = self.mem;
                let ram_used = m.total - m.avail;
                let ram_ratio = if m.total > 0 { ram_used as f32 / m.total as f32 } else { 0.0 };
                let ram_color = if ram_ratio > 0.85 {
                    Color32::from_rgb(255, 60, 60)
                } else if ram_ratio > 0.70 {
                    Color32::from_rgb(255, 160, 40)
                } else {
                    Color32::from_rgb(0, 255, 163)
                };

                let commit_ratio = if m.commit_limit > 0 {
                    m.commit_used as f32 / m.commit_limit as f32
                } else {
                    0.0
                };
                let commit_color = if commit_ratio > 0.85 {
                    Color32::from_rgb(255, 50, 70)
                } else if commit_ratio > 0.70 {
                    Color32::from_rgb(255, 170, 30)
                } else {
                    Color32::from_rgb(0, 210, 255)
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
                    );
                    render_stat_card(
                        &mut cols[1],
                        "BỘ NHỚ ẢO (PAGEFILE)",
                        if commit_ratio > 0.85 { "⚠ NGUY HIỂM" } else { "An toàn" },
                        m.commit_used as f64 / GB,
                        m.commit_limit as f64 / GB,
                        commit_ratio,
                        commit_color,
                    );
                });

                ui.add_space(8.0);

                // 3. ACTION CONTROL DECK
                let boost_btn_text = if busy {
                    RichText::new("⏳  ĐANG THANH LỌC BỘ NHỚ & DỊCH VỤ...")
                        .size(15.0)
                        .strong()
                        .color(Color32::WHITE)
                } else {
                    RichText::new("🚀  BẬT GAME BOOST (DỌN RAM & SERVICES)")
                        .size(15.5)
                        .strong()
                        .color(Color32::BLACK)
                };

                let boost_btn = egui::Button::new(boost_btn_text)
                    .fill(if busy {
                        Color32::from_rgb(40, 60, 80)
                    } else {
                        Color32::from_rgb(0, 255, 163)
                    })
                    .rounding(Rounding::same(8.0));

                if ui.add_sized([ui.available_width(), 44.0], boost_btn).clicked() && !busy {
                    self.run_optimize();
                }

                ui.add_space(6.0);

                ui.columns(2, |cols| {
                    let restore_btn = egui::Button::new(
                        RichText::new("↩  Khôi Phục App & Services")
                            .size(13.0)
                            .color(Color32::from_rgb(180, 215, 245)),
                    )
                    .fill(Color32::from_rgb(18, 26, 38))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(42, 62, 90)))
                    .rounding(Rounding::same(6.0));

                    if cols[0].add_sized([cols[0].available_width(), 32.0], restore_btn).clicked() && !busy {
                        self.run_restore();
                    }

                    let clear_btn = egui::Button::new(
                        RichText::new("🧹  Xoá Nhật Ký")
                            .size(13.0)
                            .color(Color32::from_rgb(140, 165, 190)),
                    )
                    .fill(Color32::from_rgb(15, 22, 32))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(32, 46, 68)))
                    .rounding(Rounding::same(6.0));

                    if cols[1].add_sized([cols[1].available_width(), 32.0], clear_btn).clicked() {
                        self.log.lock().unwrap().clear();
                    }
                });

                ui.add_space(6.0);

                // 3b. TRẠNG THÁI SESSION ĐÃ LƯU
                egui::Frame::none()
                    .fill(Color32::from_rgb(12, 17, 25))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(26, 38, 56)))
                    .rounding(Rounding::same(6.0))
                    .inner_margin(Margin::symmetric(10.0, 6.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            match session_view.as_ref() {
                                Some(s) => {
                                    ui.label(
                                        RichText::new(format!(
                                            "💾 Session đã lưu: {} app, {} service — lúc {}",
                                            s.apps.len(),
                                            s.services.len(),
                                            time_part(&s.saved_at)
                                        ))
                                        .size(10.5)
                                        .color(Color32::from_rgb(0, 229, 255)),
                                    );
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if ui.button(RichText::new(if self.show_session { "Ẩn" } else { "Xem danh sách" }).size(10.5)).clicked() {
                                            self.show_session = !self.show_session;
                                        }
                                    });
                                }
                                None => {
                                    ui.label(
                                        RichText::new("💾 Chưa có session nào được lưu — bấm Game Boost để chụp phiên hiện tại.")
                                            .size(10.5)
                                            .color(Color32::from_rgb(95, 125, 155)),
                                    );
                                }
                            }
                        });

                        if self.show_session {
                            if let Some(s) = session_view.as_ref() {
                                ui.add_space(3.0);
                                ui.label(
                                    RichText::new("Tick app nào để Restore mở lại app đó:")
                                        .size(9.5)
                                        .color(Color32::from_rgb(95, 125, 155)),
                                );
                                egui::ScrollArea::vertical()
                                    .id_salt("session_list")
                                    .max_height(120.0)
                                    .auto_shrink([false, false])
                                    .show(ui, |ui| {
                                        for a in s.apps.iter() {
                                            let cwd = a.cwd.clone().unwrap_or_else(|| "(không rõ)".into());
                                            let checked = self.restore_sel.entry(app_key(a)).or_insert(true);
                                            ui.horizontal(|ui| {
                                                ui.checkbox(checked, "");
                                                ui.label(
                                                    RichText::new(format!(
                                                        "{} [{}] — cwd: {}",
                                                        a.display, a.exe, cwd
                                                    ))
                                                    .monospace()
                                                    .size(10.0)
                                                    .color(Color32::from_rgb(150, 185, 215)),
                                                );
                                            });
                                        }
                                        for svc in s.services.iter() {
                                            ui.label(
                                                RichText::new(format!("  • service: {}", svc))
                                                    .monospace()
                                                    .size(10.0)
                                                    .color(Color32::from_rgb(120, 200, 170)),
                                            );
                                        }
                                    });
                            }
                        }
                    });

                ui.add_space(8.0);

                // 4. DANH SÁCH CẤU HÌNH: TABS CHO ỨNG DỤNG VÀ SERVICES
                egui::Frame::none()
                    .fill(Color32::from_rgb(13, 18, 27))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(26, 38, 56)))
                    .rounding(Rounding::same(8.0))
                    .inner_margin(Margin::same(10.0))
                    .show(ui, |ui| {
                        let kept = apps_snapshot
                            .iter()
                            .filter(|a| keep_decision(&self.keep, &a.exe_lower))
                            .count();
                        let svc_sel = self.svc_enabled.iter().filter(|&&x| x).count();

                        // Tab Switcher
                        ui.horizontal(|ui| {
                            let is_app = self.tab == ConfigTab::Apps;
                            let app_tab_btn = egui::Button::new(
                                RichText::new(format!("💻 ỨNG DỤNG ĐANG CHẠY ({}/{})", kept, apps_snapshot.len()))
                                    .size(11.5)
                                    .strong()
                                    .color(if is_app { Color32::from_rgb(0, 255, 163) } else { Color32::GRAY }),
                            )
                            .fill(if is_app { Color32::from_rgb(18, 30, 45) } else { Color32::from_rgb(11, 15, 22) })
                            .stroke(Stroke::new(1.0, if is_app { Color32::from_rgb(0, 255, 163) } else { Color32::from_rgb(28, 40, 58) }))
                            .rounding(Rounding::same(6.0));

                            if ui.add(app_tab_btn).clicked() {
                                self.tab = ConfigTab::Apps;
                            }

                            let is_svc = self.tab == ConfigTab::Services;
                            let svc_tab_btn = egui::Button::new(
                                RichText::new(format!("🛠 SERVICES HỆ THỐNG ({}/{})", svc_sel, SERVICE_TARGETS.len()))
                                    .size(11.5)
                                    .strong()
                                    .color(if is_svc { Color32::from_rgb(0, 229, 255) } else { Color32::GRAY }),
                            )
                            .fill(if is_svc { Color32::from_rgb(18, 30, 45) } else { Color32::from_rgb(11, 15, 22) })
                            .stroke(Stroke::new(1.0, if is_svc { Color32::from_rgb(0, 229, 255) } else { Color32::from_rgb(28, 40, 58) }))
                            .rounding(Rounding::same(6.0));

                            if ui.add(svc_tab_btn).clicked() {
                                self.tab = ConfigTab::Services;
                            }

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if self.tab == ConfigTab::Apps {
                                    if ui.add_enabled(!scanning, egui::Button::new(RichText::new(if scanning { "Đang quét..." } else { "🔄 Làm mới" }).size(10.5))).clicked() {
                                        self.start_scan();
                                    }
                                    if ui.button(RichText::new("Giữ tất cả").size(10.5)).clicked() {
                                        for a in apps_snapshot.iter() {
                                            self.keep.insert(a.exe_lower.clone(), true);
                                        }
                                        self.save();
                                    }
                                    if ui.button(RichText::new("Bỏ giữ tất cả").size(10.5)).clicked() {
                                        for a in apps_snapshot.iter() {
                                            self.keep.insert(a.exe_lower.clone(), false);
                                        }
                                        self.save();
                                    }
                                    if ui.button(RichText::new("Mặc định").size(10.5)).clicked() {
                                        for a in apps_snapshot.iter() {
                                            self.keep.insert(a.exe_lower.clone(), default_keep(&a.exe_lower));
                                        }
                                        self.save();
                                    }
                                } else {
                                    if ui.button(RichText::new("Mặc định").size(10.5)).clicked() {
                                        for (i, (_, _, _, def)) in SERVICE_TARGETS.iter().enumerate() {
                                            self.svc_enabled[i] = *def;
                                        }
                                        self.save();
                                    }
                                    if ui.button(RichText::new("Chọn hết").size(10.5)).clicked() {
                                        for x in self.svc_enabled.iter_mut() {
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
                                    ("✔ Đã lưu lựa chọn", Color32::from_rgb(0, 255, 163))
                                }
                                Some((t, false)) if t.elapsed() < Duration::from_millis(4000) => {
                                    ("⚠ Không lưu được cấu hình", Color32::from_rgb(255, 100, 100))
                                }
                                _ => ("💾 Tự động ghi nhớ cho lần sau", Color32::from_rgb(95, 125, 155)),
                            };
                            ui.label(RichText::new(status).size(10.5).color(color));

                            if self.tab == ConfigTab::Services {
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if self.is_admin {
                                        ui.label(RichText::new("✔ QUYỀN ADMIN").size(10.0).strong().color(Color32::from_rgb(0, 255, 163)));
                                    } else {
                                        let btn = egui::Button::new(
                                            RichText::new("🛡 Cấp Quyền Admin").size(10.0).strong().color(Color32::from_rgb(255, 180, 40)),
                                        )
                                        .fill(Color32::from_rgb(45, 30, 12))
                                        .stroke(Stroke::new(1.0, Color32::from_rgb(255, 160, 40)))
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
                                                .color(Color32::from_rgb(120, 145, 175)),
                                            );
                                            ui.add_space(8.0);
                                        }
                                        for a in apps_snapshot.iter() {
                                            let mut k = keep_decision(&self.keep, &a.exe_lower);
                                            if running_app_row(ui, &mut k, a) {
                                                self.keep.insert(a.exe_lower.clone(), k);
                                                changed = true;
                                            }
                                            ui.add_space(2.0);
                                        }
                                    }
                                    ConfigTab::Services => {
                                        for (i, (svc_name, display_name, desc, _)) in SERVICE_TARGETS.iter().enumerate() {
                                            if service_row(ui, &mut self.svc_enabled[i], svc_name, display_name, desc) {
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

                ui.add_space(8.0);

                // 5. TERMINAL LOG
                egui::Frame::none()
                    .fill(Color32::from_rgb(8, 11, 16))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(22, 33, 48)))
                    .rounding(Rounding::same(8.0))
                    .inner_margin(Margin::same(10.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("TERMINAL LOG // NHẬT KÝ HỆ THỐNG")
                                    .size(11.0)
                                    .strong()
                                    .color(Color32::from_rgb(0, 229, 255)),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(
                                    RichText::new("AUTO-SCROLL")
                                        .size(10.0)
                                        .color(Color32::from_rgb(80, 120, 160)),
                                );
                            });
                        });
                        ui.add_space(4.0);

                        let log_height = ui.available_height().max(100.0);
                        egui::ScrollArea::vertical()
                            .stick_to_bottom(true)
                            .max_height(log_height)
                            .show(ui, |ui| {
                                for (line, color) in self.log.lock().unwrap().iter() {
                                    ui.label(RichText::new(line).monospace().size(11.0).color(*color));
                                }
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
    if std::env::args().any(|a| a == "--selftest") {
        let snap = sys::snapshot();
        let mut lines = Vec::new();
        let mut cwd_ok = 0;
        for p in snap.iter() {
            let cwd = sys::working_dir_of(p.pid);
            cwd_ok += cwd.is_some() as usize;
            lines.push(format!("{}	{}	{}	{:?}", p.pid, p.name, p.cmdline.is_some(), cwd));
        }
        let apps = scan_running_apps();
        lines.insert(0, format!("processes={} cwd_ok={} apps_listed={}", snap.len(), cwd_ok, apps.len()));
        for a in apps.iter() {
            lines.push(format!("APP	{}	{}	{}", a.exe_name, a.count, fmt_ram(a.total_ws)));
        }
        let _ = std::fs::write(config_path().with_file_name("selftest.txt"), lines.join("
"));
        return Ok(());
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
