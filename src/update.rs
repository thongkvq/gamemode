//! Tự cập nhật OTA từ server nội bộ (`update-dhcs.paroda.net`).
//!
//! Luồng: thread nền GET `manifest.json` (timeout 10 giây) -> so version server với
//! `CARGO_PKG_VERSION` (biến `GAMEMODE_FAKE_VERSION` để thử) -> nếu có bản mới thì UI
//! hiện nút, tải bộ cài vào `%TEMP%\GameMode-update\`, kiểm `size` + `sha256`, rồi khi
//! người dùng bấm cài: ghi `update-pending.json`, chạy bộ cài qua `cmd.exe` có trễ (để app
//! kịp thoát trước khi Inno Setup kiểm `AppMutex`) và thoát app như menu khay "Thoát".
//!
//! Mọi lỗi mạng/đĩa đều biến thành trạng thái `Failed` + một dòng log: không bao giờ
//! panic và không bao giờ chặn UI (mọi việc nặng chạy ở thread riêng).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{sys, ui, Shared};

/// URL manifest mặc định (dùng khi `gamemode.cfg` chưa ghi `update:url`).
pub const DEFAULT_MANIFEST_URL: &str = "https://update-dhcs.paroda.net/apps/gamemode/manifest.json";
/// Chờ sau khi app mở trước lần kiểm tra đầu tiên.
pub const FIRST_CHECK_DELAY: Duration = Duration::from_secs(20);
/// Chu kỳ kiểm tra lại.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
/// Timeout cho mỗi request HTTP (manifest và bộ cài).
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);
/// Trần kích thước manifest đọc vào RAM.
const MAX_MANIFEST_BYTES: u64 = 256 * 1024;
/// Trần kích thước bộ cài tải về (chặn server trả về file rác vô hạn).
const MAX_PACKAGE_BYTES: u64 = 1024 * 1024 * 1024;
/// File đánh dấu "vừa chạy bộ cài" nằm cạnh exe (cùng chỗ `gamemode.cfg`).
pub const PENDING_FILE: &str = "update-pending.json";
/// Tên thư mục tải trong `%TEMP%`.
const TEMP_DIR_NAME: &str = "GameMode-update";
/// Chu kỳ thread nền thức dậy để xem có yêu cầu kiểm tra ngay từ UI không.
const TICK: Duration = Duration::from_secs(1);

const MB: f64 = 1024.0 * 1024.0;

/// Nội dung `manifest.json` trên server.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Manifest {
    pub app: String,
    pub channel: String,
    pub version: String,
    /// Version tối thiểu còn được cập nhật lên bản này (chỉ để tham khảo/log).
    pub minimum_version: String,
    pub filename: String,
    pub download_url: String,
    /// Kích thước bộ cài theo byte (đối chiếu sau khi tải).
    pub size: u64,
    /// SHA-256 hex thường của bộ cài.
    pub sha256: String,
    pub changelog: String,
    pub pub_date: String,
}

/// Nội dung `update-pending.json`: ghi trước khi chạy bộ cài, xoá khi app mở lại.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct PendingUpdate {
    /// Version lúc bắt đầu chạy bộ cài.
    pub from: String,
    /// Version đích ghi trong manifest.
    pub to: String,
}

/// Trạng thái cập nhật: thread nền ghi, UI đọc.
#[derive(Clone, Debug)]
pub enum UpdateState {
    /// Chưa kiểm tra lần nào, hoặc tính năng đang tắt.
    Idle,
    /// Đang GET manifest.
    Checking,
    /// Đang dùng bản mới nhất.
    UpToDate,
    /// Server có bản mới hơn.
    Available(Manifest),
    /// Đang tải bộ cài (byte đã nhận / tổng theo manifest, 0 nếu manifest không ghi).
    Downloading { done: u64, total: u64 },
    /// Đã tải xong, đã kiểm size + sha256, sẵn sàng chạy bộ cài.
    Ready(PathBuf),
    /// Lỗi (mạng, manifest, kích thước, sha256...). Một câu tiếng Việt.
    Failed(String),
}

impl UpdateState {
    /// Một dòng trạng thái cho UI.
    pub fn label(&self) -> String {
        match self {
            UpdateState::Idle => "Chưa kiểm tra".to_string(),
            UpdateState::Checking => "Đang kiểm tra…".to_string(),
            UpdateState::UpToDate => {
                format!("Đã là bản mới nhất ({})", current_version())
            }
            UpdateState::Available(m) => format!("Có bản mới {}", m.version),
            UpdateState::Downloading { done, total } => {
                if *total > 0 {
                    format!(
                        "Đang tải {:.0}% — {:.1}/{:.1} MB",
                        *done as f64 * 100.0 / *total as f64,
                        *done as f64 / MB,
                        *total as f64 / MB
                    )
                } else {
                    format!("Đang tải {:.1} MB", *done as f64 / MB)
                }
            }
            UpdateState::Ready(p) => format!(
                "Đã tải xong {}",
                p.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "bộ cài".to_string())
            ),
            UpdateState::Failed(e) => format!("Lỗi: {e}"),
        }
    }
}

// ==========================================
// VERSION
// ==========================================

/// Version đang chạy: `GAMEMODE_FAKE_VERSION` (nếu đặt) thay cho version đã build.
pub fn current_version() -> String {
    match std::env::var("GAMEMODE_FAKE_VERSION") {
        Ok(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => env!("CARGO_PKG_VERSION").to_string(),
    }
}

/// Parse "major.minor.patch" (cho phép thiếu phần sau, cho phép tiền tố `v` và hậu tố
/// `-beta`/`+build`). Trả `None` nếu không có số nào.
pub fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let core = text.trim().trim_start_matches(['v', 'V']);
    let core = core.split(['-', '+']).next().unwrap_or(core);
    let mut nums = [0u64; 3];
    let mut parts = 0usize;
    for part in core.split('.') {
        let digits: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            return None;
        }
        if parts < 3 {
            nums[parts] = digits.parse().ok()?;
        }
        parts += 1;
    }
    if parts == 0 {
        None
    } else {
        Some((nums[0], nums[1], nums[2]))
    }
}

/// Version server có mới hơn version đang chạy không (so số học major.minor.patch).
pub fn is_newer(remote: &str, current: &str) -> bool {
    match (parse_version(remote), parse_version(current)) {
        (Some(r), Some(c)) => r > c,
        _ => false,
    }
}

/// `a` >= `b` theo version; nếu không parse được thì so chuỗi.
fn at_least(a: &str, b: &str) -> bool {
    match (parse_version(a), parse_version(b)) {
        (Some(x), Some(y)) => x >= y,
        _ => a.trim() == b.trim(),
    }
}

// ==========================================
// HTTP
// ==========================================

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(HTTP_TIMEOUT)
        .user_agent(concat!("GameMode/", env!("CARGO_PKG_VERSION")))
        .build()
}

/// GET manifest và parse JSON. Chỉ chấp nhận URL `https://`.
pub fn fetch_manifest(url: &str) -> Result<Manifest, String> {
    if !url.starts_with("https://") {
        return Err(format!(
            "URL manifest phải bắt đầu bằng https:// (config ghi {url:?})"
        ));
    }
    let resp = agent()
        .get(url)
        .call()
        .map_err(|e| format!("không gọi được {url}: {e}"))?;
    if resp.status() != 200 {
        return Err(format!("{url} trả về HTTP {}", resp.status()));
    }
    let mut body = String::new();
    resp.into_reader()
        .take(MAX_MANIFEST_BYTES + 1)
        .read_to_string(&mut body)
        .map_err(|e| format!("đọc manifest lỗi: {e}"))?;
    if body.len() as u64 > MAX_MANIFEST_BYTES {
        return Err("manifest quá lớn (trên 256 KB)".to_string());
    }
    // Bỏ BOM nếu server ghi file kèm BOM (serde_json sẽ báo lỗi cú pháp).
    let body = body.trim_start_matches('\u{feff}');
    let m: Manifest =
        serde_json::from_str(body).map_err(|e| format!("manifest không phải JSON hợp lệ: {e}"))?;
    if m.version.trim().is_empty() {
        return Err("manifest thiếu trường version".to_string());
    }
    Ok(m)
}

/// Thư mục tải: `%TEMP%\GameMode-update`.
fn update_temp_dir() -> PathBuf {
    let base = std::env::var("TEMP")
        .ok()
        .filter(|t| !t.trim().is_empty())
        .or_else(|| std::env::var("TMP").ok().filter(|t| !t.trim().is_empty()))
        .unwrap_or_else(|| std::env::temp_dir().to_string_lossy().to_string());
    PathBuf::from(base).join(TEMP_DIR_NAME)
}

/// Tên file an toàn: chỉ lấy phần cuối đường dẫn, bỏ qua `..`, ký tự điều khiển và các ký tự
/// có nghĩa đặc biệt với `cmd.exe` (`"`, `%`, `&`, `|`, `<`, `>`, `^`) — tên này được nhúng
/// vào dòng lệnh chạy bộ cài. Nhận cả URL (tự cắt sau dấu `/`).
fn safe_filename(raw: &str) -> Option<String> {
    let cleaned = raw.trim().replace('\\', "/");
    let base = cleaned.rsplit('/').next().unwrap_or("").trim().to_string();
    if base.is_empty() || base == "." || base == ".." {
        return None;
    }
    if base
        .chars()
        .any(|c| c.is_control() || matches!(c, '"' | '%' | '&' | '|' | '<' | '>' | '^'))
    {
        return None;
    }
    Some(base)
}

/// SHA-256 (hex thường) của một file, đọc theo khối 64 KB.
fn sha256_file(path: &Path) -> Result<String, String> {
    let mut f =
        std::fs::File::open(path).map_err(|e| format!("không mở được {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f
            .read(&mut buf)
            .map_err(|e| format!("lỗi đọc {}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Tải bộ cài vào `%TEMP%\GameMode-update\<filename>` rồi kiểm `size` và `sha256`.
/// Sai thì xoá file và trả lỗi. `progress(done, total)` gọi sau mỗi khối đọc được.
fn download_to(m: &Manifest, mut progress: impl FnMut(u64, u64)) -> Result<PathBuf, String> {
    if !m.download_url.starts_with("https://") {
        return Err(format!(
            "download_url phải bắt đầu bằng https:// (manifest ghi {:?})",
            m.download_url
        ));
    }
    let name = safe_filename(&m.filename)
        .or_else(|| safe_filename(&m.download_url))
        .ok_or_else(|| "manifest không có tên file hợp lệ".to_string())?;
    let dir = update_temp_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("không tạo được {}: {e}", dir.display()))?;
    let path = dir.join(&name);

    let resp = agent()
        .get(&m.download_url)
        .call()
        .map_err(|e| format!("không tải được {}: {e}", m.download_url))?;
    let total = if m.size > 0 {
        m.size
    } else {
        resp.header("content-length")
            .and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap_or(0)
    };

    let mut reader = resp.into_reader();
    let mut file =
        std::fs::File::create(&path).map_err(|e| format!("không ghi được {}: {e}", path.display()))?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut done: u64 = 0;
    loop {
        let n = match reader.read(&mut buf) {
            Ok(n) => n,
            Err(e) => {
                let _ = std::fs::remove_file(&path);
                return Err(format!("lỗi khi tải bộ cài: {e}"));
            }
        };
        if n == 0 {
            break;
        }
        done += n as u64;
        if done > MAX_PACKAGE_BYTES {
            let _ = std::fs::remove_file(&path);
            return Err(format!(
                "bộ cài lớn hơn trần {} MB — dừng tải",
                MAX_PACKAGE_BYTES / (1024 * 1024)
            ));
        }
        if let Err(e) = file.write_all(&buf[..n]) {
            let _ = std::fs::remove_file(&path);
            return Err(format!("lỗi khi ghi {}: {e}", path.display()));
        }
        progress(done, total);
    }
    if let Err(e) = file.flush() {
        let _ = std::fs::remove_file(&path);
        return Err(format!("lỗi khi ghi {}: {e}", path.display()));
    }
    drop(file);

    let got = std::fs::metadata(&path).map(|md| md.len()).unwrap_or(0);
    if m.size > 0 && got != m.size {
        let _ = std::fs::remove_file(&path);
        return Err(format!(
            "file tải về {got} byte nhưng manifest ghi {} byte — đã xoá file",
            m.size
        ));
    }
    if m.sha256.trim().is_empty() {
        let _ = std::fs::remove_file(&path);
        return Err("manifest không có sha256 — không cài được, đã xoá file".to_string());
    }
    let hash = sha256_file(&path)?;
    if !hash.eq_ignore_ascii_case(m.sha256.trim()) {
        let _ = std::fs::remove_file(&path);
        return Err(format!(
            "SHA-256 không khớp (tải về {hash}, manifest {}) — đã xoá file",
            m.sha256.trim()
        ));
    }
    progress(done, total.max(done));
    Ok(path)
}

// ==========================================
// TRẠNG THÁI DÙNG CHUNG + THREAD NỀN
// ==========================================

/// Ghi trạng thái mới (đi qua đây để `update_manifest` luôn khớp trạng thái).
fn set_state(sh: &Shared, state: UpdateState) {
    if let UpdateState::Available(m) = &state {
        *sh.update_manifest.lock().unwrap() = Some(m.clone());
    }
    *sh.update_state.lock().unwrap() = state;
}

/// UI bấm "Kiểm tra cập nhật": thread nền chạy ngay (không chặn UI).
pub fn request_check(sh: &Shared) {
    sh.update_request.store(true, Ordering::SeqCst);
}

/// Thread nền: kiểm tra 20 giây sau khi app mở, rồi mỗi 6 giờ; có yêu cầu từ UI thì chạy luôn.
pub fn start_update_thread(sh: Shared) {
    let _ = std::thread::Builder::new().name("update".to_string()).spawn(move || {
        let mut next = Instant::now() + FIRST_CHECK_DELAY;
        loop {
            let asked = sh.update_request.swap(false, Ordering::SeqCst);
            if asked || Instant::now() >= next {
                next = Instant::now() + CHECK_INTERVAL;
                check_once(&sh);
            }
            std::thread::sleep(TICK);
        }
    });
}

/// Một lần kiểm tra: tắt tính năng thì về `Idle`, lỗi thì `Failed` + đúng một dòng log.
fn check_once(sh: &Shared) {
    if !sh.opts().update_enabled {
        set_state(sh, UpdateState::Idle);
        return;
    }
    let url = sh.update_url.lock().unwrap().clone();
    set_state(sh, UpdateState::Checking);
    match fetch_manifest(&url) {
        Ok(m) => {
            *sh.update_last_check.lock().unwrap() = Some(Instant::now());
            let cur = current_version();
            if is_newer(&m.version, &cur) {
                sh.log(
                    format!("  ↑ [UPDATE] Có bản mới {} — đang dùng {}.", m.version, cur),
                    ui::ACCENT,
                );
                set_state(sh, UpdateState::Available(m));
            } else {
                // Bản mới nhất: chỉ đổi trạng thái, không ghi log (tránh rác mỗi 6 giờ).
                set_state(sh, UpdateState::UpToDate);
            }
        }
        Err(e) => {
            *sh.update_last_check.lock().unwrap() = Some(Instant::now());
            sh.log(
                format!("  ⚠ [UPDATE] Không kiểm tra được cập nhật: {e}"),
                ui::DANGER,
            );
            set_state(sh, UpdateState::Failed(e));
        }
    }
}

/// UI bấm "Cập nhật lên x.y.z": tải bộ cài ở thread nền, cập nhật tiến độ vào trạng thái.
pub fn start_download(sh: Shared) {
    let Some(m) = sh.update_manifest.lock().unwrap().clone() else {
        set_state(
            &sh,
            UpdateState::Failed("Chưa có thông tin bản cập nhật — bấm \"Kiểm tra cập nhật\" trước.".to_string()),
        );
        return;
    };
    let _ = std::thread::Builder::new().name("update-download".to_string()).spawn(move || {
        set_state(&sh, UpdateState::Downloading { done: 0, total: m.size });
        let progress_sh = sh.clone();
        let result = download_to(&m, move |done, total| {
            let mut st = progress_sh.update_state.lock().unwrap();
            if matches!(*st, UpdateState::Downloading { .. }) {
                *st = UpdateState::Downloading { done, total };
            }
        });
        match result {
            Ok(path) => {
                sh.log(
                    format!(
                        "  ✔ [UPDATE] Đã tải và kiểm SHA-256 xong: {}",
                        path.display()
                    ),
                    ui::SUCCESS,
                );
                set_state(&sh, UpdateState::Ready(path));
            }
            Err(e) => {
                sh.log(
                    format!("  ⚠ [UPDATE] Tải bộ cài thất bại: {e}"),
                    ui::DANGER,
                );
                set_state(&sh, UpdateState::Failed(e));
            }
        }
    });
}

// ==========================================
// CÀI ĐẶT
// ==========================================

/// Đường dẫn `update-pending.json` (cạnh exe, cùng chỗ `gamemode.cfg`).
pub fn pending_path() -> PathBuf {
    crate::config_path().with_file_name(PENDING_FILE)
}

/// Game trong danh sách config đang chạy — cùng cách watcher phát hiện (chỉ so tên exe,
/// không mở handle vào game / anti-cheat).
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

/// Chạy bộ cài qua `cmd.exe` có trễ ~3 giây (`ping -n 4`) để GameMode kịp thoát trước khi
/// Inno Setup kiểm tra `AppMutex`; console ẩn hoàn toàn (`CREATE_NO_WINDOW`).
fn spawn_setup(path: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let setup = path.to_string_lossy().to_string();
    let cmd_exe = std::env::var("SystemRoot")
        .map(|r| format!("{r}\\System32\\cmd.exe"))
        .unwrap_or_else(|_| r"C:\Windows\System32\cmd.exe".to_string());
    // `/UPDATE`: bộ cài tự mở lại GameMode sau khi cài xong (xem `[Code]` trong
    // installer/gamemode.iss). `start "" "<setup>"` để không chờ bộ cài.
    let line = format!(
        "/c ping -n 4 127.0.0.1 >nul & start \"\" \"{setup}\" /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /UPDATE"
    );
    let mut cmd = std::process::Command::new(&cmd_exe);
    // raw_arg: không để Rust tự thêm cặp nháy bao quanh làm hỏng cú pháp của cmd.exe.
    cmd.raw_arg(&line);
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd.spawn()
        .map(|_| ())
        .map_err(|e| format!("không chạy được bộ cài {}: {e}", path.display()))
}

/// Thoát hẳn y như menu khay "Thoát": cờ `QUIT` để `App::update` gỡ icon khay rồi đóng app.
fn quit_app() {
    crate::tray::QUIT.store(true, Ordering::SeqCst);
    let hwnd = sys::find_window(crate::WINDOW_TITLE);
    sys::post_close(hwnd);
    // Dự phòng: không còn frame nào chạy thì thoát cứng (giống `tray::quit`).
    let _ = std::thread::Builder::new().name("quit".to_string()).spawn(|| {
        std::thread::sleep(Duration::from_millis(1500));
        std::process::exit(0);
    });
}

/// Cài bản đã tải xong: từ chối khi đang chơi game / đang Game Boost, ghi `update-pending.json`,
/// chạy bộ cài rồi thoát app. Trả lỗi tiếng Việt rõ ràng cho UI.
pub fn apply_update(sh: &Shared) -> Result<(), String> {
    let path = match sh.update_state.lock().unwrap().clone() {
        UpdateState::Ready(p) => p,
        _ => return Err("Chưa tải xong bộ cài — bấm \"Cập nhật\" trước.".to_string()),
    };
    if !path.is_file() {
        return Err(format!("Không thấy file cài đặt {}.", path.display()));
    }
    if sh.busy.load(Ordering::SeqCst) {
        return Err("GameMode đang tối ưu hoặc khôi phục — đợi xong rồi cập nhật.".to_string());
    }
    let running = running_games(&sh.games.lock().unwrap().clone());
    if !running.is_empty() {
        return Err(format!(
            "Đang chơi {} — thoát game rồi mới cập nhật được.",
            running.iter().map(|g| crate::clean_game_name(g)).collect::<Vec<_>>().join(", ")
        ));
    }
    // Còn session.json = phiên Game Boost chưa khôi phục.
    if crate::session_path().is_file() {
        return Err("Đang trong phiên Game Boost — bấm \"Khôi phục\" trước khi cập nhật.".to_string());
    }

    let from = current_version();
    let to = sh
        .update_manifest
        .lock()
        .unwrap()
        .as_ref()
        .map(|m| m.version.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| from.clone());
    let text = serde_json::to_string_pretty(&PendingUpdate {
        from: from.clone(),
        to: to.clone(),
    })
    .map_err(|e| format!("không tạo được nội dung {PENDING_FILE}: {e}"))?;
    std::fs::write(pending_path(), text)
        .map_err(|e| format!("không ghi được {}: {e}", pending_path().display()))?;

    spawn_setup(&path)?;
    sh.log(
        format!(
            "  ↑ [UPDATE] Đang cài {to} — GameMode thoát rồi bộ cài tự mở lại bản mới."
        ),
        ui::ACCENT,
    );
    quit_app();
    Ok(())
}

/// Dòng log của lần cập nhật vừa hoàn tất, chờ `App::new` đưa vào nhật ký UI.
static STARTUP_LOG: Mutex<Option<(String, bool)>> = Mutex::new(None);

/// Đọc `update-pending.json` lúc khởi động: so version hiện tại với version đích, ghi nhận
/// dòng log rồi XOÁ file. Trả `true` nếu vừa có cập nhật chờ xử lý (main phải hiện cửa sổ
/// kể cả khi chạy với `--minimized`).
pub fn check_pending_on_start() -> bool {
    let Some(result) = pending_result() else {
        return false;
    };
    *STARTUP_LOG.lock().unwrap() = Some(result);
    true
}

/// Lấy (và xoá) dòng log cập nhật đã ghi nhận lúc khởi động.
pub fn take_startup_log() -> Option<(String, bool)> {
    STARTUP_LOG.lock().unwrap().take()
}

fn pending_result() -> Option<(String, bool)> {
    let text = std::fs::read_to_string(pending_path()).ok()?;
    let p: PendingUpdate = serde_json::from_str(&text).unwrap_or_default();
    let _ = std::fs::remove_file(pending_path());
    let now = current_version();
    let ok = at_least(&now, &p.to);
    Some(if ok {
        (format!("✔ Đã cập nhật {} → {}", p.from, p.to), true)
    } else {
        (
            format!(
                "⚠ Cập nhật {} → {} không thành công — vẫn đang chạy {}",
                p.from, p.to, now
            ),
            false,
        )
    })
}

// ==========================================
// --updatetest
// ==========================================

/// Cờ `--updatetest`: kiểm tra manifest, in version, và nếu có bản mới thì tải + kiểm SHA-256
/// (KHÔNG chạy bộ cài). Trả nội dung file `updatetest.txt`.
pub fn self_test(url: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("admin={}", sys::is_admin()));
    lines.push(format!("url manifest = {url}"));
    lines.push(format!(
        "version đã build (CARGO_PKG_VERSION) = {}",
        env!("CARGO_PKG_VERSION")
    ));
    if let Ok(fake) = std::env::var("GAMEMODE_FAKE_VERSION") {
        if !fake.trim().is_empty() {
            lines.push(format!("GAMEMODE_FAKE_VERSION = {fake}"));
        }
    }
    let cur = current_version();
    lines.push(format!("version dùng để so sánh = {cur}"));
    lines.push(format!("thư mục tải = {}", update_temp_dir().display()));

    match fetch_manifest(url) {
        Ok(m) => {
            lines.push(format!("version server = {}", m.version));
            lines.push(format!(
                "app={} channel={} minimum_version={} pub_date={}",
                m.app, m.channel, m.minimum_version, m.pub_date
            ));
            lines.push(format!(
                "file={} size={} sha256(manifest)={}",
                m.filename, m.size, m.sha256
            ));
            let newer = is_newer(&m.version, &cur);
            lines.push(format!("có bản mới hơn {cur} = {newer}"));
            if !newer {
                lines.push("kết quả: không cần tải (đang là bản mới nhất hoặc server cũ hơn)".to_string());
                return lines.join("\n");
            }
            match download_to(&m, |_, _| {}) {
                Ok(path) => {
                    lines.push(format!("tải về = {}", path.display()));
                    match sha256_file(&path) {
                        Ok(hash) => {
                            let size = std::fs::metadata(&path).map(|md| md.len()).unwrap_or(0);
                            lines.push(format!("size tải về = {size}"));
                            lines.push(format!("sha256 tải về = {hash}"));
                            lines.push(format!(
                                "khớp manifest = {}",
                                hash.eq_ignore_ascii_case(m.sha256.trim())
                            ));
                        }
                        Err(e) => lines.push(format!("LỖI đọc lại file: {e}")),
                    }
                }
                Err(e) => lines.push(format!("LỖI tải/kiểm tra: {e}")),
            }
        }
        Err(e) => lines.push(format!("LỖI manifest: {e}")),
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Manifest mẫu đúng như server nội bộ trả về.
    /// (r##"..."## vì trong changelog có chuỗi `"#` sẽ đóng r#"..."# sớm.)
    const SAMPLE: &str = r##"{
  "app": "GameMode",
  "channel": "stable",
  "version": "1.3.0",
  "minimum_version": "1.0.0",
  "filename": "GameMode-Setup-1.3.0.exe",
  "download_url": "https://update-dhcs.paroda.net/apps/gamemode/releases/GameMode-Setup-1.3.0.exe",
  "size": 4321000,
  "sha256": "9f2c4b7d1e5a60834c2f7b9d0e1a3c5f7b9d0e1a3c5f7b9d0e1a3c5f7b9d0e1a",
  "changelog": "# 1.3.0 - ...\n\n- ...",
  "pub_date": "2026-10-22T05:00:00Z"
}"##;

    #[test]
    fn test_parse_version() {
        assert_eq!(parse_version("1.3.0"), Some((1, 3, 0)));
        assert_eq!(parse_version(" 1.2 "), Some((1, 2, 0)));
        assert_eq!(parse_version("v2.10.3"), Some((2, 10, 3)));
        assert_eq!(parse_version("1.3.0-beta.1"), Some((1, 3, 0)));
        assert_eq!(parse_version("1"), Some((1, 0, 0)));
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("abc"), None);
        assert_eq!(parse_version("1.x.0"), None);
    }

    #[test]
    fn test_is_newer() {
        assert!(is_newer("1.3.0", "1.2.0"));
        assert!(is_newer("1.2.1", "1.2.0"));
        assert!(is_newer("2.0.0", "1.9.9"));
        assert!(!is_newer("1.2.0", "1.2.0"));
        assert!(!is_newer("1.1.9", "1.2.0"));
        assert!(!is_newer("rác", "1.2.0"));
        assert!(!is_newer("1.3.0", "rác"));
    }

    #[test]
    fn test_parse_manifest_sample() {
        let m: Manifest = serde_json::from_str(SAMPLE).expect("manifest mẫu phải parse được");
        assert_eq!(m.app, "GameMode");
        assert_eq!(m.channel, "stable");
        assert_eq!(m.version, "1.3.0");
        assert_eq!(m.minimum_version, "1.0.0");
        assert_eq!(m.filename, "GameMode-Setup-1.3.0.exe");
        assert_eq!(m.size, 4_321_000);
        assert_eq!(m.sha256.len(), 64);
        assert!(m.sha256.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(m.download_url.starts_with("https://"));
        assert_eq!(m.pub_date, "2026-10-22T05:00:00Z");
        assert!(m.changelog.contains("1.3.0"));
        assert!(is_newer(&m.version, "1.2.0"));
        assert!(!is_newer(&m.version, "1.3.0"));
    }

    #[test]
    fn test_parse_manifest_thieu_truong() {
        // Manifest thiếu trường vẫn parse được (mặc định rỗng) nhưng version rỗng thì fetch_manifest từ chối.
        let m: Manifest = serde_json::from_str(r#"{"app":"GameMode"}"#).unwrap();
        assert_eq!(m.version, "");
        assert_eq!(m.size, 0);
    }

    #[test]
    fn test_safe_filename() {
        assert_eq!(safe_filename("GameMode-Setup-1.3.0.exe").as_deref(), Some("GameMode-Setup-1.3.0.exe"));
        assert_eq!(
            safe_filename("https://update-dhcs.paroda.net/apps/gamemode/releases/GameMode-Setup-1.3.0.exe").as_deref(),
            Some("GameMode-Setup-1.3.0.exe")
        );
        assert_eq!(safe_filename(r"C:\tmp\x.exe").as_deref(), Some("x.exe"));
        assert_eq!(safe_filename("../../etc/passwd").as_deref(), Some("passwd"));
        assert_eq!(safe_filename(".."), None);
        assert_eq!(safe_filename("   "), None);
        // Ký tự đặc biệt của cmd.exe không được lọt vào dòng lệnh chạy bộ cài.
        assert_eq!(safe_filename("a\"b.exe"), None);
        assert_eq!(safe_filename("a%TEMP%b.exe"), None);
        assert_eq!(safe_filename("a&b.exe"), None);
    }

    #[test]
    fn test_pending_update_json() {
        let p: PendingUpdate = serde_json::from_str(r#"{"from":"1.2.0","to":"1.3.0"}"#).unwrap();
        assert_eq!(p.from, "1.2.0");
        assert_eq!(p.to, "1.3.0");
        assert_eq!(
            serde_json::to_string(&p).unwrap(),
            r#"{"from":"1.2.0","to":"1.3.0"}"#
        );
        assert!(at_least("1.3.0", "1.3.0"));
        assert!(at_least("1.3.0", "1.2.0"));
        assert!(!at_least("1.2.0", "1.3.0"));
    }

    #[test]
    fn test_state_label() {
        assert_eq!(UpdateState::Idle.label(), "Chưa kiểm tra");
        assert_eq!(UpdateState::Checking.label(), "Đang kiểm tra…");
        assert_eq!(UpdateState::Available(Manifest {
            version: "1.3.0".into(),
            ..Default::default()
        })
        .label(), "Có bản mới 1.3.0");
        assert_eq!(
            UpdateState::Downloading { done: 1_000_000, total: 4_000_000 }.label(),
            "Đang tải 25% — 1.0/3.8 MB"
        );
        assert!(UpdateState::Failed("mất mạng".into()).label().contains("mất mạng"));
    }
}
