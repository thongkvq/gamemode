#![windows_subsystem = "windows"]

mod fps;
mod hotkey;
mod hwinfo;
mod report;
mod startup;
mod sys;
mod tray;
mod ui;
mod update;
mod wintweaks;

use eframe::egui::{self, Color32, FontId, Margin, Rect, RichText, Rounding, Stroke, pos2, vec2};
use egui_plot::{GridInput, GridMark, HLine, Line, LineStyle, MarkerShape, Plot, PlotPoints, Points, VLine};
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
    // OBS: phát trực tiếp / quay màn hình trong lúc chơi.
    "obs64.exe",
    // NitroSense (Acer): quạt & profile hiệu năng, phải giữ để máy không bị khóa quạt.
    "psagent.exe",
    "psadminagent.exe",
];

/// Exe game mặc định (dùng khi config chưa có dòng `game:` nào).
const DEFAULT_GAMES: &[&str] = &["deltaforceclient-win64-shipping.exe"];
/// Tiền tố tên exe mặc định được giữ (Tailscale: tailscale.exe, tailscaled.exe...).
/// "discord" phủ cả DiscordSystemHelper.exe (chạy tách khỏi cây Discord.exe) và bản PTB/Canary,
/// cần cho livestream / chia sẻ màn hình trên Discord.
const DEFAULT_KEEP_PREFIX: &[&str] = &["discord", "tailscale", "deltaforce", "df_garena", "sguard", "unrealcef", "ace-", "gamebar"];
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
/// App livestream không nén working set: nén Discord / OBS giữa lúc phát làm giật hình, tiếng.
const TRIM_SKIP_PREFIX: &[&str] = &["discord", "obs"];

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

/// Tên exe (viết thường) đang được GIỮ theo đúng quy tắc giữ hiện tại: `keep:` trong config
/// thắng, chưa có thì dùng danh sách mặc định. Game / anti-cheat bị loại vì đã có "họ" process
/// game bảo vệ riêng (xem `game_family_from_nodes`).
fn kept_app_names(
    nodes: &[sys::ProcNode],
    keep: &HashMap<String, bool>,
    skip: &[String],
) -> HashSet<String> {
    let mut out: HashSet<String> = HashSet::new();
    for n in nodes.iter() {
        // Shell/hệ thống không bao giờ làm gốc: con cháu của explorer là gần như mọi app.
        if n.name.is_empty() || is_ace_or_game(&n.name, skip) || FAMILY_STOP_EXE.contains(&n.name.as_str()) {
            continue;
        }
        if keep_decision(keep, &n.name) {
            out.insert(n.name.clone());
        }
    }
    out
}

/// PID -> tên exe của app ĐƯỢC GIỮ mà nó là con cháu (vd `node.exe` -> `claude.exe`).
/// Chỉ đi XUỐNG theo ppid (con, cháu, chắt...) nên tổ tiên của app được giữ không vì thế mà
/// được giữ. Mỗi PID chỉ được thăm một lần nên vòng ppid giả / PID tái sử dụng không treo,
/// và ppid trỏ tới PID không còn trong snapshot thì đơn giản là hết nhánh.
fn kept_family_from_nodes(
    nodes: &[sys::ProcNode],
    keep_names: &HashSet<String>,
) -> HashMap<u32, String> {
    let mut kept: HashMap<u32, String> = HashMap::new();
    if keep_names.is_empty() {
        return kept;
    }

    // 1. Process gốc: chính app được giữ (bản thân nó không nằm trong map).
    let mut roots: Vec<(u32, String)> = Vec::new();
    let mut visited: HashSet<u32> = HashSet::new();
    for n in nodes.iter() {
        if n.pid <= 4 || !keep_names.contains(&n.name) {
            continue;
        }
        if visited.insert(n.pid) {
            roots.push((n.pid, n.name.clone()));
        }
    }

    // 2. Duyệt xuống toàn bộ con cháu; giữ nguyên tên app gốc của cả nhánh.
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for n in nodes.iter() {
        if n.ppid != n.pid {
            children.entry(n.ppid).or_default().push(n.pid);
        }
    }
    let mut queue: Vec<(u32, String)> = roots;
    while let Some((pid, owner)) = queue.pop() {
        let Some(kids) = children.get(&pid) else { continue };
        for &kid in kids.iter() {
            // `visited` đã chứa PID gốc nên app được giữ không bị coi là con của chính nó.
            if !visited.insert(kid) {
                continue;
            }
            kept.insert(kid, owner.clone());
            queue.push((kid, owner.clone()));
        }
    }

    kept
}

/// Một dòng log tóm tắt các process con của app được giữ (dùng khi Game Boost / auto optimize).
fn kept_family_log_line(
    kept_family: &HashMap<u32, String>,
    nodes: &[sys::ProcNode],
) -> Option<String> {
    if kept_family.is_empty() {
        return None;
    }
    let name_of: HashMap<u32, &str> = nodes.iter().map(|n| (n.pid, n.name.as_str())).collect();
    // app cha -> (tên exe con -> số instance)
    let mut by_owner: HashMap<&str, HashMap<&str, usize>> = HashMap::new();
    for (pid, owner) in kept_family.iter() {
        let Some(exe) = name_of.get(pid).copied() else { continue };
        *by_owner.entry(owner.as_str()).or_default().entry(exe).or_insert(0) += 1;
    }
    let mut owners: Vec<(&str, Vec<(&str, usize)>)> = by_owner
        .into_iter()
        .map(|(owner, counts)| {
            let mut kids: Vec<(&str, usize)> = counts.into_iter().collect();
            kids.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
            (owner, kids)
        })
        .collect();
    owners.sort_by(|a, b| a.0.cmp(b.0));

    let parts: Vec<String> = owners
        .iter()
        .map(|(owner, kids)| {
            let list: Vec<String> = kids
                .iter()
                .map(|(exe, count)| {
                    if *count > 1 {
                        format!("{} ×{}", exe, count)
                    } else {
                        (*exe).to_string()
                    }
                })
                .collect();
            format!("{}: {}", owner, list.join(", "))
        })
        .collect();
    Some(format!(
        "  · Giữ {} process con của app được giữ ({})",
        kept_family.len(),
        parts.join("; ")
    ))
}

/// Tên hiển thị của app cha được giữ, dùng cho chú thích "con của ..." trên UI (vd "Claude"):
/// lấy tên hiển thị của process cha trong snapshot, không có thì rút gọn tên exe.
fn kept_parent_label(owner_exe: &str, all: &[sys::ProcInfo]) -> String {
    all.iter()
        .find(|p| p.name.eq_ignore_ascii_case(owner_exe))
        .and_then(|p| p.display_name.clone())
        .filter(|d| !d.trim().is_empty())
        .unwrap_or_else(|| clean_game_name(owner_exe))
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
/// thứ hai cần tìm lại cửa sổ của bản đang chạy (version hiện trên title bar do `ui.rs` vẽ).
const WINDOW_TITLE: &str = "Paroda GameMode";

/// Exe KHÔNG BAO GIỜ là game: app hệ thống, app luôn chạy, trình duyệt, công cụ dev.
/// Đưa nhầm một trong số này vào danh sách game từng gây Boost ngay khi vừa mở app
/// (`game:msedgewebview2.exe=1` — WebView2 luôn chạy nên phiên Boost không bao giờ kết thúc).
/// Ngoài danh sách này, mọi exe bắt đầu bằng `discord` cũng bị chặn.
const NOT_GAMES: &[&str] = &[
    "msedgewebview2.exe",
    "msedge.exe",
    "chrome.exe",
    "firefox.exe",
    "brave.exe",
    "opera.exe",
    "explorer.exe",
    "svchost.exe",
    "dwm.exe",
    "csrss.exe",
    "winlogon.exe",
    "services.exe",
    "lsass.exe",
    "system",
    "registry",
    "searchhost.exe",
    "startmenuexperiencehost.exe",
    "shellexperiencehost.exe",
    "runtimebroker.exe",
    "applicationframehost.exe",
    "textinputhost.exe",
    "ctfmon.exe",
    "conhost.exe",
    "cmd.exe",
    "powershell.exe",
    "pwsh.exe",
    "windowsterminal.exe",
    "taskmgr.exe",
    "steam.exe",
    "steamwebhelper.exe",
    "epicgameslauncher.exe",
    "obs64.exe",
    "claude.exe",
    "code.exe",
    "node.exe",
    "python.exe",
    "gamemode.exe",
];

/// Tên exe (đã viết thường) có bị cấm đưa vào danh sách game không.
fn is_not_game(exe_lower: &str) -> bool {
    NOT_GAMES.contains(&exe_lower) || exe_lower.starts_with("discord")
}

fn config_path() -> PathBuf {
    std::env::current_exe()
        .map(|p| p.with_file_name("gamemode.cfg"))
        .unwrap_or_else(|_| PathBuf::from("gamemode.cfg"))
}

/// Session khôi phục nằm cạnh file config.
fn session_path() -> PathBuf {
    config_path().with_file_name("session.json")
}

/// File sao lưu giá trị gốc của tinh chỉnh Windows — cạnh exe, cùng thư mục `config_path()`.
/// KHÔNG xoá khi gỡ app (xem NOTES): cài lại vẫn còn để hoàn tác.
fn wintweaks_backup_path() -> PathBuf {
    config_path().with_file_name(wintweaks::FILE_NAME)
}

/// Thư mục cha của mọi đường dẫn `gamepath:` trong config (bỏ trùng, sắp xếp).
/// Dùng cho tinh chỉnh `defender_games` (thêm thư mục game vào loại trừ Defender).
fn game_dirs_from_paths(paths: &HashMap<String, String>) -> Vec<String> {
    let mut dirs: Vec<String> = paths
        .values()
        .filter_map(|p| {
            std::path::Path::new(p)
                .parent()
                .map(|d| d.display().to_string())
        })
        .map(|d| d.trim().to_string())
        .filter(|d| !d.is_empty())
        .collect();
    dirs.sort();
    dirs.dedup();
    dirs
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

// ===== BEGIN PURE-PROFILE-HOTKEY =====
// Vùng này chỉ chứa kiểu/hàm THUẦN: không đọc/ghi đĩa, không WinAPI, không phụ thuộc `Shared`.
// `.tools/task26test/extract.sh` cắt NGUYÊN VĂN vùng này ra để chạy `cargo test` trên Linux —
// trong vùng chỉ được dùng item cũng nằm trong vùng, `HashMap` và module `hotkey`.

/// Tuỳ chọn tối ưu khi chơi (khối "Tối ưu khi chơi" + "Tự động khi chơi game" + "Phím tắt").
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
    /// `update:enabled`: tự kiểm tra bản cập nhật OTA từ server nội bộ.
    update_enabled: bool,
    /// `hotkey:enabled`: bật phím tắt toàn cục (Boost / Khôi phục / Hiện-ẩn cửa sổ).
    hotkey_enabled: bool,
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
            update_enabled: true,
            hotkey_enabled: true,
        }
    }
}

/// Các key của hồ sơ riêng theo game, kèm nhãn tiếng Việt (dùng cho cả khối "Hồ sơ" trên giao
/// diện lẫn dòng log). Thứ tự ở đây cũng là thứ tự hiện trên giao diện.
const PROFILE_KEYS: &[(&str, &str)] = &[
    ("standby", "Xả standby (RAM chờ)"),
    ("trim", "Nén RAM app giữ lại"),
    ("gpu", "Ép GPU rời"),
    ("wlan", "Tắt quét Wi-Fi nền"),
    ("gamedvr", "Tắt Game DVR"),
    ("report", "Ghi báo cáo trận"),
    ("close_apps", "Tắt app nền"),
    ("stop_services", "Dừng service"),
];

/// Hồ sơ riêng của MỘT game: key -> bật/tắt. Key không có trong map nghĩa là "theo cài đặt
/// chung", nên hồ sơ rỗng = chung hết.
type GameProfile = HashMap<String, bool>;

/// Thứ tự cố định của ba hành động phím tắt (config, giao diện, đăng ký đều theo thứ tự này).
const HOTKEY_ACTIONS: [hotkey::HotkeyAction; 3] = [
    hotkey::HotkeyAction::Boost,
    hotkey::HotkeyAction::Restore,
    hotkey::HotkeyAction::ToggleWindow,
];

fn is_on(v: &str) -> bool {
    v.trim() == "1"
}

/// Ghép cài đặt chung với hồ sơ riêng của game: trả `(tuỳ chọn hiệu lực, tắt app nền, dừng
/// service)`. Hàm thuần (không đụng đĩa/WinAPI) nên có unit test chạy được cả trên Linux.
///
/// `close_apps = false` nghĩa là Boost **không tắt app nền nào** — chỉ chạy các bước còn lại và
/// không ghi app nào vào session. `stop_services = false` thì không dừng service nào.
fn effective_opts(general: &Opts, profile: Option<&GameProfile>) -> (Opts, bool, bool) {
    let mut o = *general;
    let mut close_apps = true;
    let mut stop_services = true;
    if let Some(p) = profile {
        let get = |key: &str, fallback: bool| p.get(key).copied().unwrap_or(fallback);
        o.standby = get("standby", o.standby);
        o.trim = get("trim", o.trim);
        o.gpu = get("gpu", o.gpu);
        o.wlan = get("wlan", o.wlan);
        o.gamedvr = get("gamedvr", o.gamedvr);
        o.report_enabled = get("report", o.report_enabled);
        close_apps = get("close_apps", true);
        stop_services = get("stop_services", true);
    }
    (o, close_apps, stop_services)
}

/// Dòng log "đang dùng hồ sơ nào": chỉ liệt kê các key CÓ trong hồ sơ (đó chính là những khác
/// biệt so với cài đặt chung mà người dùng đã chọn).
fn profile_diff_line(game: &str, profile: &GameProfile) -> String {
    let mut parts: Vec<String> = Vec::new();
    for (key, label) in PROFILE_KEYS {
        if let Some(on) = profile.get(*key) {
            parts.push(format!("{}={}", label, if *on { "BẬT" } else { "TẮT" }));
        }
    }
    if parts.is_empty() {
        format!("• [HỒ SƠ] Dùng hồ sơ {} (chưa đặt khác biệt nào).", game)
    } else {
        format!("• [HỒ SƠ] Dùng hồ sơ {}: {}", game, parts.join(", "))
    }
}

/// Khoá trong config (`hotkey:boost/restore/window`) -> hành động phím tắt.
fn action_from_config_key(key: &str) -> Option<hotkey::HotkeyAction> {
    match key.trim().to_ascii_lowercase().as_str() {
        "boost" => Some(hotkey::HotkeyAction::Boost),
        "restore" => Some(hotkey::HotkeyAction::Restore),
        "window" => Some(hotkey::HotkeyAction::ToggleWindow),
        _ => None,
    }
}

/// Phím tắt mặc định (dùng khi config chưa có dòng `hotkey:<tên>` nào).
fn default_hotkeys() -> HashMap<hotkey::HotkeyAction, String> {
    hotkey::default_bindings()
        .into_iter()
        .map(|(action, combo)| (action, combo.to_string()))
        .collect()
}

/// Đọc tổ hợp phím tắt trong config thành danh sách đăng ký được.
/// Chuỗi rỗng = phím đó đang tắt; chuỗi sai định dạng nằm trong danh sách lỗi thứ hai để giao
/// diện hiện đỏ trong card "Phím tắt" (không đăng ký phím đó).
fn parse_hotkeys(
    map: &HashMap<hotkey::HotkeyAction, String>,
) -> (
    Vec<(hotkey::HotkeyAction, hotkey::Hotkey)>,
    Vec<(hotkey::HotkeyAction, String)>,
) {
    let mut ok = Vec::new();
    let mut errs = Vec::new();
    for action in HOTKEY_ACTIONS {
        let raw = map.get(&action).cloned().unwrap_or_default();
        if raw.trim().is_empty() {
            continue;
        }
        match hotkey::Hotkey::parse(&raw) {
            Ok(hk) => ok.push((action, hk)),
            Err(e) => errs.push((action, e)),
        }
    }
    (ok, errs)
}

/// Cấu hình thêm trong `gamemode.cfg`: hồ sơ riêng theo game + phím tắt toàn cục.
struct ExtraConfig {
    profiles: HashMap<String, GameProfile>,
    /// `hotkey:enabled` (mặc định 1).
    hotkey_enabled: bool,
    /// `hotkey:boost/restore/window` — thiếu dòng nào thì dùng mặc định của phím đó.
    hotkeys: HashMap<hotkey::HotkeyAction, String>,
}

/// Đọc các dòng `profile:<exe>:<key>=1/0` và `hotkey:enabled|boost|restore|window=...`.
/// `blocked` là bộ lọc "app hệ thống / luôn chạy" (`is_not_game`): hồ sơ của chúng bị bỏ qua.
fn parse_extra_config(text: &str, blocked: fn(&str) -> bool) -> ExtraConfig {
    let mut out = ExtraConfig {
        profiles: HashMap::new(),
        hotkey_enabled: true,
        hotkeys: default_hotkeys(),
    };
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(rest) = line.strip_prefix("profile:") {
            // `profile:<exe>:<key>=1/0`; đường dẫn exe có thể chứa ':' nên cắt ở dấu ':' CUỐI.
            let Some((head, value)) = rest.rsplit_once('=') else { continue };
            let Some((exe, key)) = head.rsplit_once(':') else { continue };
            let exe = exe.trim().to_lowercase();
            let key = key.trim().to_lowercase();
            if exe.is_empty() || !PROFILE_KEYS.iter().any(|(name, _)| *name == key) {
                continue;
            }
            // App hệ thống / app luôn chạy không bao giờ là game -> hồ sơ của nó bị bỏ.
            if blocked(&exe) {
                continue;
            }
            out.profiles.entry(exe).or_default().insert(key, is_on(value));
        } else if let Some(rest) = line.strip_prefix("hotkey:") {
            let Some((key, value)) = rest.rsplit_once('=') else { continue };
            let key = key.trim().to_ascii_lowercase();
            if key == "enabled" {
                out.hotkey_enabled = is_on(value);
            } else if let Some(action) = action_from_config_key(&key) {
                // Chuỗi rỗng = tắt hẳn phím này (vẫn ghi lại đúng chuỗi rỗng).
                out.hotkeys.insert(action, value.trim().to_string());
            }
        }
    }
    out
}

/// Các dòng config của hồ sơ riêng + phím tắt (ngược của `parse_extra_config`).
/// Hồ sơ của game KHÔNG còn trong `games` bị bỏ luôn (xoá game là xoá hồ sơ của nó).
fn extra_config_lines(
    profiles: &HashMap<String, GameProfile>,
    games: &[String],
    hotkey_enabled: bool,
    hotkeys: &HashMap<hotkey::HotkeyAction, String>,
) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut exes: Vec<&String> = profiles.keys().filter(|e| games.contains(e)).collect();
    exes.sort();
    for exe in exes {
        let Some(profile) = profiles.get(exe) else { continue };
        let mut keys: Vec<&String> = profile.keys().collect();
        keys.sort();
        for key in keys {
            lines.push(format!(
                "profile:{}:{}={}",
                exe,
                key,
                if profile[key] { 1 } else { 0 }
            ));
        }
    }
    lines.push(format!("hotkey:enabled={}", if hotkey_enabled { 1 } else { 0 }));
    for action in HOTKEY_ACTIONS {
        let combo = hotkeys.get(&action).cloned().unwrap_or_default();
        lines.push(format!("hotkey:{}={}", action.config_key(), combo));
    }
    lines
}

/// Rút gọn dòng lệnh dài của mục khởi động cho vừa một dòng (tooltip hiện đầy đủ).
fn shorten_command(cmd: &str, max_chars: usize) -> String {
    if cmd.chars().count() <= max_chars || max_chars <= 20 {
        return cmd.to_string();
    }
    let head: String = cmd.chars().take(max_chars - 18).collect();
    let tail: String = cmd
        .chars()
        .rev()
        .take(16)
        .collect::<Vec<char>>()
        .into_iter()
        .rev()
        .collect();
    format!("{}…{}", head, tail)
}

/// Mục khởi động chính là bản GameMode đang chạy (exe trùng exe hiện tại) -> khoá công tắc:
/// tắt nó là tự tắt chính app, còn việc chạy cùng Windows do task "GameMode" lo.
fn is_self_startup(exe_path: Option<&str>, current_exe: &str) -> bool {
    let Some(path) = exe_path else { return false };
    if current_exe.trim().is_empty() {
        return false;
    }
    let norm = |s: &str| s.replace('"', "").replace('/', "\\").trim().to_lowercase();
    norm(path) == norm(current_exe)
}

#[cfg(test)]
mod profile_hotkey_tests {
    use super::*;

    /// Không có hồ sơ: giữ nguyên cài đặt chung, vẫn tắt app và dừng service.
    #[test]
    fn effective_opts_khong_co_ho_so() {
        let general = Opts { trim: false, wlan: false, ..Opts::default() };
        let (o, close_apps, stop_services) = effective_opts(&general, None);
        assert!(!o.trim && !o.wlan && o.standby, "phải giữ nguyên cài đặt chung");
        assert!(close_apps && stop_services);
    }

    /// Chỉ key CÓ trong hồ sơ mới đè cài đặt chung; hồ sơ rỗng = chung hết.
    #[test]
    fn effective_opts_chi_de_key_co_trong_ho_so() {
        let general = Opts::default();
        let (o, close_apps, stop_services) = effective_opts(&general, Some(&GameProfile::new()));
        assert!(o.trim && o.wlan && o.gpu && o.gamedvr && o.standby && o.report_enabled);
        assert!(close_apps && stop_services, "hồ sơ rỗng không được tắt app / service");

        let profile = GameProfile::from([
            ("trim".to_string(), false),
            ("report".to_string(), false),
            ("close_apps".to_string(), false),
            ("stop_services".to_string(), false),
        ]);
        let (o, close_apps, stop_services) = effective_opts(&general, Some(&profile));
        assert!(!o.trim && !o.report_enabled, "key có trong hồ sơ phải đè cài đặt chung");
        assert!(o.wlan && o.gpu, "key không có trong hồ sơ giữ cài đặt chung");
        assert!(!close_apps && !stop_services, "close_apps/stop_services = 0");
    }

    /// Hồ sơ ghi 1 thì bật lại tuỳ chọn mà cài đặt chung đang tắt.
    #[test]
    fn effective_opts_bat_lai_tuy_chon_chung_dang_tat() {
        let general = Opts { standby: false, gamedvr: false, ..Opts::default() };
        let profile = GameProfile::from([("standby".to_string(), true)]);
        let (o, close_apps, stop_services) = effective_opts(&general, Some(&profile));
        assert!(o.standby, "hồ sơ ghi 1 thì bật dù cài đặt chung đang tắt");
        assert!(!o.gamedvr, "key không có trong hồ sơ vẫn theo cài đặt chung");
        assert!(close_apps && stop_services);
    }

    #[test]
    fn profile_diff_line_liet_ke_dung_thu_tu() {
        let empty = GameProfile::new();
        assert!(profile_diff_line("Delta Force", &empty).contains("chưa đặt khác biệt"));

        let profile = GameProfile::from([
            ("close_apps".to_string(), false),
            ("trim".to_string(), false),
            ("standby".to_string(), true),
        ]);
        let line = profile_diff_line("Delta Force", &profile);
        assert!(line.contains("Delta Force"), "{line}");
        let pos_standby = line.find("Xả standby").expect("thiếu standby");
        let pos_trim = line.find("Nén RAM").expect("thiếu trim");
        let pos_close = line.find("Tắt app nền").expect("thiếu close_apps");
        assert!(pos_standby < pos_trim && pos_trim < pos_close, "sai thứ tự: {line}");
        assert!(
            line.contains("Tắt app nền=TẮT") && line.contains("Xả standby (RAM chờ)=BẬT"),
            "{line}"
        );
        assert!(!line.contains("Wi-Fi"), "key không có trong hồ sơ không được liệt kê: {line}");
    }

    /// Config chưa có dòng `hotkey:` nào -> dùng mặc định của `hotkey::default_bindings()`.
    #[test]
    fn parse_extra_config_mac_dinh_phim_tat() {
        let extra = parse_extra_config("", |_| false);
        assert!(extra.hotkey_enabled, "hotkey:enabled mặc định là 1");
        assert!(extra.profiles.is_empty());
        for (action, combo) in hotkey::default_bindings() {
            assert_eq!(extra.hotkeys.get(&action).map(String::as_str), Some(combo));
        }
        let (bindings, errs) = parse_hotkeys(&extra.hotkeys);
        assert_eq!(bindings.len(), 3, "ba phím mặc định phải parse được: {errs:?}");
        assert!(errs.is_empty());
    }

    /// Chuỗi rỗng = tắt phím đó; `enabled=0` tắt cả dịch vụ phím tắt.
    #[test]
    fn parse_extra_config_tat_phim_va_tat_dich_vu() {
        let text = "hotkey:enabled=0\nhotkey:boost=Ctrl+Shift+F9\nhotkey:restore=\nhotkey:window=Ctrl+Alt+G\n";
        let extra = parse_extra_config(text, |_| false);
        assert!(!extra.hotkey_enabled);
        assert_eq!(
            extra.hotkeys.get(&hotkey::HotkeyAction::Boost).map(String::as_str),
            Some("Ctrl+Shift+F9")
        );
        assert_eq!(extra.hotkeys.get(&hotkey::HotkeyAction::Restore).map(String::as_str), Some(""));
        let (bindings, errs) = parse_hotkeys(&extra.hotkeys);
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(bindings.len(), 2, "phím để trống bị bỏ qua");
        assert!(!bindings.iter().any(|(a, _)| *a == hotkey::HotkeyAction::Restore));
        assert!(bindings.iter().any(|(a, _)| *a == hotkey::HotkeyAction::ToggleWindow));
    }

    /// Tổ hợp sai định dạng: báo lỗi để UI hiện đỏ, không đăng ký phím đó.
    #[test]
    fn parse_hotkeys_bao_loi_to_hop_sai() {
        let mut map = default_hotkeys();
        map.insert(hotkey::HotkeyAction::Boost, "Ctrl++B".to_string());
        map.insert(hotkey::HotkeyAction::ToggleWindow, "   ".to_string());
        let (bindings, errs) = parse_hotkeys(&map);
        assert_eq!(bindings.len(), 1, "chỉ còn Restore hợp lệ");
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].0, hotkey::HotkeyAction::Boost);
        assert!(!errs[0].1.is_empty());
    }

    #[test]
    fn action_from_config_key_dung_ba_ten() {
        assert_eq!(action_from_config_key("boost"), Some(hotkey::HotkeyAction::Boost));
        assert_eq!(action_from_config_key(" Restore "), Some(hotkey::HotkeyAction::Restore));
        assert_eq!(action_from_config_key("window"), Some(hotkey::HotkeyAction::ToggleWindow));
        assert_eq!(action_from_config_key("khac"), None);
    }

    fn blocked_not_games(exe: &str) -> bool {
        exe == "explorer.exe" || exe == "msedgewebview2.exe"
    }

    /// Dòng `profile:` đọc đúng; key lạ / app hệ thống / dòng rác bị bỏ qua.
    #[test]
    fn parse_extra_config_doc_ho_so_theo_game() {
        let text = "\
profile:DeltaForceClient-Win64-Shipping.exe:trim=0
profile:deltaforceclient-win64-shipping.exe:close_apps=0
profile:C:\\Games\\Delta Force\\game.exe:stop_services=0
profile:explorer.exe:trim=1
profile:msedgewebview2.exe:gpu=1
profile:game.exe:khong_co_key_nay=1
profile:game.exe=1
profile:game.exe:trim
hotkey:khong_co=1
rac
";
        let extra = parse_extra_config(text, blocked_not_games);
        let df = extra
            .profiles
            .get("deltaforceclient-win64-shipping.exe")
            .expect("phải có hồ sơ Delta Force");
        assert_eq!(df.get("trim"), Some(&false));
        assert_eq!(df.get("close_apps"), Some(&false), "hai dòng cùng exe phải gộp");
        assert_eq!(
            extra
                .profiles
                .get(r"c:\games\delta force\game.exe")
                .and_then(|p| p.get("stop_services")),
            Some(&false),
            "đường dẫn có dấu cách vẫn đọc được"
        );
        assert!(!extra.profiles.contains_key("explorer.exe"), "app hệ thống bị bỏ");
        assert!(!extra.profiles.contains_key("msedgewebview2.exe"));
        assert_eq!(extra.profiles.get("game.exe"), None, "key lạ / dòng thiếu vế bị bỏ");
        assert!(extra.hotkeys.contains_key(&hotkey::HotkeyAction::Boost), "dòng hotkey lạ không phá mặc định");
    }

    /// Ghi rồi đọc lại phải ra đúng hồ sơ + phím tắt; hồ sơ của game đã xoá không còn dòng nào.
    #[test]
    fn extra_config_lines_round_trip_va_xoa_theo_game() {
        let profiles = HashMap::from([
            (
                "deltaforceclient-win64-shipping.exe".to_string(),
                GameProfile::from([("trim".to_string(), false), ("close_apps".to_string(), false)]),
            ),
            ("cs2.exe".to_string(), GameProfile::from([("gpu".to_string(), true)])),
        ]);
        let games = vec!["deltaforceclient-win64-shipping.exe".to_string()];
        let hotkeys = HashMap::from([
            (hotkey::HotkeyAction::Boost, "Ctrl+Shift+F9".to_string()),
            (hotkey::HotkeyAction::Restore, String::new()),
            (hotkey::HotkeyAction::ToggleWindow, "Ctrl+Alt+G".to_string()),
        ]);
        let lines = extra_config_lines(&profiles, &games, false, &hotkeys);
        let text = lines.join("\n");
        assert!(!text.contains("cs2.exe"), "game đã xoá thì hồ sơ cũng bị xoá: {text}");
        assert!(text.contains("profile:deltaforceclient-win64-shipping.exe:trim=0"), "{text}");
        assert!(text.contains("hotkey:restore=\n"), "phím đang tắt vẫn ghi dòng rỗng: {text}");

        let back = parse_extra_config(&text, |_| false);
        assert!(!back.hotkey_enabled);
        assert_eq!(back.profiles.len(), 1);
        assert_eq!(back.profiles["deltaforceclient-win64-shipping.exe"]["close_apps"], false);
        assert_eq!(
            back.hotkeys.get(&hotkey::HotkeyAction::Boost).map(String::as_str),
            Some("Ctrl+Shift+F9")
        );
        assert_eq!(back.hotkeys.get(&hotkey::HotkeyAction::Restore).map(String::as_str), Some(""));
    }

    #[test]
    fn shorten_command_cat_giua_dong_lenh() {
        let short = r"C:\a\b.exe";
        assert_eq!(shorten_command(short, 60), short);
        let long = r#""C:\Program Files\Very Long App Name\app.exe" --min --flag other"#;
        let cut = shorten_command(long, 40);
        assert!(cut.chars().count() <= 40, "{cut}");
        assert!(cut.contains('…'), "{cut}");
        assert!(cut.starts_with(r#""C:\Program Files"#), "{cut}");
        assert!(cut.ends_with("other"), "{cut}");
    }

    #[test]
    fn is_self_startup_so_exe_khong_phan_biet_hoa_thuong() {
        let me = r"C:\Users\thong\AppData\Local\Programs\GameMode\GameMode.exe";
        assert!(is_self_startup(Some(me), me));
        assert!(is_self_startup(
            Some(r"C:\Users\thong\appdata\local\programs\gamemode\gamemode.exe"),
            me
        ));
        assert!(is_self_startup(Some(r#""C:\Users\thong\AppData\Local\Programs\GameMode\GameMode.exe""#), me));
        assert!(!is_self_startup(Some(r"C:\Users\thong\GameMode\GameMode.exe"), me), "bản portable cũ không bị khoá");
        assert!(!is_self_startup(Some("Discord.lnk"), me));
        assert!(!is_self_startup(None, me), "file .lnk chưa đọc được đích thì không khoá");
        assert!(!is_self_startup(Some(me), ""), "không biết exe hiện tại thì không khoá gì");
    }
}
// ===== END PURE-PROFILE-HOTKEY =====

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
    /// URL manifest cập nhật OTA (`update:url`).
    update_url: String,
    /// Dòng `game:` bị bỏ qua vì là app hệ thống / luôn chạy (`NOT_GAMES`): chỉ để báo log
    /// một lần lúc mở app. Lần ghi config sau sẽ tự bỏ các dòng này.
    skipped_games: Vec<String>,
    /// Hồ sơ riêng theo game (`profile:<exe>:<key>=1/0`); xem `PROFILE_KEYS`.
    profiles: HashMap<String, GameProfile>,
    /// Phím tắt toàn cục (`hotkey:boost/restore/window`): chuỗi tổ hợp, rỗng = phím đang tắt.
    /// `hotkey:enabled` nằm trong `opts.hotkey_enabled`.
    hotkeys: HashMap<hotkey::HotkeyAction, String>,
    opts: Opts,
}

/// Đọc cấu hình: `keep:<exe>=1/0`, `svc:<tên>=1/0`, `game:<exe>=1`, `gamepath:<exe>=<path>`,
/// `auto:enabled/restore=1/0`, `opt:standby/trim/gpu/wlan/gamedvr=1/0`,
/// `ui:close_to_tray=1/0`, `report:enabled=1/0`, `report:ping_host=<host>`,
/// `update:enabled=1/0`, `update:url=<url manifest>`,
/// `profile:<exe>:<key>=1/0` (key xem `PROFILE_KEYS`) và
/// `hotkey:enabled=1/0`, `hotkey:boost|restore|window=<tổ hợp>` (rỗng = tắt phím đó).
/// Các dòng `app:...` của bản cũ được bỏ qua (không còn danh sách app cố định).
fn load_config() -> Config {
    let text = std::fs::read_to_string(config_path()).unwrap_or_default();
    parse_config(&text)
}

/// Đọc nội dung `gamemode.cfg` đã tách khỏi đĩa (để unit test được). Xem `load_config`.
fn parse_config(text: &str) -> Config {
    let mut keep_saved: HashMap<String, bool> = HashMap::new();
    let mut svc_saved: HashMap<String, bool> = HashMap::new();
    let mut games: Vec<String> = Vec::new();
    let mut skipped_games: Vec<String> = Vec::new();
    let mut game_paths: HashMap<String, String> = HashMap::new();
    let mut ping_host = report::PING_HOST_DEFAULT.to_string();
    let mut update_url = update::DEFAULT_MANIFEST_URL.to_string();
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
                    // App hệ thống / app luôn chạy không bao giờ là game: bỏ qua dòng này
                    // (không nạp vào danh sách, lần ghi config sau sẽ tự xoá khỏi file).
                    if is_not_game(&exe) {
                        if !skipped_games.contains(&exe) {
                            skipped_games.push(exe);
                        }
                    } else {
                        games.push(exe);
                    }
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
        } else if let Some(rest) = line.strip_prefix("update:") {
            if let Some((k, v)) = rest.rsplit_once('=') {
                match k.trim() {
                    "enabled" => opts.update_enabled = is_on(v),
                    "url" => {
                        let url = v.trim();
                        if !url.is_empty() {
                            update_url = url.to_string();
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

    // Hồ sơ riêng theo game + phím tắt toàn cục (phần đọc nằm trong vùng thuần để test được).
    let extra = parse_extra_config(text, is_not_game);
    opts.hotkey_enabled = extra.hotkey_enabled;

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
        update_url,
        skipped_games,
        profiles: extra.profiles,
        hotkeys: extra.hotkeys,
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
    lines.push(format!("update:enabled={}", if o.update_enabled { 1 } else { 0 }));
    lines.push(format!("update:url={}", cfg.update_url));
    // Hồ sơ riêng theo game + phím tắt toàn cục (hồ sơ của game đã bị xoá khỏi danh sách
    // tự động bị bỏ ở đây).
    lines.extend(extra_config_lines(
        &cfg.profiles,
        &cfg.games,
        o.hotkey_enabled,
        &cfg.hotkeys,
    ));
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
    /// Số instance là CON CHÁU của một app được giữ (không tắt, không nén RAM): chúng không bị
    /// tắt nên không vào session, và UI hiện "GIỮ LẠI" cho phần này.
    kept_children: usize,
    /// Tên hiển thị của app cha được giữ (vd "Claude") khi có instance là con cháu của nó.
    kept_by: Option<String>,
}

/// Dòng app này có được GIỮ LẠI hoàn toàn không (mọi instance đều không bị tắt):
/// do tick "Giữ lại" / danh sách mặc định, hoặc vì mọi instance đều là con cháu của app được giữ.
fn app_row_kept(keep: &HashMap<String, bool>, app: &RunningApp) -> bool {
    keep_decision(keep, &app.exe_lower) || (app.kept_children > 0 && app.kept_children >= app.count)
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
/// nhóm exe có PID thuộc "họ" process game cũng không hiện. `keep` = lựa chọn "Giữ lại" của
/// config: con cháu của app được giữ vẫn hiện nhưng đánh dấu là đã được giữ.
fn scan_running_apps(
    skip: &[String],
    games: &[String],
    keep: &HashMap<String, bool>,
) -> Vec<RunningApp> {
    let me = std::process::id();
    let my_session = sys::current_session();
    let nodes = sys::process_nodes();
    // Họ process game tính trước khi quét sâu: con cháu game (helper của launcher, CEF...)
    // cũng chỉ được mở bằng quyền hạn chế, không đọc bộ nhớ.
    let protect: HashSet<u32> = game_family_from_nodes(&nodes, games).into_keys().collect();
    // Con cháu của app được giữ (`keep:` + mặc định) cũng không bị tắt -> đánh dấu để UI hiện
    // "GIỮ LẠI" và khoá công tắc của dòng đó.
    let kept_family = kept_family_from_nodes(&nodes, &kept_app_names(&nodes, keep, skip));
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
        let kept_by = kept_family
            .get(&p.pid)
            .map(|owner| kept_parent_label(owner, &all));
        let entry = map.entry(exe_lower.clone()).or_insert_with(|| RunningApp {
            exe_lower: exe_lower.clone(),
            exe_name: p.name.clone(),
            display: None,
            exe_path: None,
            count: 0,
            total_ws: 0,
            kept_children: 0,
            kept_by: None,
        });
        entry.count += 1;
        entry.total_ws += p.ws;
        if let Some(label) = kept_by {
            entry.kept_children += 1;
            if entry.kept_by.is_none() {
                entry.kept_by = Some(label);
            }
        }
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

fn restart_as_admin(sh: &Shared) {
    // Mở lại app bằng quyền admin = tiến trình này thoát: chốt báo cáo trận đang ghi trước.
    finish_live_report(sh);
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
    Startup,
    AutoOpt,
    WinOpt,
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
    /// Trạng thái cập nhật OTA (thread nền trong `update.rs` ghi, UI đọc).
    update_state: Arc<Mutex<update::UpdateState>>,
    /// Manifest của bản đang chờ cài (giữ lại để biết version đích khi cài).
    update_manifest: Arc<Mutex<Option<update::Manifest>>>,
    /// Thời điểm kiểm tra cập nhật gần nhất.
    update_last_check: Arc<Mutex<Option<Instant>>>,
    /// UI bấm "Kiểm tra cập nhật" -> thread nền kiểm tra ngay.
    update_request: Arc<AtomicBool>,
    /// URL manifest (`update:url`).
    update_url: Arc<Mutex<String>>,
    /// `session.json` còn nhưng KHÔNG có game nào trong danh sách đang chạy: phiên Game Boost
    /// bị bỏ lửng (app bị tắt giữa phiên). Bật banner "Khôi phục ngay", watcher không tự Boost
    /// để khỏi ghi đè session. Tắt khi `do_restore` chạy xong (session.json -> session.last.json).
    orphan_session: Arc<AtomicBool>,
    /// Lúc mở app thấy `session.json` còn VÀ game trong danh sách vẫn đang chạy: watcher đọc cờ
    /// này một lần rồi tự coi như đang trong phiên (không Boost lần nữa, game tắt thì khôi phục).
    resume_session: Arc<AtomicBool>,
    /// Hồ sơ riêng theo game (`profile:<exe>:<key>=1/0`), đọc/ghi cùng config.
    profiles: Arc<Mutex<HashMap<String, GameProfile>>>,
    /// Tổ hợp phím tắt trong config: rỗng = phím đang tắt. UI sửa map này rồi gọi `rebind`.
    hotkeys: Arc<Mutex<HashMap<hotkey::HotkeyAction, String>>>,
    /// Tổ hợp đã parse và ĐANG có hiệu lực (thread phím tắt đọc để ghi log).
    hotkey_bindings: Arc<Mutex<Vec<(hotkey::HotkeyAction, hotkey::Hotkey)>>>,
}

impl Shared {
    fn save_config(&self) -> bool {
        let cfg = Config {
            keep: self.keep.lock().unwrap().clone(),
            svcs: self.svc.lock().unwrap().clone(),
            games: self.games.lock().unwrap().clone(),
            game_paths: self.game_paths.lock().unwrap().clone(),
            ping_host: self.ping_host.lock().unwrap().clone(),
            update_url: self.update_url.lock().unwrap().clone(),
            skipped_games: Vec::new(),
            profiles: self.profiles.lock().unwrap().clone(),
            hotkeys: self.hotkeys.lock().unwrap().clone(),
            opts: *self.opts.lock().unwrap(),
        };
        write_config(&cfg)
    }

    /// Hồ sơ riêng của một game (`None` = chưa có dòng `profile:` nào cho game đó).
    fn profile_of(&self, exe: &str) -> Option<GameProfile> {
        self.profiles
            .lock()
            .unwrap()
            .get(&exe.to_lowercase())
            .cloned()
    }

    /// Tổ hợp đang có hiệu lực của một hành động (rỗng = phím đang tắt).
    fn hotkey_display(&self, action: hotkey::HotkeyAction) -> String {
        self.hotkey_bindings
            .lock()
            .unwrap()
            .iter()
            .find(|(a, _)| *a == action)
            .map(|(_, hk)| hk.display())
            .unwrap_or_default()
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

    /// Trạng thái cập nhật OTA hiện tại (bản sao, không giữ khoá).
    fn update_state(&self) -> update::UpdateState {
        self.update_state.lock().unwrap().clone()
    }
}

/// App thoát thật (đóng cửa sổ, menu "Thoát" ở khay, cài cập nhật OTA, mở lại bằng quyền
/// admin): chốt trận đang ghi và lưu báo cáo NGAY, không đợi watcher thấy game tắt.
/// Bản lưu tạm (`report::save_partial`) là lưới an toàn cho trường hợp tắt máy / crash.
fn finish_live_report(sh: &Shared) {
    // Lấy Arc ra rồi nhả khoá `live_report` trước khi khoá báo cáo (xem comment ở `Shared`).
    let live = sh.live_report.lock().unwrap().take();
    if let Some(l) = live {
        let out = report::finish(l);
        sh.log(
            out.line.clone(),
            if out.saved {
                ui::SUCCESS
            } else {
                Color32::from_rgb(130, 150, 170)
            },
        );
    }
}

/// Tự sửa task "chạy cùng Windows" lúc mở app: task cũ tạo bằng `/SC ONLOGON` (chỉ chạy khi
/// cắm sạc, tự chết sau 72 giờ, priority thấp) hoặc trỏ sai exe thì tạo lại bằng XML mới.
///
/// Chạy ở thread riêng vì `schtasks` là tiến trình ngoài (mất vài trăm ms), không được chặn UI.
/// `task_needs_repair()` trả `false` khi task chưa tồn tại — việc tạo task vẫn do người dùng chọn
/// bằng công tắc trong tab Tự động & Tối ưu.
fn repair_task_in_background(sh: Shared) {
    std::thread::spawn(move || {
        if !sys::task_needs_repair() {
            return;
        }
        match sys::task_create() {
            Ok(_) => sh.log(
                "✔ [TASK] Đã cập nhật task GameMode: trỏ đúng exe, chạy cả khi dùng pin.".into(),
                ui::SUCCESS,
            ),
            Err(e) => sh.log(
                format!("  ⚠ [TASK] Không cập nhật được task GameMode: {}", e),
                ui::DANGER,
            ),
        }
    });
}

/// Bắt đầu tối ưu ở thread nền. Dùng chung cho nút trên UI, menu khay và phím tắt.
fn optimize(sh: Shared, from_auto: bool) {
    if sh.busy.swap(true, Ordering::SeqCst) {
        return;
    }
    sh.last_auto.store(from_auto, Ordering::SeqCst);
    sh.opt_count.fetch_add(1, Ordering::SeqCst);
    // Xoá nhật ký NGAY trên thread gọi (trước khi spawn): tự Boost khi game mở thì giữ nhật ký
    // (dòng "[AUTO] Phát hiện", "Bắt đầu ghi trận"). Trước đây `do_optimize` xoá ở thread nền
    // nên dòng "vừa bấm phím tắt" ghi ngay sau đó bị mất.
    if !from_auto {
        log_clear(&sh);
    }
    sh.event(
        "optimize",
        if from_auto {
            "Tối ưu tự động khi game mở"
        } else {
            "Tối ưu bằng tay (nút Game Boost / menu khay / phím tắt)"
        },
    );
    std::thread::spawn(move || {
        do_optimize(&sh);
        sh.busy.store(false, Ordering::SeqCst);
    });
}

/// Bắt đầu Restore ở thread nền (UI, menu khay, phím tắt và watcher dùng chung).
/// `from_auto`: watcher tự khôi phục sau khi game tắt -> giữ nhật ký (dòng tóm tắt trận vừa ghi).
fn restore(sh: Shared, from_auto: bool) {
    if sh.busy.swap(true, Ordering::SeqCst) {
        return;
    }
    // Đã khôi phục thì phiên Game Boost tự động kết thúc: watcher không khôi phục thêm lần nữa
    // khi game tắt (trước đây bấm Khôi phục giữa trận thì lúc game tắt lại chạy restore lần 2).
    sh.last_auto.store(false, Ordering::SeqCst);
    // Xoá nhật ký ở đây (không phải trong `do_restore` chạy ở thread nền) để dòng "vừa bấm
    // phím tắt" ghi sau lời gọi này không bị xoá.
    if !from_auto {
        log_clear(&sh);
    }
    sh.event("restore", "Khôi phục hệ thống (app, service, Wi-Fi, Game DVR)");
    std::thread::spawn(move || {
        do_restore(&sh, false);
        sh.busy.store(false, Ordering::SeqCst);
    });
}

/// Bắt đầu Restore ở thread nền và GIỮ nhật ký. Dùng cho banner "phiên Game Boost bị bỏ lửng"
/// (người dùng cần đọc log của phiên trước) — khác nút Khôi phục thường là nút đó xoá log.
fn restore_keep_log(sh: Shared) {
    if sh.busy.swap(true, Ordering::SeqCst) {
        return;
    }
    sh.last_auto.store(false, Ordering::SeqCst);
    sh.event("restore", "Khôi phục phiên Game Boost bị bỏ lửng");
    std::thread::spawn(move || {
        do_restore(&sh, false);
        sh.busy.store(false, Ordering::SeqCst);
    });
}

/// Xử lý một phím tắt vừa bấm. Chạy trên **thread phím tắt** (`hotkey.rs`), không phải thread
/// UI — giống thread sự kiện của khay, vì eframe không gọi `update()` khi cửa sổ đã ẩn.
///
/// Boost/Restore làm đúng như mục menu khay tương ứng; "Hiện/ẩn cửa sổ" dùng `sys` như click
/// icon khay / thu vào khay (KHÔNG dùng `SW_HIDE` trực tiếp — xem ghi chú hiệu năng ở NOTES.md).
fn on_hotkey(sh: &Shared, ctx: &egui::Context, hwnd: isize, action: hotkey::HotkeyAction) {
    let combo = sh.hotkey_display(action);
    let busy = sh.busy.load(Ordering::SeqCst);
    match action {
        // `optimize`/`restore` tự chặn khi đang bận; gọi sau cùng của lần bấm trước vẫn đúng thứ tự.
        hotkey::HotkeyAction::Boost if !busy => optimize(sh.clone(), false),
        hotkey::HotkeyAction::Restore if !busy => restore(sh.clone(), false),
        hotkey::HotkeyAction::ToggleWindow => {
            let now_visible = sys::toggle_main_window(hwnd);
            sh.log(
                format!(
                    "  • [PHÍM TẮT] {} — cửa sổ {}.",
                    combo,
                    if now_visible { "hiện ra" } else { "thu vào khay" }
                ),
                if now_visible { ui::ACCENT } else { ui::TEXT_MUTED },
            );
            ctx.request_repaint();
            return;
        }
        _ => {}
    }
    // Ghi log SAU khi hành động chạy: `optimize`/`restore` xoá nhật ký ở đầu nên ghi trước sẽ mất.
    sh.log(
        format!(
            "  ⌨ [PHÍM TẮT] {}{}",
            combo,
            if busy { " — bỏ qua vì đang chạy tối ưu/khôi phục" } else { "" }
        ),
        if busy { ui::WARNING } else { ui::ACCENT },
    );
    ctx.request_repaint();
}

/// Thêm một app đang chạy vào danh sách game, lưu đường dẫn exe và (nếu bật) đặt GPU rời.
fn mark_as_game(sh: &Shared, app: &RunningApp) {
    // App hệ thống / app luôn chạy (WebView2, trình duyệt, explorer...) không bao giờ là game:
    // đưa nhầm vào đây làm Game Boost chạy ngay khi mở app và không bao giờ tự khôi phục.
    if is_not_game(&app.exe_lower) {
        sh.log(
            format!(
                "  ⚠ [GAME] Không thêm {} vào danh sách game: đây là app hệ thống/luôn chạy.",
                app.exe_lower
            ),
            Color32::from_rgb(255, 170, 40),
        );
        return;
    }
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
///
/// Hồ sơ riêng theo game: tự Boost do game X mở thì dùng hồ sơ X; Boost tay (nút, menu khay,
/// phím tắt) thì chỉ dùng hồ sơ khi ĐÚNG MỘT game trong danh sách đang chạy, còn lại dùng cài
/// đặt chung. Nhật ký đã được `optimize()` xoá trước khi thread này chạy (xem chú thích ở đó).
fn do_optimize(sh: &Shared) {
    let log = sh.log.clone();
    let push = |s: String, c: Color32| log.lock().unwrap().push((s, c));

    let keep = sh.keep.lock().unwrap().clone();
    let svcs = sh.svc.lock().unwrap().clone();
    let games: Vec<String> = sh.games.lock().unwrap().clone();
    let is_admin = sh.is_admin;

    // 0a. Hồ sơ riêng của game quyết định lần Boost này (nếu có).
    let from_auto = sh.last_auto.load(Ordering::SeqCst);
    let running = running_games(&games);
    let profile_game = if from_auto {
        running.first().cloned()
    } else if running.len() == 1 {
        running.first().cloned()
    } else {
        None
    };
    let profile = profile_game.as_ref().and_then(|exe| sh.profile_of(exe));
    let (opts, close_apps, stop_services) = effective_opts(&sh.opts(), profile.as_ref());

    let before_mem = sys::memory();
    let before = before_mem.avail;
    push(
        format!("▶ [DEPLOY] Bắt đầu thanh lọc | RAM khả dụng ban đầu: {:.1} GB", before as f64 / GB),
        Color32::from_rgb(0, 229, 255),
    );
    if let (Some(exe), Some(prof)) = (profile_game.as_ref(), profile.as_ref()) {
        push(
            profile_diff_line(&clean_game_name(exe), prof),
            Color32::from_rgb(0, 229, 255),
        );
    }

    let me = std::process::id();
    let my_session = sys::current_session();
    // Exe game + anti-cheat: chỉ lấy tên/đường dẫn, không mở quyền đọc bộ nhớ.
    let skip = skip_list(&games);
    let nodes = sys::process_nodes();
    // "Họ" process game (launcher / tổ tiên / con cháu) tính TRƯỚC khi quét sâu để con cháu
    // của game cũng chỉ được mở bằng quyền hạn chế, không đọc bộ nhớ.
    let family: HashSet<u32> = game_family_from_nodes(&nodes, &games)
        .into_keys()
        .collect();
    // Con cháu của app được giữ (`keep:` + danh sách mặc định): không tắt, không nén RAM,
    // không lưu vào session (vì không bị tắt nên Restore cũng không mở lại chúng).
    let kept_family = kept_family_from_nodes(&nodes, &kept_app_names(&nodes, &keep, &skip));
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
    //    Hồ sơ `close_apps=0` thì bỏ hẳn bước này: Boost không tắt app nào.
    let mut groups: HashMap<String, Vec<u32>> = HashMap::new();
    let mut labels: HashMap<String, String> = HashMap::new();
    let mut kept_exes: HashSet<String> = HashSet::new();
    if close_apps {
        for p in snap.iter() {
            if p.pid == 0 || p.pid == me || p.pid == parent || p.session != my_session {
                continue;
            }
            let exe = p.name.to_lowercase();
            if exe.is_empty() || is_never_kill(&exe, p.exe_path.as_deref()) {
                continue;
            }
            // Game + anti-cheat (ACE) + họ process game + con cháu của app được giữ: tuyệt đối
            // không tắt, không lưu session, không mở quyền đọc bộ nhớ.
            if keep_decision(&keep, &exe)
                || is_ace_or_game(&exe, &skip)
                || family.contains(&p.pid)
                || kept_family.contains_key(&p.pid)
            {
                kept_exes.insert(exe);
                continue;
            }
            groups.entry(exe.clone()).or_default().push(p.pid);
            labels
                .entry(exe)
                .or_insert_with(|| p.display_name.clone().unwrap_or_else(|| p.name.clone()));
        }
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
    // 1c. Con cháu của app được giữ (vd Claude mở node.exe / cua-driver.exe): cũng không tắt.
    if let Some(line) = kept_family_log_line(&kept_family, &nodes) {
        push(line, Color32::from_rgb(0, 255, 163));
    }
    push(
        if close_apps {
            format!(
                "  • [SCAN] {} app sẽ tắt / {} app được giữ lại (session user hiện tại).",
                groups.len(),
                kept_exes.len()
            )
        } else {
            "  • [HỒ SƠ] close_apps=0 — không tắt app nền nào lần này.".to_string()
        },
        Color32::from_rgb(50, 215, 255),
    );

    // 2. Gom "process gốc" của từng app để lưu session (TRƯỚC khi tắt).
    //    Cửa sổ phải đọc trước khi gửi WM_CLOSE, sau đó app đã đóng nên không còn gì để đọc.
    //    Không tắt app (hồ sơ close_apps=0) thì cũng KHÔNG lưu app nào vào session để Restore
    //    không mở lại app vốn chưa hề bị tắt.
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
                "  • [GPU] Chưa biết đường dẫn exe game — mở game rồi chuột phải vào game ở tab Ứng dụng đang chạy → 'Đánh dấu là game' để lưu vị trí.".into(),
                Color32::from_rgb(130, 150, 170),
            );
        }
    }

    // 4. Tạm dừng các Services được chọn (nếu có quyền Admin).
    //    Hồ sơ `stop_services=0` thì không dừng service nào (và không ghi service vào session).
    let mut newly_stopped = Vec::new();
    if !stop_services {
        push(
            "  • [HỒ SƠ] stop_services=0 — không dừng service nào lần này.".into(),
            Color32::from_rgb(130, 150, 170),
        );
    }
    for ((svc_name, display_name, _, _), on) in SERVICE_TARGETS.iter().zip(&svcs) {
        if !on || !stop_services {
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
            if close_apps {
                "  • [SKIP APPS] Không có app nền nào cần tắt (tất cả đang được giữ lại).".to_string()
            } else {
                "  • [HỒ SƠ] Giữ nguyên mọi app nền (hồ sơ của game này không tắt app).".to_string()
            },
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
    //     con cháu của app được giữ, process hệ thống nhạy cảm, pid <= 4.
    if opts.trim {
        let mut targets = 0usize;
        let mut total = 0u64;
        let mut hits: Vec<(u64, String)> = Vec::new();
        for p in sys::list_processes() {
            if p.pid <= 4 || p.pid == me || family.contains(&p.pid) || kept_family.contains_key(&p.pid)
            {
                continue;
            }
            let exe = p.name.to_lowercase();
            if exe.is_empty()
                || is_ace_or_game(&exe, &skip)
                || TRIM_SKIP_EXE.contains(&exe.as_str())
                || TRIM_SKIP_PREFIX.iter().any(|p| exe.starts_with(p))
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
        //     + họ process game + con cháu của app được giữ.
        let mut left: HashMap<String, u64> = HashMap::new();
        for p in sys::list_processes() {
            if p.pid <= 4 || p.pid == me || family.contains(&p.pid) || kept_family.contains_key(&p.pid)
            {
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
        "★ [READY] Đã tối ưu CPU & RAM cho game!".into(),
        Color32::from_rgb(0, 255, 163),
    );

    *sh.running.lock().unwrap() = scan_running_apps(&skip, &games, &keep);
}

/// Thân của Restore: mở lại app, bật lại service, Wi-Fi autoconf, Game DVR.
/// Trả `false` khi không có `session.json` (không có gì để khôi phục) — cờ `--restore` dùng
/// giá trị này làm exit code, UI bỏ qua.
fn do_restore(sh: &Shared, clear_log: bool) -> bool {
    if clear_log {
        log_clear(sh);
    }
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
        return false;
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
    // Phiên bỏ lửng đã được xử lý: watcher được phép tự Boost trở lại.
    sh.orphan_session.store(false, Ordering::SeqCst);

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
    true
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
/// Game trong danh sách đang chạy (trả về tên exe viết thường). Dùng lúc mở app để biết
/// `session.json` còn lại là "đang trong phiên" hay "phiên bị bỏ lửng".
fn running_games(games: &[String]) -> Vec<String> {
    if games.is_empty() {
        return Vec::new();
    }
    let names: Vec<String> = sys::list_processes()
        .into_iter()
        .map(|p| p.name.to_lowercase())
        .collect();
    games
        .iter()
        .filter(|g| names.iter().any(|n| n == *g))
        .cloned()
        .collect()
}

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
        // Phiên còn dở mà game vẫn đang chạy lúc mở app (app bị tắt giữa trận rồi mở lại):
        // coi như đang trong phiên -> không Boost lần nữa, game tắt thì tự khôi phục như thường.
        let mut optimized = sh.resume_session.swap(false, Ordering::SeqCst);
        // Đã báo một lần vì sao watcher không tự Boost khi có phiên bỏ lửng chưa xử lý.
        let mut orphan_block_logged = false;
        let mut last_mem_check = Instant::now();
        // Instant trên Windows tính từ lúc bật máy: `now() - X` panic khi máy mới chạy chưa tới X
        // (task chạy cùng Windows mở app ngay sau đăng nhập), nên lùi mốc bằng checked_sub.
        let mut last_purge = Instant::now().checked_sub(PURGE_GAP).unwrap_or_else(Instant::now);
        // Lỗi xả standby (thường do thiếu quyền admin) chỉ log một lần, không lặp mỗi phút.
        let mut purge_fail_logged = false;
        // Báo cáo trận đang ghi (None khi không có game nào chạy).
        let mut live: Option<Arc<Mutex<report::Live>>> = None;
        let mut last_commit_check = Instant::now().checked_sub(COMMIT_CHECK).unwrap_or_else(Instant::now);
        let mut last_commit_warn = Instant::now().checked_sub(COMMIT_WARN_GAP).unwrap_or_else(Instant::now);
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
            // Hồ sơ riêng của game đang chạy cũng áp cho việc ghi báo cáo trận và xả standby
            // trong lúc chơi (các tuỳ chọn khác chỉ dùng lúc Boost).
            let game_profile = up.first().and_then(|exe| sh.profile_of(exe));
            let (game_opts, _, _) = effective_opts(&opts, game_profile.as_ref());

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
                    if any && game_opts.standby && !commit_purged && !sh.busy.load(Ordering::SeqCst) {
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
                if live.is_none() && game_opts.report_enabled {
                    let host = sh.ping_host.lock().unwrap().clone();
                    let started = report::start(
                        &up.join(", "),
                        &up,
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
                    // Có phiên Game Boost bỏ lửng chưa xử lý: KHÔNG tự Boost (Boost sẽ ghi
                    // thêm vào session cũ và người dùng càng khó khôi phục đúng trạng thái).
                    if sh.orphan_session.load(Ordering::SeqCst) {
                        if !orphan_block_logged {
                            orphan_block_logged = true;
                            sh.log(
                                "  ⚠ [AUTO] Còn phiên Game Boost chưa khôi phục — không tự tối ưu. Bấm 'Khôi phục ngay' trên banner hoặc chạy GameMode.exe --restore.".into(),
                                Color32::from_rgb(255, 170, 40),
                            );
                        }
                    } else {
                        let (since, count_at_seen) = *game_seen.get_or_insert_with(|| {
                            (Instant::now(), sh.opt_count.load(Ordering::SeqCst))
                        });
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
                        restore(sh.clone(), true);
                    } else if !sh.busy.load(Ordering::SeqCst) {
                        optimized = false;
                    }
                }
            }

            // Trong phiên game: RAM trống thấp mà standby nhiều -> xả, tối thiểu 60 giây một lần.
            if any
                && game_opts.standby
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
                                if !purge_fail_logged {
                                    purge_fail_logged = true;
                                    let hint = if sys::is_admin() {
                                        ""
                                    } else {
                                        " — Paroda GameMode đang chạy quyền thường, thoát ở khay rồi mở lại bằng quyền admin"
                                    };
                                    sh.log(
                                        format!("  ⚠ [AUTO] Không xả được standby khi đang chơi: {}{}", e, hint),
                                        Color32::from_rgb(255, 120, 120),
                                    );
                                }
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

/// Việc chạy ở thread nền cho tab "Tối ưu Win" (đọc trạng thái tốn `powercfg`/`bcdedit`/PowerShell).
#[derive(Clone)]
enum TweakJob {
    /// Chỉ đọc lại trạng thái của mọi tinh chỉnh (mở tab lần đầu / Làm mới / sau khi bật-hoàn tác).
    Refresh,
    /// Bật các tinh chỉnh theo chỉ số trong `App::tweaks` (tuần tự).
    Apply(Vec<usize>),
    /// Hoàn tác các tinh chỉnh theo chỉ số (tuần tự).
    Revert(Vec<usize>),
}

/// Kết quả một việc của tab "Tối ưu Win" gửi từ thread nền về UI.
struct TweakOutcome {
    job: TweakJob,
    /// Dòng nhật ký (nội dung, màu) để đưa vào log của app.
    lines: Vec<(String, Color32)>,
    /// Trạng thái đọc lại của mọi tinh chỉnh (đúng thứ tự `App::tweaks`).
    states: Vec<wintweaks::TweakState>,
    /// Bản sao lưu sau thao tác (đã ghi ra file).
    backup: wintweaks::Backup,
    /// Tiêu đề các tinh chỉnh cần khởi động lại vừa bị đổi trong lần này.
    reboot: Vec<String>,
    /// Lỗi khi ghi file sao lưu (nếu có).
    save_err: Option<String>,
}

/// Thời gian chờ bấm lần thứ hai của nút cần xác nhận (gói khuyến nghị / hoàn tác tất cả / Bật
/// một tinh chỉnh rủi ro "Nâng cao").
const CONFIRM_WINDOW: Duration = Duration::from_secs(4);

/// Thứ tự nhóm tinh chỉnh trên tab "Tối ưu Win".
const TWEAK_GROUPS: [wintweaks::Group; 4] = [
    wintweaks::Group::Game,
    wintweaks::Group::Background,
    wintweaks::Group::Memory,
    wintweaks::Group::System,
];

/// Vạch ngăn mờ giữa hai hàng tinh chỉnh.
fn hairline(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, Rounding::ZERO, ui::GRID);
}

/// Mô tả một thao tác của tinh chỉnh để hiện trong tooltip (service kèm kiểu khởi động mới).
fn action_desc(a: &wintweaks::Action) -> String {
    match a {
        wintweaks::Action::ServiceStart { name, start } => {
            format!("service {} → {}", name, start.label())
        }
        _ => a.target(),
    }
}

/// Tooltip của tiêu đề một hàng tinh chỉnh: nhóm · rủi ro · khởi động lại, rồi từng thao tác.
fn tweak_tooltip(t: &wintweaks::Tweak) -> String {
    let actions: Vec<String> = t.actions().iter().map(action_desc).collect();
    format!(
        "{}\n{} thao tác:\n{}",
        t.subtitle(),
        t.action_count(),
        actions.join("\n")
    )
}

/// Một hàng tinh chỉnh: tiêu đề + badge rủi ro (+ khởi động lại / có sao lưu) + mô tả + trạng
/// thái, bên phải là nút **Bật** (khi chưa bật) và **Hoàn tác** (khi có trong bản sao lưu).
/// Trả về `(bấm Bật, bấm Hoàn tác)` để nơi gọi quyết định (xác nhận 2 lần / chạy thread nền).
fn tweak_row_ui(
    ui: &mut egui::Ui,
    t: &wintweaks::Tweak,
    state: &wintweaks::TweakState,
    in_backup: bool,
    busy: bool,
    confirming: bool,
) -> (bool, bool) {
    let (state_text, state_color, why) = match state {
        wintweaks::TweakState::Applied => ("Đang bật", ui::SUCCESS, None),
        wintweaks::TweakState::NotApplied => ("Chưa bật", ui::TEXT_MUTED, None),
        wintweaks::TweakState::Partial => ("Một phần", ui::WARNING, None),
        wintweaks::TweakState::Unknown(why) => ("Không rõ", ui::TEXT_DIM, Some(why.clone())),
    };
    let applied = state.is_applied();
    let mut click_on = false;
    let mut click_off = false;

    ui.horizontal(|ui| {
        // Chừa chỗ cho hai nút bên phải để chữ mô tả không bị đè (nút xác nhận dài hơn nút "Bật").
        let text_w = (ui.available_width() - 240.0).max(140.0);
        ui.vertical(|ui| {
            ui.set_max_width(text_w);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(
                    RichText::new(t.title)
                        .size(11.5)
                        .strong()
                        .color(ui::TEXT_MAIN),
                )
                .on_hover_text(tweak_tooltip(t));
                let risk_color = match t.risk {
                    wintweaks::Risk::Safe => ui::SUCCESS,
                    wintweaks::Risk::Consider => ui::WARNING,
                    wintweaks::Risk::Advanced => ui::DANGER,
                };
                ui::badge(ui, t.risk.label(), risk_color);
                if t.needs_reboot {
                    ui::badge(ui, "Khởi động lại", ui::ACCENT);
                }
                if in_backup {
                    ui::badge(ui, "có sao lưu", ui::DATA_CYAN);
                }
            });
            ui.label(RichText::new(t.desc).size(10.5).color(ui::TEXT_MUTED));
            ui.horizontal(|ui| {
                ui::dot(ui, state_color);
                let resp = ui.label(
                    RichText::new(state_text)
                        .size(10.5)
                        .strong()
                        .color(state_color),
                );
                if let Some(w) = why {
                    resp.on_hover_text(w);
                }
            });
        });

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if ui
                .add_enabled(
                    !busy && !applied,
                    ui::primary_button_sized(
                        if confirming { "Bấm lại để xác nhận" } else { "Bật" },
                        11.0,
                    ),
                )
                .clicked()
            {
                click_on = true;
            }
            if in_backup
                && ui
                    .add_enabled(!busy, ui::ghost_button_sized("Hoàn tác", 11.0))
                    .clicked()
            {
                click_off = true;
            }
        });
    });

    (click_on, click_off)
}

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
    /// Lỗi khi bấm cài cập nhật (đang chơi game / đang Game Boost...) để hiện trong card hoặc banner.
    update_err: Option<String>,
    /// Phiên bản cập nhật mà người dùng đã bấm "Để sau" (chỉ nhớ trong phiên, không ghi config).
    update_dismissed_version: Option<String>,
    /// Mở/đóng vùng xem changelog trong banner cập nhật.
    update_changelog_open: bool,
    /// Đã bấm Cập nhật (đang trong luồng tải / cài / thử lại) — để phân biệt với lỗi kiểm tra mạng định kỳ.
    update_download_started: bool,
    /// Banner "phiên Game Boost bỏ lửng" đã bấm "Để sau" (chỉ nhớ trong lần mở này).
    orphan_dismissed: bool,
    /// Phím tắt toàn cục đang chạy (`None` khi `hotkey:enabled=0` hoặc không đăng ký được).
    hotkeys: Option<hotkey::HotkeyService>,
    /// Lỗi parse tổ hợp phím tắt đọc từ config (hiện đỏ trong card "Phím tắt").
    hotkey_parse_errors: Vec<(hotkey::HotkeyAction, String)>,
    /// Ô nhập tổ hợp của mỗi hành động (chuỗi thô trong config; rỗng = phím đang tắt).
    hotkey_inputs: HashMap<hotkey::HotkeyAction, String>,
    /// Lỗi parse của lần bấm "Lưu" gần nhất trong phiên này (hành động -> câu lỗi).
    hotkey_input_errors: HashMap<hotkey::HotkeyAction, String>,
    /// Game đang mở khối "Hồ sơ riêng" trong card danh sách game.
    open_profile: Option<String>,
    /// Danh sách mục khởi động cùng Windows (đọc ở thread nền khi mở tab lần đầu / bấm Làm mới).
    startup_view: Vec<startup::StartupEntry>,
    /// Kết quả đọc ở thread nền (`None` = chưa xong); `update()` lấy ra đưa vào `startup_view`.
    startup_slot: Arc<Mutex<Option<Vec<startup::StartupEntry>>>>,
    startup_busy: Arc<AtomicBool>,
    /// Đã đọc xong danh sách khởi động lần nào chưa (lần mở tab đầu tiên tự đọc).
    startup_loaded: bool,
    /// Lỗi ghi cờ bật/tắt của từng mục, hiện ngay dưới dòng đó (theo tên mục).
    startup_row_err: HashMap<String, String>,
    /// Lỗi chung khi đọc danh sách mục khởi động.
    startup_err: Option<String>,
    /// Số liệu phần cứng đọc ở thread nền (`hwinfo::collect`, chặn vài giây vì WMI).
    hw_info: Option<hwinfo::HwInfo>,
    /// Kết quả thread nền (`None` = chưa xong); `update()` lấy ra tính `summary_line` + `advice`.
    hw_slot: Arc<Mutex<Option<Result<hwinfo::HwInfo, String>>>>,
    hw_busy: Arc<AtomicBool>,
    /// Đã quét phần cứng lần nào chưa (mở tab lần đầu tự quét).
    hw_loaded: bool,
    /// Gợi ý đã tính sẵn từ `hw_info` (tính lại khi quét xong).
    hw_advice: Vec<hwinfo::Advice>,
    /// Dòng tóm tắt phần cứng (`hwinfo::summary_line`).
    hw_summary: String,
    /// Lỗi quét phần cứng, hiện đỏ trong card "Phần cứng".
    hw_err: Option<String>,
    /// Danh sách tinh chỉnh Windows (`wintweaks::all_tweaks`), dựng một lần khi mở tab lần đầu.
    tweaks: Vec<wintweaks::Tweak>,
    /// Trạng thái hiện tại của từng tinh chỉnh (đúng thứ tự `tweaks`; đọc ở thread nền).
    tweak_states: Vec<wintweaks::TweakState>,
    /// `None` = chưa có việc nào xong; `Some` = kết quả một việc bật/hoàn tác/đọc lại.
    tweak_slot: Arc<Mutex<Option<TweakOutcome>>>,
    /// Có việc đang chạy ở thread nền (khoá mọi nút của tab khi bật).
    tweak_busy: Arc<AtomicBool>,
    /// Đã dựng danh sách tinh chỉnh + đọc trạng thái lần nào chưa.
    tweak_loaded: bool,
    /// Bản sao lưu giá trị gốc (`wintweaks-backup.json`) — nguồn sự thật cho nút Hoàn tác.
    tweak_backup: wintweaks::Backup,
    /// Đã đọc file sao lưu từ đĩa lần nào chưa.
    tweak_backup_loaded: bool,
    /// Lỗi ghi file sao lưu / lỗi chung của card tinh chỉnh.
    tweak_err: Option<String>,
    /// Tinh chỉnh cần khởi động lại đã bị đổi trong lần mở này (hiện callout vàng).
    reboot_pending: Vec<String>,
    /// Lần bấm đầu của nút "Áp dụng gói khuyến nghị" / "Hoàn tác tất cả" (bấm 2 lần, tự huỷ 4 giây).
    confirm_apply_all: Option<Instant>,
    confirm_revert_all: Option<Instant>,
    /// Lần bấm đầu của nút "Bật" ở hàng rủi ro `Advanced`: (id tinh chỉnh, lúc bấm).
    confirm_row: Option<(String, Instant)>,
    /// `egui::Context` giữ lại để tạo dịch vụ phím tắt sau này (khi bật lại từ UI).
    ctx: egui::Context,
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

        let mut boot_log: Vec<(String, Color32)> = vec![
            ("[SYSTEM] PARODA GAMEMODE READY".into(), Color32::from_rgb(0, 229, 255)),
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
        ];
        // Danh sách game trong config bị bỏ qua vì là app hệ thống / app luôn chạy (bản cũ từng
        // có `game:msedgewebview2.exe=1` -> Boost ngay khi mở app): báo một lần cho người dùng biết.
        for exe in cfg.skipped_games.iter() {
            boot_log.push((
                format!("  ⚠ Bỏ qua {exe} trong danh sách game: đây là app hệ thống/luôn chạy."),
                Color32::from_rgb(255, 170, 40),
            ));
        }
        // `session.json` còn từ lần chạy trước. Hai trường hợp:
        //  - game trong danh sách vẫn đang chạy -> vẫn đang trong phiên: watcher tiếp quản phiên
        //    cũ (không Boost lần nữa), khi game tắt thì tự khôi phục nếu `auto:restore`.
        //  - không có game nào chạy -> phiên bị bỏ lửng: hiện banner "Khôi phục ngay".
        let resume_games = if session.is_some() {
            running_games(&cfg.games)
        } else {
            Vec::new()
        };
        let orphan = session.is_some() && resume_games.is_empty();
        if !resume_games.is_empty() {
            boot_log.push((
                format!(
                    "  • [PHIÊN] {} đang chạy và session.json còn — tiếp tục phiên Game Boost cũ, \
                     không tối ưu lần nữa; game tắt sẽ tự khôi phục (nếu bật auto:restore).",
                    resume_games.join(", ")
                ),
                Color32::from_rgb(0, 229, 255),
            ));
        }
        if orphan {
            boot_log.push((
                "  ⚠ [PHIÊN] Còn phiên Game Boost chưa khôi phục (không thấy game nào đang chạy) — \
                 xem banner ở cột phải để Khôi phục ngay."
                    .into(),
                Color32::from_rgb(255, 170, 40),
            ));
        }
        // Vừa cài bản cập nhật ở lần chạy trước (`update-pending.json`): báo kết quả trong nhật ký.
        if let Some((line, ok)) = update::take_startup_log() {
            boot_log.push((line, if ok { ui::SUCCESS } else { ui::DANGER }));
        }
        // Trận bị bỏ dở vì lần chạy trước tắt giữa trận (Thoát ở khay, khởi động lại, mất điện,
        // crash): dựng lại báo cáo chính từ bản lưu tạm rồi báo trong nhật ký.
        for line in report::recover_partials() {
            boot_log.push((line, Color32::from_rgb(0, 229, 255)));
        }

        let sh = Shared {
            log: Arc::new(Mutex::new(boot_log)),
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
            // Phiên cũ tiếp tục: auto-restore phải chạy như một phiên auto bình thường.
            last_auto: Arc::new(AtomicBool::new(!resume_games.is_empty())),
            opt_count: Arc::new(AtomicUsize::new(0)),
            live_report: Arc::new(Mutex::new(None)),
            ping_host: Arc::new(Mutex::new(cfg.ping_host.clone())),
            commit_alert: Arc::new(AtomicBool::new(false)),
            commit_pct: Arc::new(Mutex::new(0.0)),
            update_state: Arc::new(Mutex::new(update::UpdateState::Idle)),
            update_manifest: Arc::new(Mutex::new(None)),
            update_last_check: Arc::new(Mutex::new(None)),
            update_request: Arc::new(AtomicBool::new(false)),
            update_url: Arc::new(Mutex::new(cfg.update_url.clone())),
            orphan_session: Arc::new(AtomicBool::new(orphan)),
            resume_session: Arc::new(AtomicBool::new(!resume_games.is_empty())),
            profiles: Arc::new(Mutex::new(cfg.profiles.clone())),
            hotkeys: Arc::new(Mutex::new(cfg.hotkeys.clone())),
            hotkey_bindings: Arc::new(Mutex::new(Vec::new())),
        };
        let hotkeys_view = cfg.hotkeys.clone();

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
            // Có phiên Boost bỏ lửng thì LUÔN hiện cửa sổ, kể cả khi task chạy cùng Windows gọi
            // với `--minimized` (giống cách xử lý `update-pending.json`).
            start_hidden: START_HIDDEN.load(Ordering::SeqCst) && !orphan,
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
            update_err: None,
            update_dismissed_version: None,
            update_changelog_open: false,
            update_download_started: false,
            orphan_dismissed: false,
            hotkeys: None,
            hotkey_parse_errors: Vec::new(),
            hotkey_inputs: hotkeys_view.clone(),
            hotkey_input_errors: HashMap::new(),
            open_profile: None,
            startup_view: Vec::new(),
            startup_slot: Arc::new(Mutex::new(None)),
            startup_busy: Arc::new(AtomicBool::new(false)),
            startup_loaded: false,
            startup_row_err: HashMap::new(),
            startup_err: None,
            hw_slot: Arc::new(Mutex::new(None)),
            hw_busy: Arc::new(AtomicBool::new(false)),
            hw_loaded: false,
            hw_info: None,
            hw_advice: Vec::new(),
            hw_summary: String::new(),
            hw_err: None,
            tweaks: Vec::new(),
            tweak_states: Vec::new(),
            tweak_slot: Arc::new(Mutex::new(None)),
            tweak_busy: Arc::new(AtomicBool::new(false)),
            tweak_loaded: false,
            tweak_backup: wintweaks::Backup::new(),
            tweak_backup_loaded: false,
            tweak_err: None,
            reboot_pending: Vec::new(),
            confirm_apply_all: None,
            confirm_revert_all: None,
            confirm_row: None,
            ctx: cc.egui_ctx.clone(),
        };
        app.save();
        app.start_scan();
        // Watcher sống độc lập với UI: vẫn tự tối ưu khi cửa sổ bị thu nhỏ hoặc ẩn xuống khay.
        start_watcher(sh.clone());
        // Kiểm tra cập nhật OTA: 20 giây sau khi mở app rồi mỗi 6 giờ (thread riêng).
        update::start_update_thread(sh.clone());
        // Task "GameMode" cũ (mặc định `/SC ONLOGON`: chỉ chạy khi cắm sạc, tự chết sau 72 giờ)
        // hoặc trỏ sai exe: kiểm tra và tạo lại ở thread nền để không chặn UI.
        repair_task_in_background(sh.clone());
        // Phím tắt toàn cục: thread riêng (như khay) vì eframe không gọi `update()` khi cửa sổ ẩn.
        if sh.opts().hotkey_enabled {
            let (bindings, errs) = parse_hotkeys(&hotkeys_view);
            *sh.hotkey_bindings.lock().unwrap() = bindings.clone();
            app.hotkey_parse_errors = errs;
            let sh_hk = sh.clone();
            let ctx_hk = cc.egui_ctx.clone();
            let hwnd_hk = app.hwnd;
            app.hotkeys = Some(hotkey::HotkeyService::start(bindings, move |action| {
                on_hotkey(&sh_hk, &ctx_hk, hwnd_hk, action)
            }));
            let list: Vec<String> = HOTKEY_ACTIONS
                .iter()
                .filter_map(|a| {
                    let combo = sh.hotkey_display(*a);
                    (!combo.is_empty()).then(|| format!("{} = {}", a.label(), combo))
                })
                .collect();
            sh.log(
                if list.is_empty() {
                    "  • [PHÍM TẮT] Mọi tổ hợp đang tắt (đặt lại trong tab Tự động & Tối ưu).".into()
                } else {
                    format!("  • [PHÍM TẮT] Toàn cục: {}", list.join(", "))
                },
                ui::ACCENT,
            );
        } else {
            sh.log(
                "  • [PHÍM TẮT] Đang tắt (hotkey:enabled=0).".into(),
                ui::TEXT_MUTED,
            );
        }
        for (action, err) in app.hotkey_parse_errors.clone() {
            sh.log(
                format!("  ⚠ [PHÍM TẮT] {}: {}", action.label(), err),
                ui::DANGER,
            );
        }
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

    /// Bắt đầu đọc danh sách mục khởi động cùng Windows ở thread nền (mở tab lần đầu / Làm mới).
    fn start_startup_scan(&mut self) {
        if self.startup_busy.swap(true, Ordering::SeqCst) {
            return;
        }
        let slot = self.startup_slot.clone();
        let busy = self.startup_busy.clone();
        std::thread::spawn(move || {
            let list = startup::list_startup();
            *slot.lock().unwrap() = Some(list);
            busy.store(false, Ordering::SeqCst);
        });
    }

    /// Lấy kết quả đọc xong của `startup::list_startup` vào `startup_view` (gọi mỗi frame).
    fn poll_startup(&mut self) {
        let done = self.startup_slot.lock().unwrap().take();
        if let Some(list) = done {
            self.startup_view = list;
            self.startup_loaded = true;
        }
    }

    // =================================================================================
    // Tab "Tối ưu Win": card Phần cứng (hwinfo) + card Tinh chỉnh Windows (wintweaks)
    // =================================================================================

    /// Quét phần cứng ở thread nền (`hwinfo::collect` chặn vài giây vì WMI/PowerShell).
    fn start_hw_scan(&mut self) {
        if self.hw_busy.swap(true, Ordering::SeqCst) {
            return;
        }
        let slot = self.hw_slot.clone();
        let busy = self.hw_busy.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let res = hwinfo::collect();
            *slot.lock().unwrap() = Some(res);
            busy.store(false, Ordering::SeqCst);
            ctx.request_repaint();
        });
    }

    /// Lấy kết quả quét phần cứng (gọi mỗi frame): tính sẵn dòng tóm tắt + danh sách gợi ý.
    fn poll_hw(&mut self) {
        let done = self.hw_slot.lock().unwrap().take();
        let Some(res) = done else {
            return;
        };
        self.hw_loaded = true;
        match res {
            Ok(info) => {
                self.hw_summary = hwinfo::summary_line(&info);
                self.hw_advice = hwinfo::advice(&info);
                self.hw_info = Some(info);
                self.hw_err = None;
            }
            Err(e) => {
                self.hw_advice.clear();
                self.hw_summary.clear();
                self.hw_info = None;
                self.hw_err = Some(e);
            }
        }
    }

    /// Dựng danh sách tinh chỉnh (một lần cho mỗi lần mở app) + đọc file sao lưu, rồi đọc
    /// trạng thái ở thread nền. `game_dirs` lấy từ `gamepath:` của config.
    fn ensure_tweaks(&mut self) {
        if self.tweaks.is_empty() {
            let dirs = game_dirs_from_paths(&self.sh.game_paths.lock().unwrap());
            self.tweaks = wintweaks::all_tweaks(&dirs);
            self.tweak_states = vec![
                wintweaks::TweakState::Unknown("Đang đọc trạng thái…".to_string());
                self.tweaks.len()
            ];
        }
        if !self.tweak_backup_loaded {
            self.tweak_backup = wintweaks::Backup::load(&wintweaks_backup_path());
            self.tweak_backup_loaded = true;
        }
        if !self.tweak_loaded {
            self.start_tweak_job(TweakJob::Refresh);
        }
    }

    /// Chạy một việc của tab Tối ưu Win ở thread nền (bật / hoàn tác / đọc lại trạng thái).
    /// Mọi thao tác làm TUẦN TỰ và lưu file sao lưu ngay sau mỗi tinh chỉnh.
    fn start_tweak_job(&mut self, job: TweakJob) {
        if self.tweak_busy.swap(true, Ordering::SeqCst) {
            return;
        }
        let tweaks = self.tweaks.clone();
        let mut backup = self.tweak_backup.clone();
        let path = wintweaks_backup_path();
        let slot = self.tweak_slot.clone();
        let busy = self.tweak_busy.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let mut lines: Vec<(String, Color32)> = Vec::new();
            let mut reboot: Vec<String> = Vec::new();
            let mut save_err: Option<String> = None;
            let note_save = |backup: &wintweaks::Backup, save_err: &mut Option<String>, lines: &mut Vec<(String, Color32)>| {
                if let Err(e) = backup.save(&path) {
                    lines.push((format!("  ⚠ [WIN] Không ghi được bản sao lưu: {}", e), ui::DANGER));
                    *save_err = Some(e);
                }
            };

            match &job {
                TweakJob::Apply(ids) => {
                    for i in ids {
                        let Some(t) = tweaks.get(*i) else { continue };
                        match wintweaks::apply(t, &mut backup) {
                            Ok(()) => {
                                lines.push((format!("  ✔ [WIN] Đã bật: {}", t.title), ui::SUCCESS));
                                if t.needs_reboot && !reboot.contains(&t.title.to_string()) {
                                    reboot.push(t.title.to_string());
                                }
                                note_save(&backup, &mut save_err, &mut lines);
                            }
                            Err(e) => lines.push((
                                format!("  ⚠ [WIN] {}: {}", t.title, e),
                                ui::DANGER,
                            )),
                        }
                    }
                }
                TweakJob::Revert(ids) => {
                    for i in ids {
                        let Some(t) = tweaks.get(*i) else { continue };
                        if !backup.has(t.id) {
                            continue;
                        }
                        match wintweaks::revert(t, &mut backup) {
                            Ok(()) => {
                                lines.push((
                                    format!("  ↺ [WIN] Đã hoàn tác: {}", t.title),
                                    ui::WARNING,
                                ));
                                if t.needs_reboot && !reboot.contains(&t.title.to_string()) {
                                    reboot.push(t.title.to_string());
                                }
                                note_save(&backup, &mut save_err, &mut lines);
                            }
                            Err(e) => lines.push((
                                format!("  ⚠ [WIN] {}: {}", t.title, e),
                                ui::DANGER,
                            )),
                        }
                    }
                }
                TweakJob::Refresh => {}
            }

            let states: Vec<wintweaks::TweakState> =
                tweaks.iter().map(wintweaks::state).collect();
            *slot.lock().unwrap() = Some(TweakOutcome {
                job,
                lines,
                states,
                backup,
                reboot,
                save_err,
            });
            busy.store(false, Ordering::SeqCst);
            ctx.request_repaint();
        });
    }

    /// Lấy kết quả việc chạy xong ở thread nền (gọi mỗi frame): trạng thái mới + log + sao lưu.
    fn poll_tweaks(&mut self) {
        let done = self.tweak_slot.lock().unwrap().take();
        let Some(o) = done else {
            return;
        };
        self.tweak_states = o.states;
        self.tweak_backup = o.backup;
        self.tweak_loaded = true;
        // Thông báo lỗi ghi sao lưu chỉ được thay khi việc vừa xong là bật/hoàn tác;
        // lần đọc lại trạng thái (Refresh) không xoá thông báo cũ.
        if matches!(o.job, TweakJob::Apply(_) | TweakJob::Revert(_)) {
            self.tweak_err = o.save_err.map(|e| {
                format!(
                    "Không ghi được bản sao lưu {}: {}",
                    wintweaks_backup_path().display(),
                    e
                )
            });
        }
        for (text, color) in o.lines {
            self.sh.log(text, color);
        }
        for title in o.reboot {
            if !self.reboot_pending.contains(&title) {
                self.reboot_pending.push(title);
            }
        }
    }

    /// Tab "🔧 Tối ưu Win": card Phần cứng trên cùng, card Tinh chỉnh Windows bên dưới.
    fn render_winopt_tab(&mut self, ui: &mut egui::Ui) {
        // Bề rộng lấy TRƯỚC khi vẽ: card full width trong ScrollArea không được lấn mép phải.
        let full_w = ui.available_width();
        egui::ScrollArea::vertical()
            .id_salt("winopt_scroll_area")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.allocate_ui_with_layout(
                    vec2(full_w, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_max_width(full_w);
                        self.render_hw_card(ui);
                        ui.add_space(ui::SPACE_CARD_GAP);
                        self.render_tweaks_card(ui);
                    },
                );
            });
    }

    /// Card "Phần cứng": dòng tóm tắt + gợi ý (chấm màu theo mức) + nút Quét lại.
    fn render_hw_card(&mut self, ui: &mut egui::Ui) {
        let loading = self.hw_busy.load(Ordering::SeqCst);
        let mut rescan = false;

        ui::card(ui, |ui| {
            ui::section_header(
                ui,
                "PC",
                "Phần cứng",
                Some("Chỉ đọc — không thay đổi gì trên máy"),
            );
            ui.add_space(ui::SPACE_HEADER_GAP);

            if let Some(err) = self.hw_err.clone() {
                ui.label(
                    RichText::new(format!("⚠ {}", err))
                        .size(11.0)
                        .color(ui::DANGER),
                );
                ui.add_space(6.0);
            }

            match self.hw_info.as_ref() {
                None => {
                    ui.label(
                        RichText::new(if loading {
                            "Đang đọc thông tin phần cứng…"
                        } else {
                            "Chưa có số liệu phần cứng."
                        })
                        .size(11.5)
                        .color(ui::TEXT_MUTED),
                    );
                }
                Some(_) => {
                    if !self.hw_summary.is_empty() {
                        ui.label(
                            RichText::new(&self.hw_summary)
                                .size(11.5)
                                .strong()
                                .color(ui::TEXT_MAIN),
                        );
                        ui.add_space(8.0);
                    }
                    if self.hw_advice.is_empty() {
                        ui.label(
                            RichText::new("Không có gì đáng lưu ý về phần cứng.")
                                .size(11.0)
                                .color(ui::TEXT_MUTED),
                        );
                    }
                    for a in self.hw_advice.iter() {
                        let color = match a.level {
                            hwinfo::Level::Good => ui::SUCCESS,
                            hwinfo::Level::Info => ui::ACCENT,
                            hwinfo::Level::Warn => ui::WARNING,
                        };
                        ui.horizontal(|ui| {
                            ui::dot(ui, color);
                            ui.vertical(|ui| {
                                ui.label(
                                    RichText::new(&a.title)
                                        .size(11.5)
                                        .strong()
                                        .color(ui::TEXT_MAIN),
                                );
                                ui.label(
                                    RichText::new(&a.detail).size(10.5).color(ui::TEXT_MUTED),
                                );
                            });
                        });
                        ui.add_space(5.0);
                    }
                }
            }

            ui.add_space(4.0);
            if ui
                .add_enabled(!loading, ui::ghost_button_sized("🔄 Quét lại", 11.0))
                .clicked()
            {
                rescan = true;
            }
            if loading {
                ui.label(
                    RichText::new("Đang đọc…")
                        .size(10.0)
                        .color(ui::TEXT_DIM),
                );
            }
        });

        if rescan {
            self.start_hw_scan();
        }
    }

    /// Card "Tinh chỉnh Windows": hai nút gói (bấm 2 lần để xác nhận) rồi từng nhóm tinh chỉnh.
    fn render_tweaks_card(&mut self, ui: &mut egui::Ui) {
        let busy = self.tweak_busy.load(Ordering::SeqCst);
        let now = Instant::now();

        // Xác nhận 2 lần tự huỷ sau 4 giây.
        if self
            .confirm_apply_all
            .is_some_and(|t| now.duration_since(t) > CONFIRM_WINDOW)
        {
            self.confirm_apply_all = None;
        }
        if self
            .confirm_revert_all
            .is_some_and(|t| now.duration_since(t) > CONFIRM_WINDOW)
        {
            self.confirm_revert_all = None;
        }
        if self
            .confirm_row
            .as_ref()
            .is_some_and(|(_, t)| now.duration_since(*t) > CONFIRM_WINDOW)
        {
            self.confirm_row = None;
        }

        let total = self.tweaks.len();
        let (applied, _) = wintweaks::summary(&self.tweaks, &self.tweak_states);
        let rec_ids = wintweaks::recommended_ids(&self.tweaks);
        let recommended: Vec<usize> = self
            .tweaks
            .iter()
            .enumerate()
            .filter(|(i, t)| {
                rec_ids.contains(&t.id)
                    && !self
                        .tweak_states
                        .get(*i)
                        .is_some_and(|s| s.is_applied())
            })
            .map(|(i, _)| i)
            .collect();
        let backed: Vec<usize> = self
            .tweaks
            .iter()
            .enumerate()
            .filter(|(_, t)| self.tweak_backup.has(t.id))
            .map(|(i, _)| i)
            .collect();

        let mut confirm_apply_all = self.confirm_apply_all;
        let mut confirm_revert_all = self.confirm_revert_all;
        let mut confirm_row = self.confirm_row.clone();
        let mut job: Option<TweakJob> = None;

        {
            let tweaks = &self.tweaks;
            let states = &self.tweak_states;
            let backup = &self.tweak_backup;
            let reboot = &self.reboot_pending;
            let err = self.tweak_err.clone();
            let loaded = self.tweak_loaded;
            let rec_count = recommended.len();
            let backed_count = backed.len();

            ui::card(ui, |ui| {
                ui::section_header(
                    ui,
                    "WIN",
                    "Tinh chỉnh Windows",
                    Some("Đổi Windows lâu dài (không chỉ lúc chơi). Mọi thay đổi được sao lưu và hoàn tác được."),
                );
                ui.add_space(ui::SPACE_HEADER_GAP);

                if !reboot.is_empty() {
                    ui::callout_warning(
                        ui,
                        &format!(
                            "Cần khởi động lại máy để có hiệu lực: {}",
                            reboot.join(", ")
                        ),
                    );
                    ui.add_space(8.0);
                }

                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = ui::SPACE_TOOLBAR_GAP;
                    ui.label(
                        RichText::new(if total == 0 {
                            "Đang dựng danh sách tinh chỉnh…".to_string()
                        } else {
                            format!("{}/{} đang bật", applied, total)
                        })
                        .size(11.5)
                        .color(ui::TEXT_MAIN),
                    );
                    ui.add_space(ui::SPACE_TOOLBAR_GAP);

                    let apply_label = if confirm_apply_all.is_some() {
                        "Bấm lại để xác nhận"
                    } else {
                        "✨ Áp dụng gói khuyến nghị"
                    };
                    if ui
                        .add_enabled(
                            !busy && rec_count > 0,
                            ui::primary_button_sized(apply_label, 11.5),
                        )
                        .on_hover_text(format!("{} tinh chỉnh khuyến nghị chưa bật", rec_count))
                        .clicked()
                    {
                        if confirm_apply_all.is_some() {
                            confirm_apply_all = None;
                            confirm_row = None;
                            job = Some(TweakJob::Apply(recommended.clone()));
                        } else {
                            confirm_apply_all = Some(Instant::now());
                        }
                    }

                    let revert_label = if confirm_revert_all.is_some() {
                        "Bấm lại để xác nhận"
                    } else {
                        "↺ Hoàn tác tất cả"
                    };
                    if ui
                        .add_enabled(
                            !busy && backed_count > 0,
                            ui::ghost_button_sized(revert_label, 11.5),
                        )
                        .on_hover_text(format!("{} tinh chỉnh đang có bản sao lưu", backed_count))
                        .clicked()
                    {
                        if confirm_revert_all.is_some() {
                            confirm_revert_all = None;
                            confirm_row = None;
                            job = Some(TweakJob::Revert(backed.clone()));
                        } else {
                            confirm_revert_all = Some(Instant::now());
                        }
                    }

                    if ui
                        .add_enabled(!busy, ui::ghost_button_sized("🔄 Làm mới", 11.0))
                        .clicked()
                    {
                        job = Some(TweakJob::Refresh);
                    }
                });

                if let Some(e) = err.as_ref() {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(format!("⚠ {}", e))
                            .size(10.5)
                            .color(ui::DANGER),
                    );
                }
                if !loaded {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("Đang đọc trạng thái (powercfg / bcdedit / PowerShell)…")
                            .size(10.5)
                            .color(ui::TEXT_MUTED),
                    );
                }

                for (gi, group) in TWEAK_GROUPS.iter().enumerate() {
                    let rows: Vec<usize> = tweaks
                        .iter()
                        .enumerate()
                        .filter(|(_, t)| t.group == *group)
                        .map(|(i, _)| i)
                        .collect();
                    if rows.is_empty() {
                        continue;
                    }
                    ui.add_space(if gi == 0 { 10.0 } else { 14.0 });
                    ui.label(
                        RichText::new(group.label().to_uppercase())
                            .size(10.5)
                            .strong()
                            .extra_letter_spacing(1.0)
                            .color(ui::ACCENT),
                    );
                    ui.add_space(6.0);

                    for i in rows {
                        let t = &tweaks[i];
                        let state = states
                            .get(i)
                            .cloned()
                            .unwrap_or_else(|| {
                                wintweaks::TweakState::Unknown("Chưa đọc được trạng thái".into())
                            });
                        let in_backup = backup.has(t.id);
                        let confirming = confirm_row
                            .as_ref()
                            .is_some_and(|(id, _)| id == t.id);
                        let (click_on, click_off) =
                            tweak_row_ui(ui, t, &state, in_backup, busy, confirming);
                        if click_on {
                            if t.risk == wintweaks::Risk::Advanced && !confirming {
                                // Tinh chỉnh rủi ro cao: bắt bấm lần thứ hai.
                                confirm_row = Some((t.id.to_string(), Instant::now()));
                            } else {
                                confirm_row = None;
                                job = Some(TweakJob::Apply(vec![i]));
                            }
                        }
                        if click_off {
                            confirm_row = None;
                            job = Some(TweakJob::Revert(vec![i]));
                        }
                        ui.add_space(6.0);
                        hairline(ui);
                        ui.add_space(6.0);
                    }
                }
            });
        }

        self.confirm_apply_all = confirm_apply_all;
        self.confirm_revert_all = confirm_revert_all;
        self.confirm_row = confirm_row;
        if let Some(j) = job {
            self.start_tweak_job(j);
        }
    }

    /// Đăng ký lại toàn bộ phím tắt theo `sh.hotkeys` (chuỗi rỗng = phím đang tắt).
    /// `rebind` chạy bất đồng bộ; lỗi đăng ký đọc qua `HotkeyService::errors()` ở frame sau.
    fn apply_hotkeys(&mut self) {
        let map = self.sh.hotkeys.lock().unwrap().clone();
        let (bindings, errs) = parse_hotkeys(&map);
        self.hotkey_parse_errors = errs;
        *self.sh.hotkey_bindings.lock().unwrap() = bindings.clone();
        match self.hotkeys.as_mut() {
            Some(svc) => svc.rebind(bindings),
            None => {
                let sh = self.sh.clone();
                let ctx = self.ctx.clone();
                let hwnd = self.hwnd;
                self.hotkeys = Some(hotkey::HotkeyService::start(bindings, move |action| {
                    on_hotkey(&sh, &ctx, hwnd, action)
                }));
            }
        }
    }

    /// Bật/tắt phím tắt ngay khi người dùng đổi công tắc (không cần mở lại app).
    fn set_hotkeys_enabled(&mut self, enabled: bool) {
        if enabled {
            self.apply_hotkeys();
            self.sh.log(
                "  ✔ [PHÍM TẮT] Đã bật phím tắt toàn cục.".into(),
                ui::SUCCESS,
            );
        } else {
            // Drop `HotkeyService`: thread nhận WM_QUIT, huỷ hết đăng ký rồi thoát.
            self.hotkeys = None;
            self.sh.hotkey_bindings.lock().unwrap().clear();
            self.sh.log(
                "  • [PHÍM TẮT] Đã tắt phím tắt toàn cục (tổ hợp được nhả cho app khác).".into(),
                ui::TEXT_MUTED,
            );
        }
    }

    /// Lưu một tổ hợp người dùng vừa nhập: parse lỗi thì báo đỏ dưới ô và KHÔNG lưu.
    fn save_hotkey(&mut self, action: hotkey::HotkeyAction, text: String) {
        let raw = text.trim().to_string();
        if !raw.is_empty() {
            if let Err(e) = hotkey::Hotkey::parse(&raw) {
                self.hotkey_input_errors.insert(action, e);
                return;
            }
        }
        self.hotkey_input_errors.remove(&action);
        self.sh.hotkeys.lock().unwrap().insert(action, raw);
        self.hotkey_inputs.insert(action, text);
        self.apply_hotkeys();
        self.save();
        let combo = self.sh.hotkey_display(action);
        self.sh.log(
            format!(
                "  ✔ [PHÍM TẮT] {}: {}",
                action.label(),
                if combo.is_empty() {
                    "đã tắt".to_string()
                } else {
                    combo
                }
            ),
            ui::SUCCESS,
        );
    }

    /// Tab "Khởi động": mục chạy cùng Windows (đọc khi mở tab lần đầu và khi bấm Làm mới).
    fn render_startup_tab(&mut self, ui: &mut egui::Ui) {
        let loading = self.startup_busy.load(Ordering::SeqCst);
        let total = self.startup_view.len();
        let on = self.startup_view.iter().filter(|e| e.enabled).count();
        let current_exe = std::env::current_exe()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let mut refresh = false;
        let mut toggle: Option<(startup::StartupEntry, bool)> = None;
        let err = self.startup_err.clone();
        let row_errs = self.startup_row_err.clone();

        ui::card(ui, |ui| {
            ui.spacing_mut().item_spacing.x = ui::SPACE_TOOLBAR_GAP;
            ui.label(
                RichText::new(
                    "Tắt ở đây giống Task Manager: app không tự chạy khi đăng nhập, bật lại được bất cứ lúc nào.",
                )
                .size(11.0)
                .color(ui::TEXT_MUTED),
            );
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(if loading && total == 0 {
                        "Đang đọc danh sách…".to_string()
                    } else {
                        format!("{} mục, {} đang bật", total, on)
                    })
                    .size(11.5)
                    .color(ui::TEXT_MAIN),
                );
                ui.add_space(ui::SPACE_TOOLBAR_GAP);
                if ui
                    .add_enabled(!loading, ui::ghost_button_sized("🔄 Làm mới", 11.0))
                    .clicked()
                {
                    refresh = true;
                }
            });
            if let Some(e) = err.as_ref() {
                ui.add_space(4.0);
                ui.label(RichText::new(format!("⚠ {}", e)).size(10.5).color(ui::DANGER));
            }
        });

        ui.add_space(ui::SPACE_TOOLBAR_MARGIN_BOTTOM);

        egui::ScrollArea::vertical()
            .id_salt("startup_scroll_area")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui::card(ui, |ui| {
                    if total == 0 {
                        ui.add_space(12.0);
                        ui.label(
                            RichText::new(if loading {
                                "Đang quét registry Run và thư mục Startup…"
                            } else {
                                "Không thấy mục nào khởi động cùng Windows."
                            })
                            .size(11.5)
                            .color(ui::TEXT_MUTED),
                        );
                        ui.add_space(12.0);
                        return;
                    }
                    let list = self.startup_view.clone();
                    for entry in list.iter() {
                        let locked = is_self_startup(entry.exe_path.as_deref(), &current_exe);
                        let mut enabled = entry.enabled;
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(&entry.name)
                                            .size(12.0)
                                            .strong()
                                            .color(ui::TEXT_MAIN),
                                    );
                                    ui.label(
                                        RichText::new(entry.location.label())
                                            .size(10.0)
                                            .color(ui::TEXT_DIM),
                                    );
                                    if locked {
                                        ui.label(
                                            RichText::new("chính GameMode — không tắt ở đây")
                                                .size(10.0)
                                                .color(ui::ACCENT),
                                        );
                                    }
                                });
                                let full = entry.command.clone();
                                ui.label(
                                    RichText::new(shorten_command(&entry.command, 62))
                                        .size(10.5)
                                        .monospace()
                                        .color(if entry.enabled { ui::TEXT_MUTED } else { ui::TEXT_DIM }),
                                )
                                .on_hover_text(full);
                                if let Some(e) = row_errs.get(&entry.name) {
                                    ui.label(RichText::new(format!("⚠ {}", e)).size(10.0).color(ui::DANGER));
                                }
                            });
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if locked {
                                    // Công tắc bị khoá: bấm không có tác dụng, config không đổi.
                                    let size = vec2(28.0, 16.0);
                                    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                                    ui::draw_switch(&ui.painter(), rect, enabled, false);
                                } else if ui::compact_switch(ui, &mut enabled) {
                                    toggle = Some((entry.clone(), enabled));
                                }
                            });
                        });
                        ui.add_space(6.0);
                    }
                });
            });

        if loading {
            ui.ctx().request_repaint_after(Duration::from_millis(200));
        }
        if refresh {
            self.startup_err = None;
            self.start_startup_scan();
        }
        if let Some((entry, want)) = toggle {
            match startup::set_enabled(&entry, want) {
                Ok(_) => {
                    self.startup_row_err.remove(&entry.name);
                    // Ghi cờ chỉ đổi trạng thái cho lần đăng nhập sau -> cập nhật danh sách đang hiện.
                    if let Some(e) = self
                        .startup_view
                        .iter_mut()
                        .find(|e| e.name == entry.name && e.location == entry.location)
                    {
                        e.enabled = want;
                    }
                    self.sh.log(
                        format!(
                            "  ✔ [KHỞI ĐỘNG] {} — {} (áp dụng từ lần đăng nhập sau).",
                            entry.name,
                            if want { "BẬT" } else { "TẮT" }
                        ),
                        if want { ui::SUCCESS } else { ui::TEXT_MUTED },
                    );
                }
                Err(e) => {
                    self.startup_row_err.insert(entry.name.clone(), e.clone());
                    self.sh.log(
                        format!("  ⚠ [KHỞI ĐỘNG] {}: {}", entry.name, e),
                        ui::DANGER,
                    );
                }
            }
        }
    }

    /// Card "Phím tắt": công tắc chung, mỗi hành động một ô nhập tổ hợp + nút "Lưu".
    fn render_hotkey_card(&mut self, ui: &mut egui::Ui, opts_view: &mut Opts, save_needed: &mut bool) {
        let service_errors = self.hotkeys.as_ref().map(|s| s.errors()).unwrap_or_default();
        let parse_errors = self.hotkey_parse_errors.clone();
        let mut toggle_to: Option<bool> = None;
        let mut save: Option<(hotkey::HotkeyAction, String)> = None;

        ui::card(ui, |ui| {
            ui::section_header(
                ui,
                "⌨",
                "Phím tắt",
                Some("Dùng được ở bất cứ đâu, kể cả khi cửa sổ đã ẩn xuống khay"),
            );
            ui.add_space(ui::SPACE_HEADER_GAP);

            if ui::toggle_row(
                ui,
                &mut opts_view.hotkey_enabled,
                "Bật phím tắt toàn cục",
                "Bấm tổ hợp để Boost / Khôi phục / ẩn-hiện cửa sổ; để trống ô để tắt riêng phím đó",
            ) {
                *save_needed = true;
                toggle_to = Some(opts_view.hotkey_enabled);
            }
            ui.add_space(6.0);

            if !opts_view.hotkey_enabled {
                ui.label(
                    RichText::new("Đang tắt — không phím tắt nào hoạt động.")
                        .size(11.0)
                        .color(ui::TEXT_MUTED),
                );
                return;
            }

            for action in HOTKEY_ACTIONS {
                let mut text = self.hotkey_inputs.get(&action).cloned().unwrap_or_default();
                let mut clicked = false;
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(action.label())
                            .size(11.0)
                            .color(ui::TEXT_MAIN),
                    );
                    let btn_w = 54.0_f32;
                    let spacing = 6.0_f32;
                    let edit_w = (ui.available_width() - btn_w - spacing).max(80.0_f32);
                    egui::Frame::none()
                        .fill(ui::BG)
                        .stroke(Stroke::new(1.0_f32, ui::BORDER))
                        .rounding(Rounding::same(6.0))
                        .inner_margin(Margin::symmetric(8.0, 5.0))
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut text)
                                    .hint_text("vd: Ctrl+Alt+B (để trống = tắt phím này)")
                                    .text_color(ui::TEXT_MAIN)
                                    .frame(false)
                                    .desired_width(edit_w - 16.0),
                            );
                        });
                    if ui.add(ui::ghost_button_sized("Lưu", 11.0)).clicked() {
                        clicked = true;
                    }
                });
                if let Some(err) = self.hotkey_input_errors.get(&action) {
                    ui.label(RichText::new(format!("⚠ {}", err)).size(10.0).color(ui::DANGER));
                } else if let Some((_, err)) = parse_errors.iter().find(|(a, _)| *a == action) {
                    ui.label(RichText::new(format!("⚠ {}", err)).size(10.0).color(ui::DANGER));
                } else if let Some((_, err)) = service_errors.iter().find(|(a, _)| *a == action) {
                    ui.label(RichText::new(format!("⚠ {}", err)).size(10.0).color(ui::DANGER));
                } else {
                    let combo = self.sh.hotkey_display(action);
                    ui.label(
                        RichText::new(if combo.is_empty() {
                            "Đang tắt phím này.".to_string()
                        } else {
                            format!("Đang dùng: {}", combo)
                        })
                        .size(10.0)
                        .color(ui::TEXT_DIM),
                    );
                }
                if clicked {
                    save = Some((action, text));
                }
                ui.add_space(4.0);
            }
        });

        if let Some(on) = toggle_to {
            self.set_hotkeys_enabled(on);
        }
        if let Some((action, text)) = save {
            self.save_hotkey(action, text);
        }
    }

    /// Quét lại danh sách app đang chạy ở thread nền.
    fn start_scan(&mut self) {
        if self.scanning.swap(true, Ordering::SeqCst) {
            return;
        }
        let running = self.sh.running.clone();
        let scanning = self.scanning.clone();
        let games = self.sh.games.lock().unwrap().clone();
        let keep = self.sh.keep.lock().unwrap().clone();
        let skip = skip_list(&games);
        std::thread::spawn(move || {
            let list = scan_running_apps(&skip, &games, &keep);
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
            // % nằm cùng hàng tiêu đề: hàng số GB lớn bên dưới không còn chữ bên phải để đè
            // khi cột hẹp (cửa sổ nhỏ hoặc Windows scale 125–150%).
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
        });
        ui.add_space(6.0);

        // Thanh RAM và Pagefile chia đoạn (~20 ô nhỏ, cách nhau 2px)
        ui::segmented_bar(ui, ratio);

        ui.add_space(5.0);
        ui.label(
            RichText::new(sub_text)
                .size(10.0)
                .color(if ratio > 0.80 { ui::DANGER } else { ui::TEXT_MUTED }),
        );
        // Font thường thay monospace: monospace bị giãn chữ khi xuống dòng ở cột hẹp.
        ui.label(
            RichText::new(extra.unwrap_or(" "))
                .size(9.5)
                .color(ui::TEXT_MUTED),
        );
    });
}

/// Dòng trạng thái cập nhật (kèm mốc kiểm tra gần nhất) và màu tương ứng.
fn update_status_text(state: &update::UpdateState, last: Option<Instant>) -> (String, Color32) {
    let color = match state {
        update::UpdateState::Idle | update::UpdateState::Checking => ui::TEXT_MUTED,
        update::UpdateState::UpToDate => ui::SUCCESS,
        update::UpdateState::Available(_) => ui::ACCENT,
        update::UpdateState::Downloading { .. } => ui::DATA_CYAN,
        update::UpdateState::Ready(_) => ui::SUCCESS,
        update::UpdateState::Failed(_) => ui::DANGER,
    };
    let when = match last {
        Some(t) => {
            let secs = t.elapsed().as_secs();
            if secs < 90 {
                format!(" · kiểm tra {secs} giây trước")
            } else if secs < 5400 {
                format!(" · kiểm tra {} phút trước", secs / 60)
            } else {
                format!(" · kiểm tra {} giờ trước", secs / 3600)
            }
        }
        None => String::new(),
    };
    (format!("{}{}", state.label(), when), color)
}

/// Card "Cập nhật ứng dụng" trong tab Tự động & Tối ưu: công tắc tự kiểm tra, phiên bản hiện tại, trạng thái + lần kiểm tra cuối, nút kiểm tra.
fn render_update_card(
    ui: &mut egui::Ui,
    sh: &Shared,
    opts_view: &mut Opts,
    save_needed: &mut bool,
    update_err: &mut Option<String>,
) {
    ui::card(ui, |ui| {
        ui::section_header(
            ui,
            "OTA",
            "Cập nhật ứng dụng",
            Some("Tự động kiểm tra bản phát hành mới từ server nội bộ"),
        );
        ui.add_space(ui::SPACE_HEADER_GAP);

        if ui::toggle_row(
            ui,
            &mut opts_view.update_enabled,
            "Tự động kiểm tra cập nhật",
            "Kiểm tra 20 giây sau khi mở app, sau đó mỗi 6 giờ",
        ) {
            *save_needed = true;
        }
        ui.add_space(8.0);

        let state = sh.update_state();
        let last = *sh.update_last_check.lock().unwrap();
        let (status_text, color) = update_status_text(&state, last);

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.label(
                RichText::new(format!("Phiên bản: v{}", update::current_version()))
                    .size(11.5)
                    .monospace()
                    .color(ui::TEXT_MAIN),
            );
            ui.label(RichText::new("·").size(11.5).color(ui::TEXT_DIM));
            ui.label(RichText::new(status_text).size(11.5).color(color));

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let busy_net = matches!(
                    &state,
                    update::UpdateState::Checking | update::UpdateState::Downloading { .. }
                );
                if ui
                    .add_enabled(!busy_net, ui::ghost_button_sized("Kiểm tra cập nhật", 11.5))
                    .clicked()
                {
                    update::request_check(sh);
                    *update_err = None;
                }
            });
        });

        if let Some(e) = update_err.as_ref() {
            ui.add_space(6.0);
            ui.label(RichText::new(format!("⚠ {e}")).size(11.0).color(ui::DANGER));
        }
    });
}

/// Một lựa chọn nhỏ trong hàng 3 lựa chọn "Chung / Bật / Tắt" của hồ sơ game.
fn tri_choice(ui: &mut egui::Ui, text: &str, selected: bool) -> bool {
    let btn = egui::Button::new(
        RichText::new(text)
            .size(10.5)
            .strong()
            .color(if selected { ui::ACCENT } else { ui::TEXT_MUTED }),
    )
    .fill(if selected { ui::CARD_HOVER } else { ui::CARD_BG })
    .stroke(Stroke::new(1.0_f32, if selected { ui::ACCENT } else { ui::BORDER }))
    .min_size(vec2(46.0, 20.0))
    .rounding(Rounding::same(4.0));
    ui.add(btn).clicked()
}

/// Khối "Hồ sơ riêng" của một game: mỗi tuỳ chọn một hàng 3 lựa chọn **Chung / Bật / Tắt**.
/// "Chung" nghĩa là KHÔNG ghi dòng config nào cho key đó (dùng cài đặt ở card "Tối ưu khi chơi").
fn profile_block(
    ui: &mut egui::Ui,
    exe: &str,
    profiles: &mut HashMap<String, GameProfile>,
    save_needed: &mut bool,
) {
    egui::Frame::none()
        .fill(ui::BG)
        .stroke(Stroke::new(1.0_f32, ui::BORDER))
        .rounding(Rounding::same(6.0))
        .inner_margin(Margin::symmetric(8.0, 6.0))
        .show(ui, |ui| {
            ui.label(
                RichText::new(format!("HỒ SƠ RIÊNG — {}", clean_game_name(exe)))
                    .size(10.5)
                    .strong()
                    .color(ui::ACCENT),
            );
            ui.label(
                RichText::new(
                    "Chung = theo cài đặt ở card \"Tối ưu khi chơi\". Hồ sơ chỉ áp cho lần Boost của game này.",
                )
                .size(10.0)
                .color(ui::TEXT_MUTED),
            );
            ui.add_space(4.0);
            let profile = profiles.entry(exe.to_string()).or_default();
            for (key, label) in PROFILE_KEYS {
                let cur = profile.get(*key).copied();
                let mut next = cur;
                ui.horizontal(|ui| {
                    ui.label(RichText::new(*label).size(11.0).color(ui::TEXT_MAIN));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Bố cục phải->trái nên thứ tự bấm là Tắt, Bật, Chung.
                        if tri_choice(ui, "Tắt", cur == Some(false)) {
                            next = Some(false);
                        }
                        if tri_choice(ui, "Bật", cur == Some(true)) {
                            next = Some(true);
                        }
                        if tri_choice(ui, "Chung", cur.is_none()) {
                            next = None;
                        }
                    });
                });
                if next != cur {
                    match next {
                        Some(v) => {
                            profile.insert((*key).to_string(), v);
                        }
                        None => {
                            profile.remove(*key);
                        }
                    }
                    *save_needed = true;
                }
            }
        });
    // Hồ sơ rỗng thì bỏ hẳn khỏi map: game không còn mang nhãn "hồ sơ riêng" nữa.
    if profiles.get(exe).is_some_and(|p| p.is_empty()) {
        profiles.remove(exe);
    }
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
    open_profile: &mut Option<String>,
    profiles_view: &mut HashMap<String, GameProfile>,
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
            "Task 'GameMode' tạo bằng XML: chạy lúc đăng nhập, quyền cao nhất, chạy cả khi dùng pin — không hiện UAC",
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
                    *last_task_check = Instant::now()
                        .checked_sub(Duration::from_secs(20))
                        .unwrap_or_else(Instant::now);
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
                // Mở khối "Hồ sơ" thì nới chiều cao để thấy đủ 8 tuỳ chọn mà không phải cuộn.
                .max_height(if open_profile.is_some() { 460.0 } else { 100.0 })
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for exe in games_view.iter() {
                        let path = sh.game_paths.lock().unwrap().get(exe).cloned();
                        let has_profile = profiles_view.get(exe).is_some_and(|p| !p.is_empty());
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
                            .on_hover_text("Xoá khỏi danh sách game (xoá luôn hồ sơ riêng của nó)");
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

                        // Nút "Hồ sơ": mở/thu khối tuỳ chọn riêng của game này.
                        let open = open_profile.as_deref() == Some(exe.as_str());
                        let prof_rect = Rect::from_center_size(
                            pos2(rect.right() - 58.0, rect.center().y),
                            vec2(54.0, 18.0),
                        );
                        let prof_resp = ui
                            .interact(prof_rect, ui.id().with(("profile", exe)), egui::Sense::click())
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text(
                                "Hồ sơ riêng cho game này: tuỳ chọn riêng thay vì dùng cài đặt chung",
                            );
                        let prof_color = if open || prof_resp.hovered() { ui::ACCENT } else { ui::TEXT_MUTED };
                        painter.rect_stroke(prof_rect, Rounding::same(4.0), Stroke::new(1.0_f32, prof_color));
                        painter.text(
                            prof_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            if open { "Thu" } else { "Hồ sơ" },
                            FontId::proportional(10.5),
                            prof_color,
                        );
                        if prof_resp.clicked() {
                            *open_profile = if open { None } else { Some(exe.clone()) };
                        }

                        // Chữ chỉ được chiếm tới sát nút "Hồ sơ" (chừa 92px bên phải).
                        let text_right = rect.right() - 92.0;
                        let text_max_w = (text_right - rect.left() - 2.0).max(40.0);
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
                        let mut x_cursor = rect.left() + 2.0 + exe_w + 6.0;

                        // Nhãn nhỏ cho game đang có hồ sơ riêng.
                        if has_profile {
                            let tag = painter.layout_no_wrap(
                                "hồ sơ riêng".to_string(),
                                FontId::proportional(10.0),
                                ui::ACCENT,
                            );
                            let tag_w = tag.size().x;
                            if x_cursor + tag_w < text_right {
                                painter.galley(
                                    pos2(x_cursor, rect.center().y - tag.size().y / 2.0),
                                    tag,
                                    ui::ACCENT,
                                );
                                x_cursor += tag_w + 6.0;
                            }
                        }

                        let path_desc = match path {
                            Some(ref p) => format!(" — {}", p),
                            None => " — (chưa biết đường dẫn)".to_string(),
                        };
                        let path_max_w = (rect.left() + 2.0 + text_max_w - x_cursor).max(0.0);
                        if path_max_w > 20.0 {
                            // Một dòng, cắt "…" khi hẹp: xuống dòng trong khe hẹp bị che mất chữ.
                            let mut path_job = egui::text::LayoutJob::simple(
                                path_desc,
                                FontId::proportional(10.5),
                                ui::TEXT_DIM,
                                path_max_w,
                            );
                            path_job.wrap.max_rows = 1;
                            path_job.wrap.break_anywhere = true;
                            let path_galley = painter.layout_job(path_job);
                            painter.galley(
                                pos2(x_cursor, rect.center().y - path_galley.size().y / 2.0),
                                path_galley,
                                ui::TEXT_DIM,
                            );
                        }
                        ui.add_space(2.0);

                        // Khối "Hồ sơ" nằm ngay dưới dòng của game đang mở.
                        if open {
                            profile_block(ui, exe, profiles_view, save_needed);
                            ui.add_space(4.0);
                        }
                    }
                });
        }
        if let Some(exe) = remove {
            sh.games.lock().unwrap().retain(|e| e != &exe);
            // Xoá game thì xoá luôn hồ sơ riêng của nó (config cũng không còn dòng nào).
            profiles_view.remove(&exe);
            if open_profile.as_deref() == Some(exe.as_str()) {
                *open_profile = None;
            }
            sh.log(
                format!("  • [GAME] Đã xoá {} khỏi danh sách game (kèm hồ sơ riêng nếu có).", exe),
                ui::TEXT_MUTED,
            );
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
                    // App hệ thống / app luôn chạy (WebView2, trình duyệt, explorer...) không
                    // bao giờ là game: đưa nhầm vào đây làm Boost chạy ngay khi mở app.
                    if is_not_game(&exe) {
                        game_input.clear();
                        sh.log(
                            format!(
                                "  ⚠ [GAME] Không thêm {exe} — đây là app hệ thống/luôn chạy, không phải game."
                            ),
                            ui::WARNING,
                        );
                    } else {
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
            }
        });

        ui.add_space(4.0);
        ui.label(
            RichText::new("(hoặc chuột phải vào app ở tab Ứng dụng đang chạy → 'Đánh dấu là game')")
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

/// Một dòng app đang chạy: tick "Giữ lại" = không tắt app này; menu chuột phải "Đánh dấu là game"
/// = thêm nhanh exe vào danh sách game để watcher tự tối ưu. Dòng mà MỌI instance đều là con cháu của một
/// app được giữ (vd node.exe do Claude mở) luôn hiện "GIỮ LẠI" và công tắc bị khoá: dòng chỉ
/// nhận `Sense::hover()` nên bấm vào không có tác dụng, config không đổi.
fn running_app_row(
    ui: &mut egui::Ui,
    keep: &mut bool,
    app: &RunningApp,
    is_game: bool,
) -> RowAction {
    use egui::{pos2, vec2, Align2, CursorIcon, FontId, Rect, Sense};

    let row_h = ui::ROW_APP_HEIGHT;
    // Khoá công tắc khi mọi instance đều là con cháu của app được giữ: không tắt được dòng này.
    let locked = app.kept_children > 0 && app.kept_children >= app.count;
    let keep_shown = *keep || locked;
    let sense = if locked { Sense::hover() } else { Sense::click() };
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), sense);
    let resp = if locked {
        resp
    } else {
        resp.on_hover_cursor(CursorIcon::PointingHand)
    };
    let mut action = if !locked && resp.clicked() {
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
    let bar_color = if keep_shown { ui::SUCCESS } else { ui::DANGER };
    let bar = Rect::from_min_size(rect.min + vec2(0.0, 6.0), vec2(3.0, row_h - 12.0));
    painter.rect_filled(bar, Rounding::same(1.0), bar_color);

    let switch_rect = Rect::from_center_size(
        pos2(rect.left() + 20.0, rect.center().y),
        vec2(28.0, 16.0),
    );
    ui::draw_switch(&painter, switch_rect, keep_shown, hovered);

    let title = app.display.clone().unwrap_or_else(|| app.exe_name.clone());
    let name_color = if keep_shown {
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
    // Dòng có instance là con cháu của app được giữ: ghi rõ bao nhiêu giữ / bao nhiêu sẽ tắt.
    let full_info_str = match app.kept_by.as_deref() {
        Some(parent) if locked => format!(
            " · {} proc — {} · con của {}",
            app.count,
            fmt_ram(app.total_ws),
            parent
        ),
        Some(parent) => format!(
            " · {} giữ (con của {}) · {} sẽ tắt — {}",
            app.kept_children,
            parent,
            app.count - app.kept_children,
            fmt_ram(app.total_ws)
        ),
        None => format!(" · {} proc — {}", app.count, fmt_ram(app.total_ws)),
    };
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
    let mb_fill_color = if keep_shown { ui::DATA_CYAN } else { ui::DANGER };
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
    let (badge, badge_color) = if keep_shown {
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
        let lock_str = if locked {
            format!(
                "\nCon của {} — app được giữ lại, không tắt được dòng này.",
                app.kept_by.as_deref().unwrap_or("app được giữ")
            )
        } else {
            String::new()
        };
        let mut s = if app.display.as_deref().map_or(false, |d| !d.eq_ignore_ascii_case(&app.exe_name)) {
            format!("{}\n{}\n{}", display_str, app.exe_name, path_str)
        } else {
            format!("{}\n{}", app.exe_name, path_str)
        };
        s.push_str(&lock_str);
        s
    };
    let resp = resp.on_hover_text(hover_text);

    // Exe đã nằm trong danh sách game thì hiện nhãn ✔; thêm vào danh sách game qua menu chuột phải
    // (nút "Là game" trên mỗi dòng trông rối nên đã bỏ).
    if is_game {
        painter.text(
            pos2(rect.right() - 8.0, y_sub),
            Align2::RIGHT_CENTER,
            "✔ LÀ GAME",
            FontId::proportional(9.5),
            ui::SUCCESS,
        );
    } else if !locked {
        resp.context_menu(|ui| {
            if ui
                .button("Đánh dấu là game")
                .on_hover_text("Thêm exe này vào danh sách game để tự tối ưu khi mở game")
                .clicked()
            {
                action = RowAction::MarkGame;
                ui.close_menu();
            }
        });
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

/// Rút gọn và chuẩn hoá tên game thân thiện cho HUD.
fn clean_game_name(raw: &str) -> String {
    let trimmed = raw.trim();
    let no_exe = if trimmed.len() >= 4 && trimmed[trimmed.len() - 4..].eq_ignore_ascii_case(".exe") {
        &trimmed[..trimmed.len() - 4]
    } else {
        trimmed
    };

    let lower = no_exe.to_ascii_lowercase();

    // Mapping đặc biệt: Delta Force
    if lower.starts_with("deltaforceclient") || lower == "deltaforce" || lower == "delta force" {
        return "Delta Force".to_string();
    }

    // Tên khác: bỏ hậu tố -Win64-Shipping hoặc -Shipping (không phân biệt hoa thường)
    let mut cleaned = no_exe;
    if lower.ends_with("-win64-shipping") {
        cleaned = &cleaned[..cleaned.len() - "-win64-shipping".len()];
    } else if lower.ends_with("-shipping") {
        cleaned = &cleaned[..cleaned.len() - "-shipping".len()];
    }

    cleaned.to_string()
}

/// Cắt ngắn tên game có kèm dấu chấm lửng cho danh sách cột trái.
fn truncate_game_name(name: &str, max_chars: usize) -> String {
    if name.chars().count() > max_chars {
        format!("{}…", name.chars().take(max_chars.saturating_sub(1)).collect::<String>())
    } else {
        name.to_string()
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

    // FPS: cùng cách tính với `report::summarize` (trung bình có trọng số theo thời gian mỗi mẫu),
    // để số đang xem lúc ghi khớp số chốt lúc trận kết thúc.
    let mut prev_t = 0.0_f32;
    let mut fps_sum = 0.0_f64;
    let mut fps_weight = 0.0_f64;
    let mut fps_low1_sum = 0.0_f32;
    let mut fps_low1_count = 0_u32;
    for s in &rep.samples {
        let dt = (s.t - prev_t).max(0.0);
        prev_t = s.t;
        if let Some(f) = s.fps {
            let w = if dt > 0.0 { dt } else { report::SAMPLE_SEC as f32 };
            fps_sum += f as f64 * w as f64;
            fps_weight += w as f64;
        }
        if let Some(l) = s.fps_low1 {
            fps_low1_sum += l;
            fps_low1_count += 1;
        }
    }
    rep.summary.fps_avg = (fps_weight > 0.0).then(|| (fps_sum / fps_weight) as f32);
    rep.summary.fps_low1 = (fps_low1_count > 0).then(|| fps_low1_sum / fps_low1_count as f32);
    rep.summary.frametime_max_ms = rep
        .samples
        .iter()
        .filter_map(|s| s.frametime_max_ms)
        .reduce(f32::max);
    rep.summary.hitches = rep
        .samples
        .iter()
        .map(|s| s.hitches)
        .fold(0, u32::saturating_add);
    // 0,1% low và p99 chỉ có khi chốt trận bằng histogram ETW — lúc đang ghi thì chưa có.
    rep.summary.fps_low01 = None;
    rep.summary.frametime_p99_ms = None;

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

/// Định dạng thời gian biểu đồ (giây) thành chuỗi m:ss hoặc h:mm:ss.
fn format_plot_time(sec: f64) -> String {
    let total_sec = sec.round() as i64;
    let sign = if total_sec < 0 { "-" } else { "" };
    let abs_sec = total_sec.unsigned_abs();
    let s = abs_sec % 60;
    let m = (abs_sec / 60) % 60;
    let h = abs_sec / 3600;
    if h > 0 {
        format!("{}{}:{:02}:{:02}", sign, h, m, s)
    } else {
        format!("{}{}:{:02}", sign, m, s)
    }
}

/// Chọn bước thời gian hợp lý (giây) để có khoảng 4-8 nhãn cho mọi độ dài (20s đến 2h+).
fn pick_time_step(span: f64) -> f64 {
    const CANDIDATES: &[f64] = &[
        5.0,     // 20s - 35s
        10.0,    // 40s - 70s
        15.0,    // 75s - 105s
        30.0,    // 120s - 210s (2m - 3.5m)
        60.0,    // 4m - 7m
        120.0,   // 8m - 14m
        300.0,   // 15m - 35m
        600.0,   // 40m - 70m
        900.0,   // 75m - 105m
        1200.0,  // 110m - 140m
        1800.0,  // 2.5h+
    ];
    for &step in CANDIDATES {
        if span / step <= 7.0 {
            return step;
        }
    }
    1800.0
}

/// Tạo danh sách GridMark không âm (>= 0.0) theo bước lưới cố định.
fn generate_non_negative_grid_marks(min: f64, max: f64, step: f64) -> Vec<GridMark> {
    let mut marks = Vec::new();
    if step <= 0.0 || !min.is_finite() || !max.is_finite() {
        return marks;
    }
    let effective_min = min.max(0.0);
    let start = (effective_min / step).floor() as i64;
    let end = (max / step).ceil() as i64;
    for i in start..=end {
        let val = (i as f64) * step;
        if val >= 0.0 && val <= max + step * 0.001 {
            marks.push(GridMark {
                value: val,
                step_size: step,
            });
        }
    }
    marks
}

/// Grid spacer cho trục X thời gian (tính bằng giây).
fn time_x_grid_spacer(input: GridInput) -> Vec<GridMark> {
    let (min, max) = input.bounds;
    let span = (max - min).max(1.0);
    let step = pick_time_step(span);
    generate_non_negative_grid_marks(min, max, step)
}

/// Formatter nhãn trục X thời gian (0:30, 1:00, 1:02:05...).
fn time_x_axis_formatter(mark: GridMark, _range: &std::ops::RangeInclusive<f64>) -> String {
    format_plot_time(mark.value)
}

/// Grid spacer cho trục Y của biểu đồ RAM (bước 1, 2, 4, 8 GB tuỳ biên độ).
fn ram_y_grid_spacer(input: GridInput) -> Vec<GridMark> {
    let (min, max) = input.bounds;
    let span = (max - min).max(1.0);
    // Bước nhỏ quá thì vạch sát nhau và egui_plot làm mờ nhãn, nên giữ tối đa ~5 nhãn.
    let step = if span <= 4.0 {
        1.0
    } else if span <= 10.0 {
        2.0
    } else if span <= 20.0 {
        4.0
    } else {
        8.0
    };
    generate_non_negative_grid_marks(min, max, step)
}

/// Grid spacer cho trục Y phần trăm (%) (bước 25%).
fn pct_y_grid_spacer(input: GridInput) -> Vec<GridMark> {
    let (min, max) = input.bounds;
    generate_non_negative_grid_marks(min, max, 25.0)
}

/// Grid spacer cho trục Y độ trễ Ping (ms) (bước 10, 20, 50, 100 ms tuỳ biên độ).
fn ping_y_grid_spacer(input: GridInput) -> Vec<GridMark> {
    let (min, max) = input.bounds;
    let span = (max - min).max(1.0);
    let step = if span <= 50.0 {
        10.0
    } else if span <= 120.0 {
        20.0
    } else if span <= 300.0 {
        50.0
    } else {
        100.0
    };
    generate_non_negative_grid_marks(min, max, step)
}

/// Grid spacer cho trục Y của biểu đồ FPS (bước 10, 20, 30, 60, 120 FPS tuỳ biên độ).
fn fps_y_grid_spacer(input: GridInput) -> Vec<GridMark> {
    let (min, max) = input.bounds;
    let span = (max - min).max(1.0);
    let step = if span <= 40.0 {
        10.0
    } else if span <= 90.0 {
        20.0
    } else if span <= 150.0 {
        30.0
    } else if span <= 300.0 {
        60.0
    } else {
        120.0
    };
    generate_non_negative_grid_marks(min, max, step)
}

/// Khung Plot dùng chung cho cả 4 biểu đồ báo cáo trận đấu.
fn base_report_plot<'a>(id: &'a str) -> Plot<'a> {
    Plot::new(id)
        .height(175.0)
        .show_background(false)
        .show_axes([true, true])
        .show_grid([true, true])
        .allow_scroll(false)
        .allow_drag(false)
        .allow_zoom(false)
        .include_y(0.0)
        .y_axis_min_width(50.0)
        .x_grid_spacer(time_x_grid_spacer)
        .x_axis_formatter(time_x_axis_formatter)
}

/// Vẽ các vạch dọc mờ thể hiện sự kiện trên các biểu đồ.
fn render_plot_events(plot_ui: &mut egui_plot::PlotUi, events: &[report::Event]) {
    for ev in events {
        let x = ev.t as f64;
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
        ui::section_header(ui, "// RAM", "RAM Khả dụng & Standby", Some("GB theo thời gian"));
        ui.add_space(2.0);
        ui::plot_legend_dots(ui, &[
            ("Khả dụng (GB)", ui::DATA_CYAN),
            ("Standby (GB)", ui::ACCENT),
        ]);
        ui.add_space(6.0);

        let avail_pts: Vec<[f64; 2]> = report.samples.iter()
            .map(|s| [s.t as f64, s.avail_gb as f64])
            .collect();
        let standby_pts: Vec<[f64; 2]> = report.samples.iter()
            .map(|s| [s.t as f64, s.standby_gb.unwrap_or(0.0) as f64])
            .collect();

        let l_avail = Line::new(PlotPoints::new(avail_pts))
            .name("Khả dụng (GB)")
            .color(ui::DATA_CYAN)
            .width(1.5_f32);
        let l_standby = Line::new(PlotPoints::new(standby_pts))
            .name("Standby (GB)")
            .color(ui::ACCENT)
            .width(1.5_f32);

        let plot = base_report_plot("plot_ram")
            .y_grid_spacer(ram_y_grid_spacer)
            .y_axis_formatter(|mark, _| {
                if mark.value.fract().abs() < 1e-4 {
                    format!("{:.0} GB", mark.value)
                } else {
                    format!("{:.1} GB", mark.value)
                }
            })
            .label_formatter(|name, val| {
                let t = format_plot_time(val.x);
                if !name.is_empty() {
                    format!("{}: {:.2} GB · {}", name, val.y, t)
                } else {
                    format!("{:.2} GB · {}", val.y, t)
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
        ui::section_header(ui, "// COMMIT", "Bộ nhớ ảo (Commit %)", Some("Tải bộ nhớ ảo theo thời gian"));
        ui.add_space(2.0);
        ui::plot_legend_dots(ui, &[
            ("Commit (%)", ui::ACCENT),
            ("Cảnh báo 90%", ui::DANGER),
        ]);
        ui.add_space(6.0);

        let commit_pts: Vec<[f64; 2]> = report.samples.iter()
            .map(|s| [s.t as f64, s.commit_pct as f64])
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

        let plot = base_report_plot("plot_commit")
            .include_y(100.0)
            .y_grid_spacer(pct_y_grid_spacer)
            .y_axis_formatter(|mark, _| format!("{:.0}%", mark.value))
            .label_formatter(|name, val| {
                let t = format_plot_time(val.x);
                if !name.is_empty() {
                    format!("{}: {:.1}% · {}", name, val.y, t)
                } else {
                    format!("{:.1}% · {}", val.y, t)
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
        ui::section_header(ui, "// CPU", "Tải CPU (%)", Some("Toàn hệ thống theo thời gian"));
        ui.add_space(2.0);
        ui::plot_legend_dots(ui, &[
            ("CPU (%)", Color32::from_rgb(130, 180, 255)),
        ]);
        ui.add_space(6.0);

        let cpu_pts: Vec<[f64; 2]> = report.samples.iter()
            .map(|s| [s.t as f64, s.cpu_pct as f64])
            .collect();

        let l_cpu = Line::new(PlotPoints::new(cpu_pts))
            .name("CPU (%)")
            .color(Color32::from_rgb(130, 180, 255))
            .width(1.5_f32);

        let plot = base_report_plot("plot_cpu")
            .include_y(100.0)
            .y_grid_spacer(pct_y_grid_spacer)
            .y_axis_formatter(|mark, _| format!("{:.0}%", mark.value))
            .label_formatter(|name, val| {
                let t = format_plot_time(val.x);
                if !name.is_empty() {
                    format!("{}: {:.1}% · {}", name, val.y, t)
                } else {
                    format!("{:.1}% · {}", val.y, t)
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
        ui.add_space(2.0);
        ui::plot_legend_dots(ui, &[
            ("Internet (ms)", ui::DATA_CYAN),
            ("Router (ms)", ui::SUCCESS),
            ("Mất gói", ui::DANGER),
        ]);
        ui.add_space(6.0);

        let mut net_pts = Vec::new();
        let mut gw_pts = Vec::new();
        let mut loss_pts = Vec::new();

        for s in &report.samples {
            let t_sec = s.t as f64;
            if let Some(p) = s.ping_net_ms {
                net_pts.push([t_sec, p as f64]);
            }
            if let Some(p) = s.ping_gw_ms {
                gw_pts.push([t_sec, p as f64]);
            }
            if s.loss_net_pct > 0.0 || s.loss_gw_pct > 0.0 || s.ping_net_ms.is_none() {
                let y = s.ping_net_ms.or(s.ping_gw_ms).unwrap_or(0.0) as f64;
                loss_pts.push([t_sec, y]);
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

        let plot = base_report_plot("plot_ping")
            .y_grid_spacer(ping_y_grid_spacer)
            .y_axis_formatter(|mark, _| format!("{:.0} ms", mark.value))
            .label_formatter(|name, val| {
                let t = format_plot_time(val.x);
                if !name.is_empty() {
                    format!("{}: {:.1} ms · {}", name, val.y, t)
                } else {
                    format!("{:.1} ms · {}", val.y, t)
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

/// Biểu đồ FPS (đặt full width trên 4 biểu đồ kia): FPS TB & 1% low theo thời gian,
/// điểm đỏ ở các mẫu có khựng.
fn render_plot_fps(ui: &mut egui::Ui, report: &report::GameReport) {
    ui::card(ui, |ui| {
        ui::section_header(ui, "// FPS", "FPS & Khựng hình", Some("FPS theo thời gian"));
        ui.add_space(2.0);
        ui::plot_legend_dots(ui, &[
            ("FPS TB", ui::SUCCESS),
            ("1% low", ui::WARNING),
            ("Khựng", ui::DANGER),
        ]);
        ui.add_space(6.0);

        let mut avg_pts: Vec<[f64; 2]> = Vec::new();
        let mut low_pts: Vec<[f64; 2]> = Vec::new();
        let mut hitch_pts: Vec<[f64; 2]> = Vec::new();
        for s in &report.samples {
            let t = s.t as f64;
            if let Some(f) = s.fps {
                avg_pts.push([t, f as f64]);
            }
            if let Some(f) = s.fps_low1 {
                low_pts.push([t, f as f64]);
            }
            if s.hitches > 0 {
                // Điểm đỏ nằm trên đường FPS TB (hoặc 1% low) để thấy khựng rơi vào lúc nào.
                hitch_pts.push([t, s.fps.or(s.fps_low1).unwrap_or(0.0) as f64]);
            }
        }

        // Không có mẫu FPS nào: nói rõ vì sao trống thay vì vẽ biểu đồ rỗng vô nghĩa.
        if avg_pts.is_empty() {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new("Không có số liệu FPS (cần quyền admin, hoặc game không dùng DirectX)")
                        .size(11.0)
                        .color(ui::TEXT_MUTED),
                );
            });
            ui.add_space(20.0);
            return;
        }

        let l_avg = Line::new(PlotPoints::new(avg_pts))
            .name("FPS TB")
            .color(ui::SUCCESS)
            .width(1.6_f32);
        let l_low = Line::new(PlotPoints::new(low_pts))
            .name("1% low")
            .color(ui::WARNING)
            .width(1.2_f32);

        let plot = base_report_plot("plot_fps")
            .y_grid_spacer(fps_y_grid_spacer)
            .y_axis_formatter(|mark, _| format!("{:.0} FPS", mark.value))
            .label_formatter(|name, val| {
                let t = format_plot_time(val.x);
                if !name.is_empty() {
                    format!("{}: {:.0} FPS · {}", name, val.y, t)
                } else {
                    format!("{:.0} FPS · {}", val.y, t)
                }
            });

        plot.show(ui, |plot_ui| {
            plot_ui.line(l_avg);
            plot_ui.line(l_low);
            render_plot_events(plot_ui, &report.events);
            // Chấm khựng vẽ sau cùng để vạch sự kiện không đè lên.
            if !hitch_pts.is_empty() {
                let p_hitch = Points::new(PlotPoints::new(hitch_pts))
                    .name("Khựng")
                    .color(ui::DANGER)
                    .radius(4.5_f32)
                    .filled(true)
                    .shape(MarkerShape::Circle);
                plot_ui.points(p_hitch);
            }
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
                    RichText::new("ĐANG GHI SỐ LIỆU")
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

    // 2. Hàng thẻ chỉ số (12 chỉ số: 3 cột × 4 hàng, hoặc 2 cột × 6 hàng khi hẹp)
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

        let ping_text = match (sum.ping_net_avg_ms, sum.ping_net_p95_ms) {
            (Some(a), Some(p)) => format!("{:.0}/{:.0} ms", a, p),
            (Some(a), None) => format!("{:.0} ms", a),
            _ => "—".to_string(),
        };

        let net_max_text = sum
            .ping_net_max_ms
            .map(|m| format!("{:.0} ms", m))
            .unwrap_or_else(|| "—".to_string());

        let jitter_text = sum.jitter_net_ms.map(|j| format!("{:.1} ms", j)).unwrap_or_else(|| "—".to_string());
        let loss_text = format!("{:.1}%", sum.loss_net_pct);
        let loss_color = if sum.loss_net_pct > 0.0 { ui::DANGER } else { ui::SUCCESS };

        let gw_text = sum.ping_gw_avg_ms.map(|g| format!("{:.1} ms", g)).unwrap_or_else(|| "—".to_string());
        let gw_sub = format!("Loss: {:.1}%", sum.loss_gw_pct);

        let purge_text = format!("{} lần", sum.standby_purges);

        // FPS: "TB / 1% low"; dòng phụ có 0,1% low và p99 khi chốt trận bằng histogram ETW.
        let fps_text = match (sum.fps_avg, sum.fps_low1) {
            (Some(a), Some(l)) => format!("{:.0} / {:.0}", a, l),
            (Some(a), None) => format!("{:.0} / —", a),
            _ => "—".to_string(),
        };
        let fps_sub: Option<String> = match (sum.fps_low01, sum.frametime_p99_ms) {
            (Some(low01), Some(p99)) => Some(format!("0,1% low {:.0} · p99 {:.1} ms", low01, p99)),
            (Some(low01), None) => Some(format!("0,1% low {:.0}", low01)),
            (None, Some(p99)) => Some(format!("p99 {:.1} ms", p99)),
            (None, None) => None,
        };

        let hitch_text = format!("{} lần", sum.hitches);
        let hitch_color = if sum.hitches > 0 { ui::WARNING } else { ui::SUCCESS };

        // 12 thẻ: lưới 3 cột × 4 hàng (rộng) hoặc 2 cột × 6 hàng (hẹp) cho cân đối.
        let is_wide = ui.available_width() >= 540.0;
        if is_wide {
            ui.columns(3, |cols| {
                render_metric_box(&mut cols[0], "Thời lượng", &dur_text, Some(&format!("Bắt đầu: {}", format_short_date(&report.started_at))), ui::TEXT_MAIN);
                cols[0].add_space(8.0);
                render_metric_box(&mut cols[0], "RAM trống thấp nhất", &ram_min_text, Some(&format!("TB: {:.2} GB", sum.ram_avail_avg_gb)), ram_color);
                cols[0].add_space(8.0);
                render_metric_box(&mut cols[0], "Commit đỉnh", &commit_text, Some(&commit_sub), commit_color);
                cols[0].add_space(8.0);
                render_metric_box(&mut cols[0], "Xả Standby", &purge_text, Some("Giải phóng RAM"), ui::ACCENT);

                render_metric_box(&mut cols[1], "FPS TB / 1% low", &fps_text, fps_sub.as_deref(), ui::SUCCESS);
                cols[1].add_space(8.0);
                render_metric_box(&mut cols[1], "Khựng > 100 ms", &hitch_text, Some("Frame dài bất thường"), hitch_color);
                cols[1].add_space(8.0);
                render_metric_box(&mut cols[1], "CPU TB / Đỉnh", &cpu_text, Some("Toàn hệ thống"), ui::DATA_CYAN);
                cols[1].add_space(8.0);
                render_metric_box(&mut cols[1], "Ping Net TB / P95", &ping_text, Some("Internet"), ui::DATA_CYAN);

                render_metric_box(&mut cols[2], "Ping Net Max", &net_max_text, Some("Gói chậm nhất"), ui::DATA_CYAN);
                cols[2].add_space(8.0);
                render_metric_box(&mut cols[2], "Jitter độ trễ", &jitter_text, Some("Biên độ dao động"), ui::TEXT_MAIN);
                cols[2].add_space(8.0);
                render_metric_box(&mut cols[2], "Mất gói Net", &loss_text, Some("Tỉ lệ rớt gói"), loss_color);
                cols[2].add_space(8.0);
                render_metric_box(&mut cols[2], "Ping Router TB", &gw_text, Some(&gw_sub), ui::SUCCESS);
            });
        } else {
            ui.columns(2, |cols| {
                render_metric_box(&mut cols[0], "Thời lượng", &dur_text, None, ui::TEXT_MAIN);
                cols[0].add_space(8.0);
                render_metric_box(&mut cols[0], "RAM trống thấp nhất", &ram_min_text, None, ram_color);
                cols[0].add_space(8.0);
                render_metric_box(&mut cols[0], "Commit đỉnh", &commit_text, Some(&commit_sub), commit_color);
                cols[0].add_space(8.0);
                render_metric_box(&mut cols[0], "Xả Standby", &purge_text, None, ui::ACCENT);
                cols[0].add_space(8.0);
                render_metric_box(&mut cols[0], "FPS TB / 1% low", &fps_text, fps_sub.as_deref(), ui::SUCCESS);
                cols[0].add_space(8.0);
                render_metric_box(&mut cols[0], "Khựng > 100 ms", &hitch_text, None, hitch_color);

                render_metric_box(&mut cols[1], "CPU TB / Đỉnh", &cpu_text, None, ui::DATA_CYAN);
                cols[1].add_space(8.0);
                render_metric_box(&mut cols[1], "Ping Net TB / P95", &ping_text, None, ui::DATA_CYAN);
                cols[1].add_space(8.0);
                render_metric_box(&mut cols[1], "Ping Net Max", &net_max_text, None, ui::DATA_CYAN);
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

    // 4. Biểu đồ FPS full width + 4 biểu đồ đường
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
        // Biểu đồ FPS full width, nằm trên 4 biểu đồ cũ ở cả hai bố cục.
        render_plot_fps(ui, report);
        ui.add_space(ui::SPACE_CARD_GAP);
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
                                        let clean = clean_game_name(&l.report.game);
                                        (truncate_game_name(&clean, 16), l.report.samples.last().map(|s| s.t as u64).unwrap_or(0))
                                    };
                                    let is_sel = self.selected_report_id.as_deref() == Some("live");
                                    if ui::report_list_row(
                                        ui,
                                        is_sel,
                                        "ĐANG GHI",
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
                                        let game_clean = clean_game_name(&meta.game);
                                        let game_short = truncate_game_name(&game_clean, 16);
                                        // Dòng phụ: thời lượng, kèm FPS TB khi trận có đo được.
                                        let dur_str = match meta.fps_avg {
                                            Some(f) => format!(
                                                "{} · {:.0} FPS",
                                                format_duration(meta.duration_sec),
                                                f
                                            ),
                                            None => format_duration(meta.duration_sec),
                                        };

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
                                    RichText::new("Mở game trong danh sách, Paroda GameMode sẽ tự ghi số liệu và lưu báo cáo khi bạn tắt game.")
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
            // Bỏ chọn báo cáo vừa xoá để `refresh_reports` chọn lại (kể cả khi có trận đang ghi).
            if self.selected_report_id.as_deref() == Some(id.as_str()) {
                self.selected_report_id = None;
            }
            self.delete_confirm = None;
            self.cached_report = None;
            self.refresh_reports();
        } else if let Some(id) = confirm_req {
            self.delete_confirm = Some((id, Instant::now()));
        }
    }

    /// Banner thông báo cập nhật OTA trên cùng cột phải (phía trên hàng tab).
    /// Chỉ hiện khi có bản mới (Available), đang tải (Downloading), đã tải xong (Ready),
    /// hoặc lỗi tải/cài sau khi người dùng đã bấm cập nhật (Failed).
    fn render_update_banner(&mut self, ui: &mut egui::Ui) {
        let state = self.sh.update_state();

        match &state {
            update::UpdateState::Available(m) => {
                if self.update_dismissed_version.as_deref() == Some(&m.version) {
                    return;
                }

                ui::banner_card(ui, ui::ACCENT, Color32::from_rgb(70, 50, 15), |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        ui.label(RichText::new("⬆").size(14.0).strong().color(ui::ACCENT));
                        ui.label(
                            RichText::new(format!("Có bản mới {}", m.version))
                                .size(13.0)
                                .strong()
                                .color(ui::TEXT_MAIN),
                        );
                        ui.label(
                            RichText::new(format!("(hiện tại v{})", update::current_version()))
                                .size(11.0)
                                .color(ui::TEXT_MUTED),
                        );

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;

                            // Nút chính "Cập nhật" ở ngoài cùng bên phải
                            if ui.add(ui::primary_button_sized("Cập nhật", 11.5)).clicked() {
                                self.update_download_started = true;
                                self.update_err = None;
                                update::start_download(self.sh.clone());
                            }

                            // Nút phụ "Xem thay đổi"
                            let changelog_btn_text = if self.update_changelog_open {
                                "Ẩn thay đổi"
                            } else {
                                "Xem thay đổi"
                            };
                            if ui.add(ui::ghost_button_sized(changelog_btn_text, 11.5)).clicked() {
                                self.update_changelog_open = !self.update_changelog_open;
                            }

                            // Nút "Để sau": ẩn banner cho tới khi có version khác
                            if ui.add(ui::ghost_button_sized("Để sau", 11.5)).clicked() {
                                self.update_dismissed_version = Some(m.version.clone());
                            }
                        });
                    });

                    if self.update_changelog_open {
                        ui.add_space(8.0);
                        egui::Frame::none()
                            .fill(Color32::from_rgb(9, 12, 16))
                            .stroke(Stroke::new(1.0, ui::BORDER))
                            .inner_margin(Margin::same(10.0))
                            .show(ui, |ui| {
                                egui::ScrollArea::vertical()
                                    .max_height(160.0)
                                    .auto_shrink([false, false])
                                    .show(ui, |ui| {
                                        if m.changelog.trim().is_empty() {
                                            ui.label(
                                                RichText::new("Không có ghi chú thay đổi.")
                                                    .size(11.0)
                                                    .color(ui::TEXT_MUTED),
                                            );
                                        } else {
                                            for raw in m.changelog.lines() {
                                                let trimmed = raw.trim();
                                                if trimmed.is_empty() {
                                                    ui.add_space(3.0);
                                                    continue;
                                                }
                                                if let Some(h) = trimmed.strip_prefix('#') {
                                                    let heading = h.trim_start_matches('#').trim();
                                                    ui.add_space(4.0);
                                                    ui.label(
                                                        RichText::new(heading)
                                                            .font(FontId::proportional(12.0))
                                                            .strong()
                                                            .color(ui::ACCENT),
                                                    );
                                                } else if let Some(bullet) = trimmed.strip_prefix("- ") {
                                                    ui.horizontal(|ui| {
                                                        ui.spacing_mut().item_spacing.x = 4.0;
                                                        ui.add_space(4.0);
                                                        ui.label(
                                                            RichText::new("–")
                                                                .size(11.0)
                                                                .color(ui::TEXT_MUTED),
                                                        );
                                                        ui.label(
                                                            RichText::new(bullet)
                                                                .size(11.0)
                                                                .color(ui::TEXT_MAIN),
                                                        );
                                                    });
                                                } else {
                                                    ui.label(
                                                        RichText::new(trimmed)
                                                            .size(11.0)
                                                            .color(ui::TEXT_MUTED),
                                                    );
                                                }
                                            }
                                        }
                                    });
                            });
                    }
                });
                ui.add_space(ui::SPACE_CARD_GAP);
            }
            update::UpdateState::Downloading { done, total } => {
                let mb_done = *done as f64 / (1024.0 * 1024.0);
                let text = if *total > 0 {
                    let mb_total = *total as f64 / (1024.0 * 1024.0);
                    let pct = (*done as f64 * 100.0 / *total as f64).clamp(0.0, 100.0);
                    format!("Đang tải {:.1} / {:.1} MB ({:.0}%)", mb_done, mb_total, pct)
                } else {
                    format!("Đang tải {:.1} MB", mb_done)
                };

                let ver_suffix = self
                    .sh
                    .update_manifest
                    .lock()
                    .unwrap()
                    .as_ref()
                    .map(|m| format!(" bản {}", m.version))
                    .unwrap_or_default();

                ui::banner_card(ui, ui::DATA_CYAN, Color32::from_rgb(15, 60, 55), |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        ui.label(RichText::new("⬆").size(14.0).strong().color(ui::DATA_CYAN));
                        ui.label(
                            RichText::new(format!("Đang tải{ver_suffix}…"))
                                .size(13.0)
                                .strong()
                                .color(ui::TEXT_MAIN),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(text)
                                    .size(11.5)
                                    .monospace()
                                    .color(ui::DATA_CYAN),
                            );
                        });
                    });
                    ui.add_space(8.0);
                    let ratio = if *total > 0 {
                        (*done as f32 / *total as f32).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    ui::segmented_bar(ui, ratio);
                });
                ui.add_space(ui::SPACE_CARD_GAP);
            }
            update::UpdateState::Ready(_) => {
                let ver_str = self
                    .sh
                    .update_manifest
                    .lock()
                    .unwrap()
                    .as_ref()
                    .map(|m| format!(" bản {}", m.version))
                    .unwrap_or_default();

                ui::banner_card(ui, ui::SUCCESS, Color32::from_rgb(20, 70, 40), |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        ui.label(RichText::new("✔").size(14.0).strong().color(ui::SUCCESS));
                        ui.label(
                            RichText::new(format!(
                                "Đã tải xong{} — Paroda GameMode sẽ tự đóng, cài rồi mở lại.",
                                ver_str
                            ))
                            .size(12.0)
                            .strong()
                            .color(ui::TEXT_MAIN),
                        );

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .add(ui::primary_button_sized("Cài và khởi động lại", 11.5))
                                .clicked()
                            {
                                match update::apply_update(&self.sh) {
                                    Ok(()) => self.update_err = None,
                                    Err(msg) => {
                                        self.sh.log(format!("  ⚠ [UPDATE] {msg}"), ui::DANGER);
                                        self.update_err = Some(msg);
                                    }
                                }
                            }
                        });
                    });

                    if let Some(err) = self.update_err.as_ref() {
                        ui.add_space(6.0);
                        ui.label(RichText::new(format!("⚠ {err}")).size(11.5).color(ui::WARNING));
                    }
                });
                ui.add_space(ui::SPACE_CARD_GAP);
            }
            update::UpdateState::Failed(msg) if self.update_download_started => {
                ui::banner_card(ui, ui::DANGER, Color32::from_rgb(80, 25, 25), |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        ui.label(RichText::new("⚠").size(14.0).strong().color(ui::DANGER));
                        ui.label(
                            RichText::new(format!("Lỗi tải bản cập nhật: {msg}"))
                                .size(12.0)
                                .color(ui::DANGER),
                        );

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;

                            if ui.add(ui::primary_button_sized("Thử lại", 11.5)).clicked() {
                                self.update_err = None;
                                update::start_download(self.sh.clone());
                            }

                            if ui.add(ui::ghost_button_sized("Để sau", 11.5)).clicked() {
                                self.update_download_started = false;
                            }
                        });
                    });
                });
                ui.add_space(ui::SPACE_CARD_GAP);
            }
            update::UpdateState::UpToDate => {
                self.update_download_started = false;
            }
            _ => {}
        }
    }

    /// Banner "phiên Game Boost bị bỏ lửng": `session.json` còn nhưng không có game nào trong
    /// danh sách đang chạy (app bị tắt giữa phiên, Wi-Fi/service có thể vẫn đang tắt). Nằm cùng
    /// chỗ với banner cập nhật OTA, màu cảnh báo. "Để sau" chỉ ẩn banner tới lần mở sau.
    fn render_orphan_banner(&mut self, ui: &mut egui::Ui) {
        if self.orphan_dismissed || !self.sh.orphan_session.load(Ordering::SeqCst) {
            return;
        }
        // Số app / service và giờ lưu lấy từ chính session chưa khôi phục.
        let Some(s) = self.sh.session.lock().unwrap().clone() else {
            return;
        };
        let busy = self.sh.busy.load(Ordering::SeqCst);

        ui::banner_card(ui, ui::WARNING, Color32::from_rgb(80, 55, 15), |ui| {
            // horizontal_wrapped: câu dài tự xuống dòng trong bề rộng cột phải.
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(RichText::new("⚠").size(14.0).strong().color(ui::WARNING));
                ui.label(
                    RichText::new(format!(
                        "Còn phiên Game Boost chưa khôi phục (lưu lúc {}, {} app, {} service). Wi-Fi/service có thể vẫn đang tắt.",
                        time_part(&s.saved_at),
                        s.apps.len(),
                        s.services.len()
                    ))
                    .size(12.0)
                    .strong()
                    .color(ui::TEXT_MAIN),
                );
            });
            ui.add_space(8.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if ui
                    .add_enabled(!busy, ui::primary_button_sized("Khôi phục ngay", 11.5))
                    .clicked()
                {
                    // Giữ nhật ký: người dùng cần đọc log của phiên bị bỏ lửng.
                    restore_keep_log(self.sh.clone());
                }
                if ui.add(ui::ghost_button_sized("Để sau", 11.5)).clicked() {
                    self.orphan_dismissed = true;
                }
            });
        });
        ui.add_space(ui::SPACE_CARD_GAP);
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.start_hidden {
            self.start_hidden = false;
            let hwnd = self.hwnd;
            if self.tray.is_none() || hwnd == 0 {
                // Không có icon khay thì ẩn đi sẽ không mở lại được -> chỉ thu nhỏ xuống taskbar.
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            } else {
                // --minimized: vào thẳng khay (cửa sổ ẩn) chứ không thu xuống taskbar.
                // eframe tự hiện cửa sổ sau frame đầu, nên ẩn lại thêm vài lần trong giây đầu.
                sys::hide_main_window(hwnd);
                std::thread::spawn(move || {
                    for ms in [150u64, 400, 900] {
                        std::thread::sleep(Duration::from_millis(ms));
                        sys::hide_main_window(hwnd);
                    }
                });
            }
        }
        if self.last_mem.elapsed() > Duration::from_millis(800) {
            self.mem = sys::memory();
            self.last_mem = Instant::now();
        }
        // Danh sách mục khởi động đọc ở thread nền: lấy kết quả khi xong.
        self.poll_startup();
        // Tab "Tối ưu Win": kết quả quét phần cứng và việc bật/hoàn tác chạy ở thread nền.
        self.poll_hw();
        self.poll_tweaks();
        // Danh sách trang bộ nhớ (free / standby) — rẻ, chỉ đọc khi thẻ RAM cần hiển thị.
        if self.last_ml.elapsed() > Duration::from_millis(2000) {
            self.mem_lists = sys::memory_lists();
            self.last_ml = Instant::now();
        }
        // Ở nền = mất focus (game đang ở trước) hoặc thu nhỏ / ở khay: vẽ lại thưa, tắt nhấp nháy.
        let (focused, minimized) = ctx.input(|i| {
            let vp = i.viewport();
            (vp.focused.unwrap_or(true), vp.minimized.unwrap_or(false))
        });
        let background = minimized || !focused;
        ui::set_animate(!background);
        // Trạng thái task "GameMode" hỏi lại định kỳ (schtasks là tiến trình riêng, khá tốn),
        // chỉ khi đang xem cửa sổ: không spawn tiến trình mỗi 20 giây trong lúc chơi game.
        if !background && self.last_task_check.elapsed() > Duration::from_secs(20) {
            self.task_on = sys::task_exists();
            self.last_task_check = Instant::now();
        }
        // Commit từ sys::commit() cho thẻ Pagefile (tối đa mỗi 2 giây).
        if self.last_commit_sys.elapsed() > Duration::from_millis(2000) {
            self.commit_sys = sys::commit();
            self.last_commit_sys = Instant::now();
        }
        // Với trận đang ghi: repaint mỗi ~2 giây khi tab Báo cáo đang mở.
        // Thu nhỏ / ở khay: 2 giây; mất focus (đang chơi game): 1 giây.
        if minimized
            || (self.tab == ConfigTab::Reports && self.sh.live_report.lock().unwrap().is_some())
        {
            ctx.request_repaint_after(Duration::from_secs(2));
        } else if background {
            ctx.request_repaint_after(Duration::from_secs(1));
        } else {
            ctx.request_repaint_after(Duration::from_millis(400));
        }
        // Repaint nhanh khi đang kiểm tra hoặc tải cập nhật OTA
        let update_st = self.sh.update_state();
        if matches!(
            update_st,
            update::UpdateState::Downloading { .. } | update::UpdateState::Checking
        ) {
            ctx.request_repaint_after(Duration::from_millis(100));
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
                // Thoát thật: chốt báo cáo trận đang ghi và lưu NGAY (trận dài chưa tới lúc
                // watcher chốt vẫn còn số liệu). Bản lưu tạm là lưới an toàn cho crash/tắt máy.
                finish_live_report(&sh);
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
                "Free {:.1} · Standby {:.1} GB",
                l.free_bytes as f64 / GB,
                l.standby_bytes as f64 / GB
            )
        });

        let kept = {
            let keep = sh.keep.lock().unwrap();
            apps_snapshot.iter().filter(|a| app_row_kept(&keep, *a)).count()
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
                        restore(sh.clone(), false);
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

                // Banner phiên Game Boost bị bỏ lửng (session.json còn, không có game nào chạy)
                self.render_orphan_banner(ui);

                // Banner cập nhật OTA (nếu có bản mới, đang tải, đã tải xong, hoặc lỗi tải)
                self.render_update_banner(ui);

                // Thanh tab ở trên cùng: `horizontal_wrapped` để 5 tab tự xuống dòng khi cửa sổ hẹp
                // (cửa sổ mặc định 1240px không đủ cho 5 tab trên một hàng).
                ui.horizontal_wrapped(|ui| {
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

                    let is_startup = self.tab == ConfigTab::Startup;
                    let startup_on = self.startup_view.iter().filter(|e| e.enabled).count();
                    let startup_tab_btn = ui::tab_button(
                        if self.startup_loaded {
                            format!("🚀 Khởi động ({}/{})", startup_on, self.startup_view.len())
                        } else {
                            "🚀 Khởi động".to_string()
                        },
                        is_startup,
                        ui::ACCENT,
                    );
                    if ui.add(startup_tab_btn).clicked() {
                        self.tab = ConfigTab::Startup;
                        // Đọc khi mở tab lần đầu (đọc ở thread nền vì registry + .lnk hơi chậm).
                        if !self.startup_loaded {
                            self.start_startup_scan();
                        }
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

                    let is_win = self.tab == ConfigTab::WinOpt;
                    let win_tab_btn = ui::tab_button("🔧 Tối ưu Win", is_win, ui::ACCENT);
                    if ui.add(win_tab_btn).clicked() {
                        self.tab = ConfigTab::WinOpt;
                        // Mở tab lần đầu: quét phần cứng + đọc trạng thái tinh chỉnh (thread nền).
                        if !self.hw_loaded {
                            self.start_hw_scan();
                        }
                        self.ensure_tweaks();
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
                                        restart_as_admin(&sh);
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
                    ConfigTab::Startup => {
                        self.render_startup_tab(ui);
                    }
                    ConfigTab::AutoOpt => {
                        let is_wide = ui.available_width() >= 560.0;
                        egui::ScrollArea::vertical()
                            .id_salt("auto_opt_scroll_area")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                // Bề rộng thật của tab, lấy TRƯỚC `ui.columns`: khi một cột có nội
                                // dung rộng hơn phần chia, `columns` nới max_rect của `ui` và các
                                // card full width bên dưới bị lấn ra ngoài mép phải.
                                let full_w = ui.available_width();
                                let mut save_needed = false;
                                // Bản sao hồ sơ riêng: card sửa bản sao, cuối frame ghi lại vào
                                // `Shared` + config khi có thay đổi.
                                let mut profiles_view = sh.profiles.lock().unwrap().clone();
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
                                            &mut self.open_profile,
                                            &mut profiles_view,
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
                                        &mut self.open_profile,
                                        &mut profiles_view,
                                    );
                                    ui.add_space(ui::SPACE_CARD_GAP);
                                    render_optimize_card(
                                        ui,
                                        &mut opts_view,
                                        &mut save_needed,
                                    );
                                }

                                // Cập nhật OTA: luôn nằm dưới cùng, phủ hết chiều rộng tab.
                                ui.add_space(ui::SPACE_CARD_GAP);
                                ui.allocate_ui_with_layout(
                                    vec2(full_w, 0.0),
                                    egui::Layout::top_down(egui::Align::Min),
                                    |ui| {
                                        ui.set_max_width(full_w);
                                        render_update_card(
                                            ui,
                                            &sh,
                                            &mut opts_view,
                                            &mut save_needed,
                                            &mut self.update_err,
                                        );

                                        // Phím tắt toàn cục (bật/tắt + đổi tổ hợp) — dưới card cập nhật.
                                        ui.add_space(ui::SPACE_CARD_GAP);
                                        self.render_hotkey_card(ui, &mut opts_view, &mut save_needed);
                                    },
                                );

                                if save_needed {
                                    *sh.opts.lock().unwrap() = opts_view;
                                    *sh.profiles.lock().unwrap() = profiles_view;
                                    self.save();
                                }
                            });
                    }
                    ConfigTab::WinOpt => {
                        self.render_winopt_tab(ui);
                    }
                    ConfigTab::Reports => {
                        self.render_reports_tab(ui);
                    }
                }
            });

        ui::handle_window_resize_and_border(ctx);
    }
}

/// `Shared` tối thiểu cho việc chạy không có UI (`--restore`): mọi lựa chọn đọc từ config,
/// không có thread watcher / cập nhật nào chạm vào.
fn headless_shared(cfg: Config) -> Shared {
    Shared {
        log: Arc::new(Mutex::new(Vec::new())),
        busy: Arc::new(AtomicBool::new(false)),
        running: Arc::new(Mutex::new(Vec::new())),
        session: Arc::new(Mutex::new(None)),
        keep: Arc::new(Mutex::new(cfg.keep)),
        svc: Arc::new(Mutex::new(cfg.svcs)),
        games: Arc::new(Mutex::new(cfg.games)),
        game_paths: Arc::new(Mutex::new(cfg.game_paths)),
        opts: Arc::new(Mutex::new(cfg.opts)),
        // Rỗng = mọi entry của session đều được mở lại (như mặc định trong UI).
        restore_sel: Arc::new(Mutex::new(HashMap::new())),
        is_admin: sys::is_admin(),
        last_auto: Arc::new(AtomicBool::new(false)),
        opt_count: Arc::new(AtomicUsize::new(0)),
        live_report: Arc::new(Mutex::new(None)),
        ping_host: Arc::new(Mutex::new(cfg.ping_host)),
        commit_alert: Arc::new(AtomicBool::new(false)),
        commit_pct: Arc::new(Mutex::new(0.0)),
        update_state: Arc::new(Mutex::new(update::UpdateState::Idle)),
        update_manifest: Arc::new(Mutex::new(None)),
        update_last_check: Arc::new(Mutex::new(None)),
        update_request: Arc::new(AtomicBool::new(false)),
        update_url: Arc::new(Mutex::new(cfg.update_url)),
        // Đang khôi phục chính là đang xử lý phiên bỏ lửng.
        orphan_session: Arc::new(AtomicBool::new(true)),
        resume_session: Arc::new(AtomicBool::new(false)),
        profiles: Arc::new(Mutex::new(cfg.profiles)),
        hotkeys: Arc::new(Mutex::new(cfg.hotkeys)),
        hotkey_bindings: Arc::new(Mutex::new(Vec::new())),
    }
}

/// `--restore`: chạy đúng logic khôi phục của `do_restore` mà không mở UI.
/// Exit code: 0 = khôi phục xong không bước nào lỗi; 1 = đang chơi game / không có session /
/// có bước lỗi. Trả về (exit code, nội dung file `restore.txt`).
fn restore_cli() -> (i32, String) {
    let cfg = load_config();
    let mut lines: Vec<String> = vec![
        format!("Paroda GameMode --restore | {}", sys::local_time_string()),
        format!("admin={}", sys::is_admin()),
    ];
    // Đang chơi thì KHÔNG khôi phục (sẽ mở lại app và bật service giữa trận).
    let up = running_games(&cfg.games);
    let mut code = 1;
    if !up.is_empty() {
        lines.push(format!(
            "TỪ CHỐI: game trong danh sách đang chạy ({}). Tắt game rồi chạy lại --restore.",
            up.join(", ")
        ));
    } else {
        let sh = headless_shared(cfg);
        let restored = do_restore(&sh, false);
        let logs: Vec<String> = sh
            .log
            .lock()
            .unwrap()
            .iter()
            .map(|(text, _)| text.clone())
            .collect();
        // Bước nào lỗi thì `do_restore` ghi dòng có '⚠' — tính là thất bại cho exit code.
        let warns = logs.iter().filter(|l| l.contains('⚠')).count();
        lines.extend(logs);
        if !restored {
            lines.push(
                "KHÔNG khôi phục được: không có session.json (không có phiên nào đang chờ).".into(),
            );
        } else if warns > 0 {
            lines.push(format!(
                "KẾT QUẢ: khôi phục xong nhưng có {} bước lỗi (các dòng ⚠ ở trên).",
                warns
            ));
        } else {
            lines.push("KẾT QUẢ: khôi phục xong, không có bước nào lỗi.".into());
            code = 0;
        }
    }
    lines.push(String::new());
    (code, lines.join("\n"))
}

/// Chạy `hwinfo::SCRIPT` bằng PowerShell ẩn cửa sổ và trả NGUYÊN VĂN output
/// (phần "JSON thô" của `--hwinfo`; `hwinfo::collect` chỉ trả struct đã parse).
#[cfg(windows)]
fn hw_script_raw() -> Result<String, String> {
    use std::os::windows::process::CommandExt;

    let out = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            hwinfo::SCRIPT,
        ])
        .creation_flags(0x0800_0000)
        .output()
        .map_err(|e| format!("chạy powershell lỗi: {}", e))?;
    let text = sys::decode_win_output(&out.stdout);
    if text.trim().is_empty() {
        let err = sys::decode_win_output(&out.stderr);
        let err = err.trim();
        return Err(if err.is_empty() {
            format!("powershell không trả dữ liệu (mã {})", out.status)
        } else {
            format!("powershell lỗi: {}", err)
        });
    }
    Ok(text)
}

#[cfg(not(windows))]
fn hw_script_raw() -> Result<String, String> {
    Err("chỉ đọc được phần cứng trên Windows".to_string())
}

/// `--hwinfo`: đọc phần cứng (WMI/PowerShell) và ghi `hwinfo.txt`:
/// dòng tóm tắt, từng gợi ý, rồi JSON thô. Exit code: 0 = đọc được, 1 = lỗi.
fn hwinfo_cli() -> (i32, String) {
    let mut lines: Vec<String> = vec![
        format!("Paroda GameMode --hwinfo | {}", sys::local_time_string()),
        format!("admin={}", sys::is_admin()),
    ];
    let raw = match hw_script_raw() {
        Ok(raw) => raw,
        Err(e) => {
            lines.push(format!("⚠ {}", e));
            return (1, lines.join("\n"));
        }
    };
    let mut code = 0;
    match hwinfo::parse(&raw) {
        Ok(hw) => {
            lines.push(format!("TÓM TẮT: {}", hwinfo::summary_line(&hw)));
            lines.push(String::new());
            let advice = hwinfo::advice(&hw);
            lines.push(format!("GỢI Ý ({}):", advice.len()));
            for a in advice {
                let level = match a.level {
                    hwinfo::Level::Good => "TỐT",
                    hwinfo::Level::Info => "TIN",
                    hwinfo::Level::Warn => "CẢNH BÁO",
                };
                lines.push(format!("[{}] {} — {}", level, a.title, a.detail));
            }
        }
        Err(e) => {
            lines.push(format!("⚠ parse JSON phần cứng lỗi: {}", e));
            code = 1;
        }
    }
    lines.push(String::new());
    lines.push("=== JSON THÔ ===".to_string());
    lines.push(raw);
    (code, lines.join("\n"))
}

/// `--tweakstatus`: ghi trạng thái từng tinh chỉnh Windows ra `wintweaks-status.txt`,
/// mỗi dòng `id | group | risk | state | có backup`.
/// Exit code: 0 = đọc được ít nhất một trạng thái, 1 = không đọc được cái nào.
fn tweakstatus_cli() -> (i32, String) {
    let cfg = load_config();
    let tweaks = wintweaks::all_tweaks(&game_dirs_from_paths(&cfg.game_paths));
    let path = wintweaks_backup_path();
    let backup = wintweaks::Backup::load(&path);
    let mut lines: Vec<String> = vec![
        format!("Paroda GameMode --tweakstatus | {}", sys::local_time_string()),
        format!("admin={}", sys::is_admin()),
        format!("backup={} ({} mục)", path.display(), backup.ids().len()),
    ];
    let mut unknown = 0usize;
    for t in &tweaks {
        let st = wintweaks::state(t);
        let known = !matches!(st, wintweaks::TweakState::Unknown(_));
        if !known {
            unknown += 1;
        }
        lines.push(format!(
            "{} | {} | {} | {} | {}",
            t.id,
            t.group.label(),
            t.risk.label(),
            st.label(),
            if backup.has(t.id) { "có" } else { "không" }
        ));
    }
    lines.push(format!(
        "tổng={} đọc_được={} không_rõ={}",
        tweaks.len(),
        tweaks.len() - unknown,
        unknown
    ));
    let code = if tweaks.is_empty() || unknown == tweaks.len() {
        1
    } else {
        0
    };
    (code, lines.join("\n"))
}

/// `--tweakapply <id>` / `--tweakrevert <id>`: bật/hoàn tác MỘT tinh chỉnh bằng đúng hàm và
/// đúng file sao lưu như UI, ghi `wintweaks-result.txt`. Exit code: 0 = xong, 1 = lỗi.
fn tweak_cli(id: &str, apply: bool) -> (i32, String) {
    let cfg = load_config();
    let tweaks = wintweaks::all_tweaks(&game_dirs_from_paths(&cfg.game_paths));
    let path = wintweaks_backup_path();
    let mut backup = wintweaks::Backup::load(&path);
    let mut lines: Vec<String> = vec![
        format!(
            "Paroda GameMode --tweak{} {} | {}",
            if apply { "apply" } else { "revert" },
            id,
            sys::local_time_string()
        ),
        format!("admin={}", sys::is_admin()),
        format!("backup={}", path.display()),
    ];
    let Some(t) = tweaks.iter().find(|t| t.id == id) else {
        let ids: Vec<&str> = tweaks.iter().map(|t| t.id).collect();
        lines.push(format!(
            "⚠ Không có tinh chỉnh id “{}”. Id hợp lệ: {}",
            id,
            ids.join(", ")
        ));
        return (1, lines.join("\n"));
    };
    lines.push(format!("trước: {}", wintweaks::state(t).label()));
    let result = if apply {
        wintweaks::apply(t, &mut backup)
    } else {
        wintweaks::revert(t, &mut backup)
    };
    match result {
        Ok(()) => {
            lines.push(if apply {
                format!("✔ [WIN] Đã bật: {}", t.title)
            } else {
                format!("↺ [WIN] Đã hoàn tác: {}", t.title)
            });
            let mut code = 0;
            match backup.save(&path) {
                Ok(()) => lines.push(format!("đã ghi {}", path.display())),
                Err(e) => {
                    lines.push(format!("⚠ ghi bản sao lưu lỗi: {}", e));
                    code = 1;
                }
            }
            lines.push(format!("sau: {}", wintweaks::state(t).label()));
            lines.push(format!(
                "trong bản sao lưu: {}",
                if backup.has(t.id) { "có" } else { "không" }
            ));
            (code, lines.join("\n"))
        }
        Err(e) => {
            lines.push(format!("⚠ [WIN] {}: {}", t.title, e));
            (1, lines.join("\n"))
        }
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

    // --restore: khôi phục phiên Game Boost bị bỏ lửng mà KHÔNG mở UI (trường hợp app đã bị tắt
    // giữa phiên, session.json còn lại). Từ chối nếu còn game trong danh sách đang chạy. Kết quả
    // từng bước (đúng các dòng log của `do_restore`) ghi ra restore.txt cạnh exe.
    // Exit code: 0 = đã khôi phục, 1 = lỗi hoặc không có session.
    if std::env::args().any(|a| a == "--restore") {
        let (code, text) = restore_cli();
        let _ = std::fs::write(config_path().with_file_name("restore.txt"), text);
        std::process::exit(code);
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
        let apps = scan_running_apps(&skip, &cfg.games, &cfg.keep);
        lines.insert(0, format!("processes={} cwd_ok={} apps_listed={}", snap.len(), cwd_ok, apps.len()));
        for a in apps.iter() {
            lines.push(format!(
                "APP	{}	{}	{}	kept_children={}	kept_by={}",
                a.exe_name,
                a.count,
                fmt_ram(a.total_ws),
                a.kept_children,
                a.kept_by.as_deref().unwrap_or("-")
            ));
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
        let live = report::start("test", &[], &cfg.ping_host, 2, 0);
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

    // --updatetest: kiểm tra manifest cập nhật, và nếu có bản mới thì tải + kiểm SHA-256
    // (KHÔNG chạy bộ cài, không cần admin) rồi ghi updatetest.txt.
    if std::env::args().any(|a| a == "--updatetest") {
        let url = load_config().update_url;
        let text = format!("{}\n", update::self_test(&url));
        let _ = std::fs::write(config_path().with_file_name("updatetest.txt"), text);
        return Ok(());
    }

    // --hwinfo: đọc phần cứng (WMI/PowerShell) và ghi hwinfo.txt (tóm tắt, gợi ý, JSON thô).
    if std::env::args().any(|a| a == "--hwinfo") {
        let (code, text) = hwinfo_cli();
        let _ = std::fs::write(config_path().with_file_name("hwinfo.txt"), text);
        std::process::exit(code);
    }

    // --tweakstatus: trạng thái từng tinh chỉnh Windows -> wintweaks-status.txt.
    if std::env::args().any(|a| a == "--tweakstatus") {
        let (code, text) = tweakstatus_cli();
        let _ = std::fs::write(config_path().with_file_name("wintweaks-status.txt"), text);
        std::process::exit(code);
    }

    // --tweakapply <id> / --tweakrevert <id>: bật/hoàn tác một tinh chỉnh (cùng hàm, cùng file
    // sao lưu như UI) -> wintweaks-result.txt. Exit code 0/1.
    let argv: Vec<String> = std::env::args().collect();
    if let Some(pos) = argv
        .iter()
        .position(|a| a == "--tweakapply" || a == "--tweakrevert")
    {
        let apply = argv[pos] == "--tweakapply";
        let id = argv.get(pos + 1).cloned().unwrap_or_default();
        let (code, text) = tweak_cli(&id, apply);
        let _ = std::fs::write(config_path().with_file_name("wintweaks-result.txt"), text);
        std::process::exit(code);
    }

    // --purgetest: xả standby list một lần, ghi free/standby trước & sau ra purgetest.txt.
    // --tasktest: tạo task "GameMode", đọc lại XML, rồi xoá (chỉ để kiểm tra cách quote đường dẫn).
    // Task đã có sẵn (người dùng đang bật chạy cùng Windows) thì CHỈ đọc, không tạo/xoá: bản cũ
    // từng xoá mất task thật của người dùng khi bước tạo bị lỗi.
    if std::env::args().any(|a| a == "--tasktest") {
        if sys::task_exists() {
            use std::os::windows::process::CommandExt;
            let xml = std::process::Command::new("schtasks")
                .args(["/Query", "/TN", "GameMode", "/XML"])
                .creation_flags(0x0800_0000)
                .output()
                .map(|o| sys::decode_win_output(&o.stdout))
                .unwrap_or_default();
            let _ = std::fs::write(
                config_path().with_file_name("tasktest.txt"),
                format!(
                    "task GameMode đã tồn tại — chỉ đọc, không tạo/xoá\nneeds_repair={}\n{}",
                    sys::task_needs_repair(),
                    xml
                ),
            );
            return Ok(());
        }
        let created = sys::task_create();
        use std::os::windows::process::CommandExt;
        let xml = std::process::Command::new("schtasks")
            .args(["/Query", "/TN", "GameMode", "/XML"])
            .creation_flags(0x0800_0000)
            .output()
            .map(|o| sys::decode_win_output(&o.stdout))
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

    // --fpstest [giây]: đo FPS / frametime của game đầu tiên trong danh sách đang chạy (cần admin),
    // ghi kết quả ra fpstest.txt.
    if std::env::args().any(|a| a == "--fpstest") {
        let secs = std::env::args()
            .skip_while(|a| a != "--fpstest")
            .nth(1)
            .and_then(|a| a.parse().ok())
            .unwrap_or(10);
        let cfg = load_config();
        // Mọi tiến trình trùng tên game: có app vẽ hình từ tiến trình con (vd GPU process).
        let procs: Vec<sys::Proc> = sys::list_processes()
            .into_iter()
            .filter(|p| cfg.games.iter().any(|g| g.eq_ignore_ascii_case(&p.name)))
            .collect();
        let text = match procs.first() {
            Some(p) => {
                let pids: Vec<u32> = procs.iter().map(|p| p.pid).collect();
                format!("game = {}
{}", p.name, fps::self_test(&pids, secs))
            }
            None => "Không có game nào trong danh sách đang chạy.".to_string(),
        };
        let _ = std::fs::write(config_path().with_file_name("fpstest.txt"), text);
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

    // Vừa cài bản cập nhật ở lần chạy trước (`update-pending.json`): ghi nhận kết quả và
    // LUÔN hiện cửa sổ — kể cả khi task "chạy cùng Windows" gọi với `--minimized`.
    let just_updated = update::check_pending_on_start();
    if !just_updated && std::env::args().any(|a| a == "--minimized") {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_game_name() {
        assert_eq!(clean_game_name("DeltaForceClient-Win64-Shipping.exe"), "Delta Force");
        assert_eq!(clean_game_name("deltaforceclient-win64-shipping.exe"), "Delta Force");
        assert_eq!(clean_game_name("DELTAFORCECLIENT-WIN64-SHIPPING"), "Delta Force");
        assert_eq!(clean_game_name("DeltaForceClient.exe"), "Delta Force");
        assert_eq!(clean_game_name("test"), "test");
        assert_eq!(clean_game_name("ShooterGame-Win64-Shipping.exe"), "ShooterGame");
        assert_eq!(clean_game_name("Game-Shipping.exe"), "Game");
        assert_eq!(clean_game_name("cs2.exe"), "cs2");
        assert_eq!(clean_game_name("cs2.EXE"), "cs2");
    }

    #[test]
    fn test_streaming_and_ssh_are_kept() {
        for exe in ["discord.exe", "discordsystemhelper.exe", "discordptb.exe", "obs64.exe", "claude.exe"] {
            assert!(default_keep(exe), "{exe} phải được giữ mặc định");
        }
        assert!(!default_keep("chrome.exe"));
        assert!(
            SERVICE_TARGETS.iter().all(|(name, ..)| !name.eq_ignore_ascii_case("sshd")),
            "Game Boost không được dừng OpenSSH Server"
        );
    }

    /// App hệ thống / app luôn chạy KHÔNG BAO GIỜ được nằm trong danh sách game: bản cũ có
    /// `game:msedgewebview2.exe=1` nên WebView2 (luôn chạy) làm Boost chạy ngay khi mở app.
    #[test]
    fn test_not_games_blocks_always_on_apps() {
        for exe in [
            "msedgewebview2.exe",
            "msedge.exe",
            "chrome.exe",
            "explorer.exe",
            "svchost.exe",
            "searchhost.exe",
            "taskmgr.exe",
            "steam.exe",
            "obs64.exe",
            "claude.exe",
            "node.exe",
            "gamemode.exe",
            "system",
            "registry",
            "discord.exe",
            "discordsystemhelper.exe",
        ] {
            assert!(is_not_game(exe), "{exe} phải bị chặn khỏi danh sách game");
        }
        // Tên hoa cũng bị chặn sau khi hạ chữ thường (config và UI đều hạ trước khi gọi).
        assert!(is_not_game(&"MSEDGEWEBVIEW2.EXE".to_lowercase()));
        for exe in [
            "deltaforceclient-win64-shipping.exe",
            "cs2.exe",
            "game-client-win64-shipping.exe",
        ] {
            assert!(!is_not_game(exe), "{exe} vẫn phải được coi là game");
        }
    }

    /// Dòng `game:` là app hệ thống bị bỏ qua lúc đọc config (không nạp vào danh sách), tên exe
    /// được ghi lại vào `skipped_games` để báo log; lần ghi config sau sẽ tự xoá dòng đó.
    #[test]
    fn test_parse_config_skips_not_games() {
        let cfg = parse_config(
            "game:msedgewebview2.exe=1\ngame:DeltaForceClient-Win64-Shipping.exe=1\n\
             game:discord.exe=1\nkeep:steam.exe=1\n",
        );
        assert_eq!(cfg.games, vec!["deltaforceclient-win64-shipping.exe".to_string()]);
        assert!(cfg.skipped_games.contains(&"msedgewebview2.exe".to_string()));
        assert!(cfg.skipped_games.contains(&"discord.exe".to_string()));
        assert!(!cfg.skipped_games.contains(&"deltaforceclient-win64-shipping.exe".to_string()));

        // Chỉ có dòng bị chặn -> không nạp game nào, rơi về danh sách game mặc định.
        let only_bad = parse_config("game:explorer.exe=1\n");
        assert_eq!(only_bad.games, DEFAULT_GAMES.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(only_bad.skipped_games, vec!["explorer.exe".to_string()]);
    }

    /// Giờ lưu trong banner phiên bỏ lửng lấy từ `saved_at` của session.
    #[test]
    fn test_time_part() {
        assert_eq!(time_part("2026-10-22 18:50:12"), "18:50");
        assert_eq!(time_part(""), "");
    }

    #[test]
    fn test_truncate_game_name() {
        assert_eq!(truncate_game_name("Delta Force", 16), "Delta Force");
        assert_eq!(truncate_game_name("test", 16), "test");
        assert_eq!(truncate_game_name("SuperUltraMegaLongGameTitle", 16), "SuperUltraMegaL…");
    }

    #[test]
    fn test_format_plot_time() {
        assert_eq!(format_plot_time(0.0), "0:00");
        assert_eq!(format_plot_time(90.0), "1:30");
        assert_eq!(format_plot_time(3725.0), "1:02:05");
        assert_eq!(format_plot_time(20.0), "0:20");
        assert_eq!(format_plot_time(70.0), "1:10");
        assert_eq!(format_plot_time(3600.0), "1:00:00");
    }

    #[test]
    fn test_pick_time_step() {
        let durations = [20.0, 30.0, 70.0, 90.0, 180.0, 300.0, 600.0, 1800.0, 3600.0, 5400.0, 7200.0];
        for &dur in &durations {
            let step = pick_time_step(dur);
            let label_count = (dur / step).floor() as usize + 1;
            assert!(
                label_count >= 4 && label_count <= 8,
                "dur={}s, step={}s, count={}",
                dur,
                step,
                label_count
            );
        }
    }

    fn node(pid: u32, ppid: u32, name: &str) -> sys::ProcNode {
        sys::ProcNode {
            pid,
            ppid,
            name: name.to_string(),
        }
    }

    fn kept_names(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    /// Cây giả: claude.exe -> node.exe -> node.exe, claude.exe -> cua-driver.exe;
    /// node.exe khác do explorer.exe mở KHÔNG được giữ; tổ tiên của claude.exe cũng không.
    /// Instance nào là con cháu của app được giữ thì giữ, instance khác theo quy tắc cũ.
    #[test]
    fn test_kept_family_from_nodes_descendants() {
        let nodes = vec![
            node(50, 4, "explorer.exe"),
            node(100, 50, "claude.exe"),
            node(101, 100, "node.exe"),
            node(102, 101, "node.exe"),
            node(103, 100, "cua-driver.exe"),
            node(200, 50, "node.exe"),
        ];
        let kept = kept_family_from_nodes(&nodes, &kept_names(&["claude.exe"]));

        assert_eq!(kept.len(), 3, "chỉ 3 process con cháu được giữ: {:?}", kept);
        assert_eq!(kept.get(&101).map(String::as_str), Some("claude.exe"));
        assert_eq!(kept.get(&102).map(String::as_str), Some("claude.exe"), "cháu cũng được giữ");
        assert_eq!(kept.get(&103).map(String::as_str), Some("claude.exe"));
        assert!(!kept.contains_key(&200), "node.exe do explorer mở không được giữ");
        assert!(!kept.contains_key(&100), "chính app được giữ không nằm trong map con cháu");
        assert!(!kept.contains_key(&50), "tổ tiên của app được giữ không được bảo vệ");
    }

    /// Vòng ppid giả (A là cha B, B là cha A) không được treo; ppid trỏ tới PID không tồn tại
    /// thì chỉ đơn giản là hết nhánh.
    #[test]
    fn test_kept_family_from_nodes_cycle_and_missing_ppid() {
        let nodes = vec![
            node(10, 11, "a.exe"),
            node(11, 10, "b.exe"),
            node(12, 999, "c.exe"),
            node(13, 12, "d.exe"),
        ];
        let kept = kept_family_from_nodes(&nodes, &kept_names(&["a.exe"]));

        assert_eq!(kept.len(), 1, "chỉ b.exe là con của a.exe: {:?}", kept);
        assert_eq!(kept.get(&11).map(String::as_str), Some("a.exe"));
        assert!(!kept.contains_key(&10), "a.exe không phải con của chính nó");
        assert!(!kept.contains_key(&12) && !kept.contains_key(&13), "nhánh ppid 999 không được giữ");

        // Không có app nào được giữ -> map rỗng, không duyệt gì.
        assert!(kept_family_from_nodes(&nodes, &HashSet::new()).is_empty());
    }

    /// Dòng log tóm tắt: số process + app cha + tên exe con kèm số instance.
    #[test]
    fn test_kept_family_log_line() {
        let nodes = vec![
            node(100, 50, "claude.exe"),
            node(101, 100, "node.exe"),
            node(102, 100, "node.exe"),
            node(103, 100, "cua-driver.exe"),
        ];
        let kept = kept_family_from_nodes(&nodes, &kept_names(&["claude.exe"]));
        let line = kept_family_log_line(&kept, &nodes).expect("phải có dòng log");
        assert!(line.contains("Giữ 3 process con"), "{}", line);
        assert!(line.contains("claude.exe: node.exe ×2, cua-driver.exe"), "{}", line);

        assert!(kept_family_log_line(&HashMap::new(), &nodes).is_none());
    }

    /// Đếm "x/y" trên tab: dòng có cả instance được giữ lẫn instance sẽ tắt KHÔNG tính là giữ.
    #[test]
    fn test_app_row_kept_mixed_instances() {
        let make = |count: usize, kept_children: usize| RunningApp {
            exe_lower: "node.exe".into(),
            exe_name: "node.exe".into(),
            display: None,
            exe_path: None,
            count,
            total_ws: 0,
            kept_children,
            kept_by: Some("Claude".into()),
        };
        let keep: HashMap<String, bool> = HashMap::new();
        assert!(app_row_kept(&keep, &make(3, 3)), "mọi instance đều là con của app được giữ");
        assert!(!app_row_kept(&keep, &make(3, 1)), "còn instance sẽ tắt thì không tính là giữ");
        assert!(app_row_kept(&HashMap::from([("node.exe".to_string(), true)]), &make(3, 1)));
    }

    /// `game_dirs` cho tinh chỉnh `defender_games`: thư mục cha của mọi `gamepath:` đã bỏ trùng.
    #[test]
    fn test_game_dirs_from_paths() {
        let paths = HashMap::from([
            (
                "deltaforceclient-win64-shipping.exe".to_string(),
                r"D:\Games\Delta Force\DeltaForceClient-Win64-Shipping.exe".to_string(),
            ),
            (
                "cs2.exe".to_string(),
                r"D:\Games\Delta Force\cs2.exe".to_string(),
            ),
            ("steam.exe".to_string(), "steam.exe".to_string()),
        ]);
        let dirs = game_dirs_from_paths(&paths);
        assert_eq!(dirs, vec![r"D:\Games\Delta Force".to_string()]);
        assert!(game_dirs_from_paths(&HashMap::new()).is_empty());
    }
}
