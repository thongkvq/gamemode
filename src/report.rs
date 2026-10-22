//! Báo cáo trận đấu: ghi số liệu RAM / commit / CPU / ping trong lúc chơi,
//! tính nhận xét rồi lưu JSON vào thư mục `reports/` cạnh exe.
//!
//! Mọi số liệu đều là TOÀN HỆ THỐNG (`sys::memory`, `sys::commit`, `GetSystemTimes`,
//! ICMP tới gateway / internet): không mở handle tới process game hay anti-cheat.
//!
//! Vòng đời do watcher ở `main.rs` điều khiển: `start` khi game xuất hiện,
//! `push_event` trong lúc chơi, `finish` khi game tắt.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::{fps, sys};

/// Số báo cáo giữ lại trên đĩa (mới nhất trước, file cũ hơn bị xoá).
pub const KEEP_REPORTS: usize = 30;
/// Trận ngắn hơn mức này thì không lưu file.
pub const MIN_DURATION_SEC: u64 = 60;
/// Chu kỳ lấy mẫu trong lúc chơi.
pub const SAMPLE_SEC: u64 = 5;
/// Đích ping internet mặc định khi config không ghi `report:ping_host`.
pub const PING_HOST_DEFAULT: &str = "1.1.1.1";

/// Thời gian chờ mỗi gói ICMP.
const PING_TIMEOUT_MS: u32 = 1000;
/// Gửi 1 gói mỗi giây cho mỗi đích.
const PING_GAP: Duration = Duration::from_secs(1);
/// Ping internet vượt mức này (ms) coi là "vọt".
const SPIKE_MS: f32 = 150.0;
/// Hai event ping_spike cách nhau tối thiểu.
const SPIKE_MIN_GAP: Duration = Duration::from_secs(30);
/// Ngưỡng đếm `ping_spikes` trong bản tóm tắt.
const SUMMARY_SPIKE_MS: f32 = 100.0;
/// Ngưỡng commit coi là nguy hiểm (nhận xét + cảnh báo).
pub const COMMIT_WARN_PCT: f32 = 90.0;

/// Frametime lớn hơn mức này (ms) trong một mẫu thì ghi event "hitch" (khựng hình).
const HITCH_EVENT_MS: f32 = 100.0;
/// Hai event khựng cách nhau tối thiểu (giống `SPIKE_MIN_GAP` của ping).
const HITCH_MIN_GAP: Duration = Duration::from_secs(30);
/// Event `hitch` cách event `purge` không quá mức này (giây) thì coi là khựng trùng lúc xả standby.
const HITCH_PURGE_GAP_SEC: f32 = 5.0;

/// Chu kỳ ghi bản lưu tạm `reports/<id>.partial.json` trong lúc chơi: app/máy tắt giữa trận
/// (Thoát ở khay, khởi động lại, mất điện, crash) vẫn còn số liệu để khôi phục.
const PARTIAL_GAP: Duration = Duration::from_secs(60);
/// Hậu tố bản lưu tạm. Tên file thành `<id>.partial.json` nên `valid_id` (chỉ chữ số và '-')
/// tự loại nó khỏi `list_reports` / `load_report` / `delete_report`.
const PARTIAL_SUFFIX: &str = ".partial";
/// `ended_at` của báo cáo khôi phục: mốc kết thúc thật không đọc được (app đã chết giữa trận),
/// nên ghi rõ là bị cắt ngang thay vì bịa một mốc giờ.
const INTERRUPTED_ENDED_AT: &str = "(GameMode tắt giữa trận)";

const GIB: f32 = 1024.0 * 1024.0 * 1024.0;

/// Một mẫu số liệu lấy mỗi 5 giây trong lúc chơi.
#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct Sample {
    /// Giây kể từ lúc bắt đầu trận.
    pub t: f32,
    /// RAM khả dụng (GB).
    pub avail_gb: f32,
    /// % RAM vật lý đang dùng.
    pub ram_used_pct: f32,
    /// Free từ `sys::memory_lists()` (None nếu không đọc được).
    pub free_gb: Option<f32>,
    /// Standby từ `sys::memory_lists()`.
    pub standby_gb: Option<f32>,
    /// Commit đang dùng (GB).
    pub commit_gb: f32,
    /// Commit limit (GB).
    pub commit_limit_gb: f32,
    /// % commit đang dùng / limit.
    pub commit_pct: f32,
    /// % CPU toàn hệ thống trong khoảng mẫu.
    pub cpu_pct: f32,
    /// RTT trung bình tới gateway trong khoảng mẫu; None nếu mất hết.
    pub ping_gw_ms: Option<f32>,
    /// RTT trung bình tới host internet trong khoảng mẫu; None nếu mất hết.
    pub ping_net_ms: Option<f32>,
    /// RTT lớn nhất (một gói) tới host internet trong khoảng mẫu.
    pub ping_net_max_ms: Option<f32>,
    pub loss_gw_pct: f32,
    pub loss_net_pct: f32,
    /// FPS trung bình của cửa sổ mẫu; None khi không có frame nào (menu/alt-tab/không đo được).
    pub fps: Option<f32>,
    /// 1% low của cửa sổ mẫu.
    pub fps_low1: Option<f32>,
    /// Frametime phân vị 99% trong cửa sổ mẫu (ms).
    pub frametime_p99_ms: Option<f32>,
    /// Frametime lớn nhất trong cửa sổ mẫu (ms).
    pub frametime_max_ms: Option<f32>,
    /// Số frame khựng (> max(50 ms, 2,5 × trung vị)) trong cửa sổ mẫu.
    pub hitches: u32,
}

/// Sự kiện xảy ra trong trận (kind: "optimize","purge","commit_warn","ping_spike","restore").
#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct Event {
    pub t: f32,
    pub kind: String,
    pub text: String,
}

/// Bản tóm tắt cả trận.
#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct Summary {
    pub duration_sec: u64,
    pub ram_avail_min_gb: f32,
    pub ram_avail_avg_gb: f32,
    pub commit_peak_pct: f32,
    pub commit_peak_gb: f32,
    pub cpu_avg_pct: f32,
    pub cpu_peak_pct: f32,
    pub ping_net_avg_ms: Option<f32>,
    pub ping_net_p95_ms: Option<f32>,
    pub ping_net_max_ms: Option<f32>,
    /// Độ lệch trung bình giữa hai mẫu ping liên tiếp.
    pub jitter_net_ms: Option<f32>,
    pub loss_net_pct: f32,
    pub ping_gw_avg_ms: Option<f32>,
    pub loss_gw_pct: f32,
    /// Số mẫu có ping internet > 100 ms.
    pub ping_spikes: u32,
    /// FPS trung bình cả trận (chính xác từ histogram ETW khi có meter; nếu không thì tính
    /// có trọng số từ các mẫu).
    pub fps_avg: Option<f32>,
    /// 1% low cả trận.
    pub fps_low1: Option<f32>,
    /// 0,1% low cả trận — chỉ có khi chốt bằng histogram ETW.
    pub fps_low01: Option<f32>,
    /// Frametime p99 cả trận (ms) — chỉ có khi chốt bằng histogram ETW.
    pub frametime_p99_ms: Option<f32>,
    /// Frametime lớn nhất cả trận (ms).
    pub frametime_max_ms: Option<f32>,
    /// Tổng số lần khựng cả trận.
    pub hitches: u32,
    pub commit_warnings: u32,
    pub standby_purges: u32,
    /// Nhận xét tiếng Việt.
    pub verdict: Vec<String>,
}

/// Toàn bộ một trận.
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct GameReport {
    /// "20261022-213005"
    pub id: String,
    /// exe game (nhiều game cùng lúc thì nối bằng ", ").
    pub game: String,
    pub started_at: String,
    pub ended_at: String,
    pub samples: Vec<Sample>,
    pub events: Vec<Event>,
    pub summary: Summary,
}

/// Bản rút gọn cho danh sách báo cáo.
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct ReportMeta {
    pub id: String,
    pub game: String,
    pub started_at: String,
    pub duration_sec: u64,
    pub commit_peak_pct: f32,
    pub ping_net_avg_ms: Option<f32>,
    /// FPS trung bình của trận (None: trận cũ chưa đo FPS, hoặc không đo được).
    #[serde(default)]
    pub fps_avg: Option<f32>,
}

/// Thư mục `reports/` cạnh exe.
pub fn reports_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("reports")))
        .unwrap_or_else(|| PathBuf::from("reports"))
}

/// Id hợp lệ = "YYYYMMDD-HHMMSS" (chỉ chữ số và '-'): chặn id độc hại đi ra ngoài thư mục.
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 32
        && id.chars().all(|c| c.is_ascii_digit() || c == '-')
}

fn report_path(id: &str) -> PathBuf {
    reports_dir().join(format!("{}.json", id))
}

/// Đường dẫn bản lưu tạm của một trận trong thư mục `dir`.
fn partial_path_in(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{}{}.json", id, PARTIAL_SUFFIX))
}

/// Danh sách báo cáo trên đĩa, mới nhất trước (id sắp theo thời gian).
/// Bản lưu tạm (`<id>.partial.json`) KHÔNG phải một báo cáo: `valid_id` loại id chứa '.'.
pub fn list_reports() -> Vec<ReportMeta> {
    let mut out: Vec<ReportMeta> = Vec::new();
    let Ok(rd) = std::fs::read_dir(reports_dir()) else {
        return out;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if !valid_id(id) {
            continue;
        }
        let Some(report) = read_report(&path) else {
            continue;
        };
        out.push(ReportMeta {
            id: report.id,
            game: report.game,
            started_at: report.started_at,
            duration_sec: report.summary.duration_sec,
            commit_peak_pct: report.summary.commit_peak_pct,
            ping_net_avg_ms: report.summary.ping_net_avg_ms,
            fps_avg: report.summary.fps_avg,
        });
    }
    out.sort_by(|a, b| b.id.cmp(&a.id));
    out
}

/// Đọc một báo cáo theo id (`None` nếu không có / file hỏng).
pub fn load_report(id: &str) -> Option<GameReport> {
    if !valid_id(id) {
        return None;
    }
    read_report(&report_path(id))
}

/// Xoá một báo cáo. Trả `true` nếu file đã bị xoá.
/// (Tab báo cáo ở task sau dùng hàm này; hiện chưa có chỗ gọi trong app.)
#[allow(dead_code)]
pub fn delete_report(id: &str) -> bool {
    if !valid_id(id) {
        return false;
    }
    std::fs::remove_file(report_path(id)).is_ok()
}

fn read_report(path: &std::path::Path) -> Option<GameReport> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// Khôi phục các bản lưu tạm còn sót lại khi mở app (lần chạy trước bị tắt giữa trận).
/// Trả về các dòng log tiếng Việt để `main.rs` đẩy vào nhật ký.
pub fn recover_partials() -> Vec<String> {
    recover_partials_in(&reports_dir())
}

/// Như `recover_partials` nhưng nhận thẳng thư mục (để test được).
///
/// Với mỗi `<id>.partial.json`:
/// - đã có `<id>.json` bản chính -> chỉ xoá bản tạm;
/// - chưa có -> dựng lại báo cáo (độ dài = `t` của mẫu cuối, `summary` tính lại, thêm event
///   `interrupted`), lưu thành `<id>.json` rồi xoá bản tạm.
fn recover_partials_in(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };

    let mut partials: Vec<(String, PathBuf)> = Vec::new();
    for entry in rd.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Some(id) = stem.strip_suffix(PARTIAL_SUFFIX) else {
            continue;
        };
        // Id không hợp lệ: file lạ, bỏ qua (không xoá của người khác).
        if !valid_id(id) {
            continue;
        }
        partials.push((id.to_string(), path));
    }
    // Thứ tự ổn định để log khôi phục đọc theo thời gian.
    partials.sort_by(|a, b| a.0.cmp(&b.0));

    for (id, path) in partials {
        // Bản chính đã lưu xong (trận kết thúc bình thường nhưng chưa kịp xoá bản tạm).
        if dir.join(format!("{}.json", id)).is_file() {
            let _ = std::fs::remove_file(&path);
            continue;
        }
        let Some(mut report) = read_report(&path) else {
            // Bản tạm hỏng: không dựng lại được, xoá để lần sau không thử lại mãi.
            let _ = std::fs::remove_file(&path);
            continue;
        };

        // Độ dài trận = mốc của mẫu cuối cùng đã ghi được (không có mẫu thì lấy độ dài cũ).
        let duration_sec = report
            .samples
            .last()
            .map(|s| s.t.max(0.0) as u64)
            .unwrap_or(report.summary.duration_sec);

        report.id = id.clone();
        report.ended_at = INTERRUPTED_ENDED_AT.to_string();
        report.summary = summarize(&report, duration_sec);
        report.events.push(Event {
            t: duration_sec as f32,
            kind: "interrupted".to_string(),
            text: "GameMode tắt giữa trận — báo cáo khôi phục từ bản lưu tạm".to_string(),
        });

        if save_report_in(dir, &report).is_some() {
            let _ = std::fs::remove_file(&path);
            out.push(format!(
                "  • [BÁO CÁO] Khôi phục trận {} ({}) từ bản lưu tạm.",
                id,
                duration_text(duration_sec)
            ));
        }
    }
    out
}

/// Ghi báo cáo ra `reports/<id>.json` (tạo thư mục nếu chưa có).
fn save_report(report: &GameReport) -> Option<PathBuf> {
    save_report_in(&reports_dir(), report)
}

/// Ghi báo cáo ra `<dir>/<id>.json` (tạo thư mục nếu chưa có).
fn save_report_in(dir: &Path, report: &GameReport) -> Option<PathBuf> {
    if !valid_id(&report.id) {
        return None;
    }
    if std::fs::create_dir_all(dir).is_err() {
        return None;
    }
    let path = dir.join(format!("{}.json", report.id));
    let text = serde_json::to_string_pretty(report).ok()?;
    std::fs::write(&path, text).ok()?;
    Some(path)
}

/// Ghi bản lưu tạm của trận đang chơi ra `reports/<id>.partial.json`, an toàn với crash:
/// ghi ra `<id>.partial.tmp` rồi `rename` đè lên bản tạm (rename là thao tác nguyên tử nên
/// không bao giờ để lại bản tạm đọc dở).
fn save_partial(report: &GameReport) -> Option<PathBuf> {
    save_partial_in(&reports_dir(), report)
}

fn save_partial_in(dir: &Path, report: &GameReport) -> Option<PathBuf> {
    if !valid_id(&report.id) {
        return None;
    }
    if std::fs::create_dir_all(dir).is_err() {
        return None;
    }
    let text = serde_json::to_string_pretty(report).ok()?;
    let tmp = dir.join(format!("{}{}.tmp", report.id, PARTIAL_SUFFIX));
    std::fs::write(&tmp, text).ok()?;
    let path = partial_path_in(dir, &report.id);
    std::fs::rename(&tmp, &path).ok()?;
    Some(path)
}

/// Xoá bản lưu tạm của một trận (không làm gì nếu không có).
fn remove_partial(dir: &Path, id: &str) {
    if valid_id(id) {
        let _ = std::fs::remove_file(partial_path_in(dir, id));
    }
}

/// Chỉ giữ `keep` báo cáo mới nhất, xoá các file cũ hơn.
pub fn prune_reports(keep: usize) {
    let ids: Vec<String> = list_reports().into_iter().map(|m| m.id).collect();
    if ids.len() <= keep {
        return;
    }
    for id in ids.into_iter().skip(keep) {
        let _ = std::fs::remove_file(report_path(&id));
    }
}

/// Kết quả ping gom trong một khoảng mẫu.
#[derive(Clone, Copy, Default)]
struct PingBucket {
    sent: u32,
    got: u32,
    sum_ms: f64,
    max_ms: f32,
}

impl PingBucket {
    fn add(&mut self, rtt: Option<f32>) {
        self.sent += 1;
        if let Some(v) = rtt {
            self.got += 1;
            self.sum_ms += v as f64;
            if v > self.max_ms {
                self.max_ms = v;
            }
        }
    }

    /// RTT trung bình trong khoảng mẫu; `None` nếu mất hết.
    fn avg(self) -> Option<f32> {
        (self.got > 0).then(|| (self.sum_ms / self.got as f64) as f32)
    }

    /// RTT lớn nhất của MỘT gói trong khoảng mẫu.
    fn max(self) -> Option<f32> {
        (self.got > 0).then_some(self.max_ms)
    }

    fn loss_pct(self) -> f32 {
        if self.sent == 0 {
            0.0
        } else {
            (self.sent - self.got) as f32 * 100.0 / self.sent as f32
        }
    }
}

/// Kết quả ping thô của hai đích, thread ping đẩy vào, thread lấy mẫu lấy ra.
#[derive(Default)]
struct PingState {
    gw: PingBucket,
    net: PingBucket,
}

/// Một trận đang ghi.
pub struct Live {
    pub report: GameReport,
    started: Instant,
    /// Cờ dừng cho thread ping + thread lấy mẫu.
    stop: Arc<AtomicBool>,
    ping: Arc<Mutex<PingState>>,
    /// `GetSystemTimes` cần delta giữa hai lần đọc.
    cpu: sys::CpuMeter,
    /// Đích ping đã phân giải (None: không có route / không phân giải được).
    gw_target: Option<u32>,
    net_target: Option<u32>,
    /// Tên exe game (viết thường) để dò PID mỗi lần lấy mẫu.
    game_exes: Vec<String>,
    /// Meter ETW đo FPS. None khi không mở được session (thường là thiếu quyền admin).
    fps: Option<fps::FpsMeter>,
    /// Lần ghi event "hitch" gần nhất.
    last_hitch: Option<Instant>,
    sample_every: Duration,
    min_duration_sec: u64,
    /// Lần ghi event ping_spike gần nhất.
    last_spike: Option<Instant>,
    /// Giây kể từ đầu trận lúc watcher thấy game đã tắt (None: chưa tắt).
    /// Watcher chỉ chốt sau 15 giây, nên độ dài trận lấy mốc này cho khỏi cộng thêm 15 giây.
    game_end: Option<f32>,
    /// Trận đã được `finish` chốt và lưu chưa: chặn lần gọi thứ hai (watcher giữ chung `Arc`
    /// với đường thoát app) ghi đè báo cáo đã lưu.
    finished: bool,
}

/// Kết quả đóng một trận.
pub struct Outcome {
    pub id: String,
    pub saved: bool,
    pub path: Option<PathBuf>,
    pub samples: usize,
    /// Dòng tóm tắt để ghi log.
    pub line: String,
}

/// Bắt đầu ghi một trận và chạy thread ping + thread lấy mẫu.
/// `exes` = tên exe của game (khớp không phân biệt hoa thường) để dò PID đo FPS;
/// `sample_secs` = chu kỳ lấy mẫu; `min_duration_sec` = độ dài tối thiểu để lưu file
/// (0 = luôn lưu, dùng cho `--reporttest`).
pub fn start(
    game: &str,
    exes: &[String],
    ping_host: &str,
    sample_secs: u64,
    min_duration_sec: u64,
) -> Arc<Mutex<Live>> {
    // Đích internet: host trong config, không phân giải được thì lùi về mặc định.
    let net_target = sys::resolve_ipv4(ping_host)
        .or_else(|| sys::resolve_ipv4(PING_HOST_DEFAULT))
        .map(|(addr, _)| addr);

    let game_exes: Vec<String> = exes.iter().map(|e| e.to_lowercase()).collect();
    let mut events: Vec<Event> = Vec::new();
    // ETW cần quyền admin: không mở được thì ghi rõ lý do vào báo cáo (một lần) rồi ghi
    // trận bình thường, KHÔNG bịa số FPS.
    let meter = match fps::FpsMeter::start(fps::DEFAULT_SESSION) {
        Ok(m) => Some(m),
        Err(e) => {
            events.push(Event {
                t: 0.0,
                kind: "fps_off".to_string(),
                text: e,
            });
            None
        }
    };

    let live = Arc::new(Mutex::new(Live {
        report: GameReport {
            id: report_id(),
            game: game.to_string(),
            started_at: sys::local_time_string(),
            ended_at: String::new(),
            samples: Vec::new(),
            events,
            summary: Summary::default(),
        },
        started: Instant::now(),
        stop: Arc::new(AtomicBool::new(false)),
        ping: Arc::new(Mutex::new(PingState::default())),
        cpu: sys::CpuMeter::new(),
        gw_target: sys::gateway_ip(),
        net_target,
        game_exes,
        fps: meter,
        last_hitch: None,
        sample_every: Duration::from_secs(sample_secs.max(1)),
        min_duration_sec,
        last_spike: None,
        game_end: None,
        finished: false,
    }));

    // Mồi mốc CPU: `GetSystemTimes` cần delta, không mồi thì mẫu đầu luôn là 0%.
    live.lock().unwrap().cpu.pct();

    let weak = Arc::downgrade(&live);
    std::thread::spawn(move || ping_loop(weak));
    let weak = Arc::downgrade(&live);
    std::thread::spawn(move || sample_loop(weak));
    live
}

/// Id báo cáo từ giờ địa phương: "2026-10-22 21:30:05" -> "20261022-213005".
fn report_id() -> String {
    let digits: String = sys::local_time_string()
        .chars()
        .filter(|c| c.is_ascii_digit())
        .collect();
    if digits.len() >= 14 {
        format!("{}-{}", &digits[0..8], &digits[8..14])
    } else {
        digits
    }
}

impl Live {
    /// Giây kể từ lúc bắt đầu trận.
    pub fn elapsed(&self) -> f32 {
        self.started.elapsed().as_secs_f32()
    }

    /// Thêm một sự kiện vào báo cáo đang ghi.
    pub fn push_event(&mut self, kind: &str, text: impl Into<String>) {
        let t = self.elapsed();
        self.report.events.push(Event {
            t,
            kind: kind.to_string(),
            text: text.into(),
        });
    }

    /// PID của tiến trình game đầu tiên khớp tên exe (không phân biệt hoa thường).
    /// Dò lại mỗi lần lấy mẫu vì game khởi động lại là PID khác.
    fn game_pids(&self) -> Vec<u32> {
        if self.game_exes.is_empty() {
            return Vec::new();
        }
        sys::list_processes()
            .into_iter()
            .filter(|p| self.game_exes.iter().any(|g| g.eq_ignore_ascii_case(&p.name)))
            .map(|p| p.pid)
            .collect()
    }

    /// Số liệu FPS của khoảng vừa qua từ meter ETW (`None` khi không có meter, chưa dò được
    /// PID, hoặc khoảng đó không có frame nào — game đang ở menu loading / alt-tab).
    fn take_fps_window(&self) -> Option<fps::FpsWindow> {
        let meter = self.fps.as_ref()?;
        meter.set_target_pids(&self.game_pids());
        meter.take_window()
    }

    /// Chốt FPS bằng số chính xác cả trận từ histogram ETW (nếu meter còn sống) rồi DỪNG meter.
    /// Gọi ở mọi đường kết thúc `Live` (finish, Drop) để session ETW `DEFAULT_SESSION`
    /// không treo lại sau khi trận kết thúc.
    fn finish_fps(&mut self) {
        let Some(meter) = self.fps.take() else {
            return;
        };
        if let Some(t) = meter.total() {
            let s = &mut self.report.summary;
            s.fps_avg = Some(t.fps_avg);
            s.fps_low1 = Some(t.fps_low1);
            s.fps_low01 = Some(t.fps_low01);
            s.frametime_p99_ms = Some(t.frametime_p99_ms);
            s.frametime_max_ms = Some(t.frametime_max_ms);
            s.hitches = t.hitches.min(u32::MAX as u64) as u32;
        }
        meter.stop();
    }

    /// Lấy một mẫu: RAM / commit / CPU toàn hệ thống + kết quả ping gom từ lần trước + FPS.
    fn take_sample(&mut self) {
        let mem = sys::memory();
        let lists = sys::memory_lists();
        let (commit_used, commit_limit) = sys::commit();
        let cpu = self.cpu.pct();
        let fps_win = self.take_fps_window();
        let (gw, net) = {
            let mut p = self.ping.lock().unwrap();
            (std::mem::take(&mut p.gw), std::mem::take(&mut p.net))
        };

        let sample = Sample {
            t: self.elapsed(),
            avail_gb: mem.avail as f32 / GIB,
            ram_used_pct: if mem.total > 0 {
                (mem.total - mem.avail) as f32 * 100.0 / mem.total as f32
            } else {
                0.0
            },
            free_gb: lists.map(|l| l.free_bytes as f32 / GIB),
            standby_gb: lists.map(|l| l.standby_bytes as f32 / GIB),
            commit_gb: commit_used as f32 / GIB,
            commit_limit_gb: commit_limit as f32 / GIB,
            commit_pct: if commit_limit > 0 {
                commit_used as f32 * 100.0 / commit_limit as f32
            } else {
                0.0
            },
            cpu_pct: cpu,
            ping_gw_ms: gw.avg(),
            ping_net_ms: net.avg(),
            ping_net_max_ms: net.max(),
            loss_gw_pct: gw.loss_pct(),
            loss_net_pct: net.loss_pct(),
            fps: fps_win.map(|w| w.fps_avg),
            fps_low1: fps_win.map(|w| w.fps_low1),
            frametime_p99_ms: fps_win.map(|w| w.frametime_p99_ms),
            frametime_max_ms: fps_win.map(|w| w.frametime_max_ms),
            hitches: fps_win.map(|w| w.hitches).unwrap_or(0),
        };

        // Ping internet vọt: ghi event, tối thiểu 30 giây một lần.
        if let Some(max) = sample.ping_net_max_ms {
            let room = self
                .last_spike
                .map(|s| s.elapsed() >= SPIKE_MIN_GAP)
                .unwrap_or(true);
            if max > SPIKE_MS && room {
                self.last_spike = Some(Instant::now());
                self.push_event("ping_spike", format!("Ping vọt {:.0} ms", max));
            }
        }

        // Khựng hình: frametime dài nhất trong khoảng vượt 100 ms, tối thiểu 30 giây một event.
        if let Some(max) = sample.frametime_max_ms.filter(|m| *m > HITCH_EVENT_MS) {
            let room = self
                .last_hitch
                .map(|h| h.elapsed() >= HITCH_MIN_GAP)
                .unwrap_or(true);
            if room {
                self.last_hitch = Some(Instant::now());
                self.push_event("hitch", format!("Khựng {:.0} ms", max));
            }
        }

        self.report.samples.push(sample);
    }
}

/// Dừng meter ETW khi `Live` bị bỏ (app thoát giữa trận, watcher bỏ `Arc` mà không `finish`).
impl Drop for Live {
    fn drop(&mut self) {
        self.finish_fps();
    }
}

/// Thêm sự kiện vào báo cáo đang ghi (không làm gì nếu trận đã đóng).
pub fn add_event(live: &Arc<Mutex<Live>>, kind: &str, text: &str) {
    if let Ok(mut l) = live.lock() {
        if !l.stop.load(Ordering::SeqCst) {
            l.push_event(kind, text);
        }
    }
}

/// Watcher vừa thấy game tắt: ghi mốc kết thúc (game mở lại rồi tắt lần nữa thì lấy mốc mới).
pub fn mark_game_end(live: &Arc<Mutex<Live>>) {
    if let Ok(mut l) = live.lock() {
        l.game_end = Some(l.elapsed());
    }
}

/// Dừng ghi, tính `Summary` và lưu file (nếu trận đủ dài).
/// Gọi lần hai trên cùng một trận (watcher và đường thoát app cùng giữ `Arc`) chỉ trả về
/// thông báo "đã lưu", KHÔNG ghi đè báo cáo đã lưu.
pub fn finish(live: Arc<Mutex<Live>>) -> Outcome {
    // Đặt cờ dừng trước rồi mới khoá lại: mẫu đang dở (nếu có) kết thúc xong mới tới lượt ta,
    // nhờ vậy bản tóm tắt không bỏ sót mẫu cuối.
    let stop = live.lock().unwrap().stop.clone();
    stop.store(true, Ordering::SeqCst);

    let (id, samples, saved, path, line) = {
        let mut l = live.lock().unwrap();
        let id = l.report.id.clone();
        let samples = l.report.samples.len();
        if l.finished {
            // Gọi lần hai: vẫn dừng meter (nếu còn) để không treo session ETW.
            l.finish_fps();
            (
                id.clone(),
                samples,
                false,
                None,
                format!("▣ [BÁO CÁO] Trận {} đã được lưu trước đó.", id),
            )
        } else {
            l.finished = true;
            // Độ dài trận tính tới lúc watcher thấy game tắt, không tính 15 giây chờ xác nhận.
            let duration_sec = l.game_end.unwrap_or_else(|| l.elapsed()) as u64;
            l.report.ended_at = sys::local_time_string();
            l.report.summary = summarize(&l.report, duration_sec);
            // Số FPS chính xác cả trận (histogram ETW) đè lên số tính từ mẫu, rồi dừng meter —
            // phải xong TRƯỚC khi ghi file.
            l.finish_fps();

            let long_enough = duration_sec >= l.min_duration_sec;
            let path = if long_enough {
                save_report(&l.report)
            } else {
                None
            };
            let saved = path.is_some();
            let line = if saved {
                summary_line(&l.report)
            } else if long_enough {
                "▣ [BÁO CÁO] Không ghi được file báo cáo (kiểm tra quyền ghi thư mục reports)."
                    .to_string()
            } else {
                format!(
                    "▣ [BÁO CÁO] Trận {} giây — ngắn hơn {} giây nên không lưu.",
                    duration_sec, l.min_duration_sec
                )
            };
            (id, samples, saved, path, line)
        }
    };

    // Trận đã xử lý xong (dù có lưu bản chính hay không): bản lưu tạm không còn cần nữa.
    remove_partial(&reports_dir(), &id);

    if saved {
        prune_reports(KEEP_REPORTS);
    }
    Outcome {
        id,
        saved,
        path,
        samples,
        line,
    }
}

/// Dòng tóm tắt một trận, ví dụ:
/// `▣ [BÁO CÁO] 42 phút · RAM trống thấp nhất 0.6 GB · commit đỉnh 95% · ping TB 38 ms (p95 120 ms, mất 1.2%) · FPS TB 191 (1% low 91)`
pub fn summary_line(report: &GameReport) -> String {
    let s = &report.summary;
    let mut out = format!(
        "▣ [BÁO CÁO] {} · RAM trống thấp nhất {:.1} GB · commit đỉnh {:.0}%",
        duration_text(s.duration_sec),
        s.ram_avail_min_gb,
        s.commit_peak_pct
    );
    match s.ping_net_avg_ms {
        Some(avg) => {
            let p95 = s
                .ping_net_p95_ms
                .map(|p| format!("{:.0} ms", p))
                .unwrap_or_else(|| "—".to_string());
            out.push_str(&format!(
                " · ping TB {:.0} ms (p95 {}, mất {:.1}%)",
                avg, p95, s.loss_net_pct
            ));
        }
        None => out.push_str(" · ping: không đo được"),
    }
    // FPS: chỉ thêm khi thật sự có số (không đo được thì thôi, khỏi rác log).
    if let Some(avg) = s.fps_avg {
        match s.fps_low1 {
            Some(low) => out.push_str(&format!(" · FPS TB {:.0} (1% low {:.0})", avg, low)),
            None => out.push_str(&format!(" · FPS TB {:.0}", avg)),
        }
    }
    out
}

fn duration_text(sec: u64) -> String {
    if sec >= 60 {
        format!("{} phút", sec / 60)
    } else {
        format!("{} giây", sec)
    }
}

/// Tính bản tóm tắt từ mẫu + sự kiện đã ghi.
fn summarize(report: &GameReport, duration_sec: u64) -> Summary {
    let samples = &report.samples;

    let mean = |v: &[f32]| -> Option<f32> {
        if v.is_empty() {
            None
        } else {
            Some(v.iter().sum::<f32>() / v.len() as f32)
        }
    };
    let net: Vec<f32> = samples.iter().filter_map(|s| s.ping_net_ms).collect();
    let gw: Vec<f32> = samples.iter().filter_map(|s| s.ping_gw_ms).collect();

    let ram_min = samples
        .iter()
        .map(|s| s.avail_gb)
        .fold(f32::INFINITY, f32::min);
    let commit_pct = samples.iter().map(|s| s.commit_pct).fold(0.0f32, f32::max);
    let commit_gb = samples.iter().map(|s| s.commit_gb).fold(0.0f32, f32::max);
    let cpu_peak = samples.iter().map(|s| s.cpu_pct).fold(0.0f32, f32::max);

    // FPS: bản lưu tạm và bản khôi phục không có histogram của meter, nên tự tính từ mẫu.
    // `fps_avg` là trung bình CÓ TRỌNG SỐ theo thời gian mỗi mẫu đại diện.
    let fps_low1_samples: Vec<f32> = samples.iter().filter_map(|s| s.fps_low1).collect();
    let frametime_max = samples
        .iter()
        .filter_map(|s| s.frametime_max_ms)
        .reduce(f32::max);

    let mut summary = Summary {
        duration_sec,
        ram_avail_min_gb: if ram_min.is_finite() { ram_min } else { 0.0 },
        ram_avail_avg_gb: mean(&samples.iter().map(|s| s.avail_gb).collect::<Vec<_>>())
            .unwrap_or(0.0),
        commit_peak_pct: commit_pct,
        commit_peak_gb: commit_gb,
        cpu_avg_pct: mean(&samples.iter().map(|s| s.cpu_pct).collect::<Vec<_>>()).unwrap_or(0.0),
        cpu_peak_pct: cpu_peak,
        ping_net_avg_ms: mean(&net),
        ping_net_p95_ms: percentile(&net, 95.0),
        ping_net_max_ms: net.iter().copied().reduce(f32::max),
        jitter_net_ms: jitter(&net),
        loss_net_pct: mean(&samples.iter().map(|s| s.loss_net_pct).collect::<Vec<_>>())
            .unwrap_or(0.0),
        ping_gw_avg_ms: mean(&gw),
        loss_gw_pct: mean(&samples.iter().map(|s| s.loss_gw_pct).collect::<Vec<_>>())
            .unwrap_or(0.0),
        ping_spikes: net.iter().filter(|v| **v > SUMMARY_SPIKE_MS).count() as u32,
        fps_avg: weighted_fps_avg(samples),
        fps_low1: mean(&fps_low1_samples),
        // 0,1% low và p99 chỉ có khi chốt bằng histogram ETW (`Live::finish_fps`).
        fps_low01: None,
        frametime_p99_ms: None,
        frametime_max_ms: frametime_max,
        hitches: samples.iter().map(|s| s.hitches).fold(0, u32::saturating_add),
        commit_warnings: report
            .events
            .iter()
            .filter(|e| e.kind == "commit_warn")
            .count() as u32,
        standby_purges: report
            .events
            .iter()
            .filter(|e| e.kind == "purge")
            .count() as u32,
        verdict: Vec::new(),
    };
    summary.verdict = verdicts(&summary, &report.events);
    summary
}

/// FPS trung bình có trọng số theo thời gian: mỗi mẫu đại diện cho khoảng thời gian của nó
/// (mẫu mỗi `SAMPLE_SEC` giây nên trọng số thường đúng bằng chu kỳ lấy mẫu), và CHỈ tính trên
/// các mẫu thật sự có FPS — mẫu ở menu loading / alt-tab không có frame nào nên bị bỏ.
fn weighted_fps_avg(samples: &[Sample]) -> Option<f32> {
    let mut prev_t = 0.0_f32;
    let mut sum = 0.0_f64;
    let mut weight = 0.0_f64;
    for s in samples {
        let dt = (s.t - prev_t).max(0.0);
        prev_t = s.t;
        let Some(fps) = s.fps else { continue };
        // Mốc thời gian trùng/không hợp lệ: lùi về chu kỳ lấy mẫu để không mất mẫu FPS.
        let w = if dt > 0.0 { dt } else { SAMPLE_SEC as f32 };
        sum += fps as f64 * w as f64;
        weight += w as f64;
    }
    (weight > 0.0).then(|| (sum / weight) as f32)
}

/// Nhận xét tự động (tiếng Việt, ngắn).
fn verdicts(s: &Summary, events: &[Event]) -> Vec<String> {
    let mut out = Vec::new();
    if s.commit_peak_pct >= COMMIT_WARN_PCT {
        out.push(format!(
            "Commit chạm {:.0}% — pagefile/RAM sắp cạn, nguy cơ văng game. Tăng pagefile (16–24 GB) hoặc thêm RAM.",
            s.commit_peak_pct
        ));
    }
    if s.ram_avail_min_gb < 1.0 {
        out.push("RAM trống có lúc dưới 1 GB — nên bỏ 'Giữ lại' bớt app nền.".to_string());
    }
    if s.cpu_avg_pct > 85.0 {
        out.push(
            "CPU gần full suốt trận — giảm thiết lập đồ hoạ nặng CPU hoặc tắt app nền.".to_string(),
        );
    }
    let net_bad = s.ping_net_p95_ms.map(|p| p > 100.0).unwrap_or(false) || s.loss_net_pct > 2.0;
    if net_bad {
        // Gateway "xấu" = RTT cao, hoặc mất gói trong khi VẪN có hồi âm.
        // Gateway không trả lời ICMP suốt trận (ping_gw_avg_ms = None) là không đo được,
        // không phải bằng chứng Wi-Fi nhà kém.
        let gw_bad = s.ping_gw_avg_ms.map(|a| a > 20.0).unwrap_or(false)
            || (s.loss_gw_pct > 1.0 && s.ping_gw_avg_ms.is_some());
        out.push(if gw_bad {
            "Wi-Fi trong nhà chập chờn — nên cắm dây LAN.".to_string()
        } else {
            "Mạng giật phía internet/nhà mạng.".to_string()
        });
    }
    // Khung hình không đều: 1% low tụt dưới nửa FPS trung bình là dấu hiệu khựng rõ.
    if let (Some(avg), Some(low1)) = (s.fps_avg, s.fps_low1) {
        if avg > 0.0 && low1 < avg * 0.5 {
            out.push(format!(
                "Khung hình không đều: 1% low chỉ {low1:.0} FPS so với TB {avg:.0} FPS — có khựng khi tải nặng."
            ));
        }
    }
    if s.hitches > 0 {
        out.push(if hitch_trung_xa_standby(events) {
            format!(
                "Khựng hình {} lần (frame > 100 ms), có lần trùng lúc xả standby — xả standby gây nghẽn I/O ngay trong lúc chơi.",
                s.hitches
            )
        } else {
            format!(
                "Khựng hình {} lần (frame > 100 ms) — giảm thiết lập đồ hoạ hoặc tắt app nền.",
                s.hitches
            )
        });
    }
    // Không có số FPS vì không mở được session ETW (thường là thiếu quyền admin).
    if s.fps_avg.is_none() && events.iter().any(|e| e.kind == "fps_off") {
        out.push("Không đo được FPS — mở GameMode bằng quyền admin để đo FPS.".to_string());
    }
    if out.is_empty() {
        out.push("Trận ổn định, không phát hiện nút thắt.".to_string());
    }
    out
}

/// Có event "hitch" nào cách một event "purge" không quá `HITCH_PURGE_GAP_SEC` giây không?
/// (Xả standby đúng lúc đó là nghi phạm rõ nhất của cú khựng.)
fn hitch_trung_xa_standby(events: &[Event]) -> bool {
    let hitches: Vec<f32> = events
        .iter()
        .filter(|e| e.kind == "hitch")
        .map(|e| e.t)
        .collect();
    if hitches.is_empty() {
        return false;
    }
    events
        .iter()
        .filter(|e| e.kind == "purge")
        .any(|p| hitches.iter().any(|h| (h - p.t).abs() <= HITCH_PURGE_GAP_SEC))
}

/// Phân vị `p` (0..100) của dãy RTT.
fn percentile(v: &[f32], p: f32) -> Option<f32> {
    if v.is_empty() {
        return None;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = (((p / 100.0) * s.len() as f32).ceil() as usize).clamp(1, s.len()) - 1;
    Some(s[idx])
}

/// Jitter = trung bình |RTT[i] - RTT[i-1]| giữa hai mẫu liên tiếp.
fn jitter(v: &[f32]) -> Option<f32> {
    if v.len() < 2 {
        return None;
    }
    let sum: f32 = v.windows(2).map(|w| (w[1] - w[0]).abs()).sum();
    Some(sum / (v.len() - 1) as f32)
}

/// Thread ping: 1 gói/giây cho mỗi đích, gom kết quả vào `PingState` cho thread lấy mẫu.
fn ping_loop(live: Weak<Mutex<Live>>) {
    loop {
        let Some(l) = live.upgrade() else {
            return;
        };
        let (stop, gw, net) = {
            let l = l.lock().unwrap();
            (l.stop.clone(), l.gw_target, l.net_target)
        };
        if stop.load(Ordering::SeqCst) {
            return;
        }
        let gw_rtt = gw.and_then(|a| sys::ping(a, PING_TIMEOUT_MS));
        let net_rtt = net.and_then(|a| sys::ping(a, PING_TIMEOUT_MS));
        {
            let l = l.lock().unwrap();
            let mut p = l.ping.lock().unwrap();
            p.gw.add(gw_rtt);
            p.net.add(net_rtt);
        }
        if stop.load(Ordering::SeqCst) {
            return;
        }
        std::thread::sleep(PING_GAP);
    }
}

/// Thread lấy mẫu: cứ `sample_every` giây một mẫu, dừng khi `stop` bật.
/// Cứ `PARTIAL_GAP` giây, khi trận đã đủ dài, ghi thêm một bản lưu tạm ra đĩa để app/máy tắt
/// giữa trận vẫn còn số liệu (khôi phục lúc mở app: `recover_partials`).
fn sample_loop(live: Weak<Mutex<Live>>) {
    let mut last_partial = Instant::now();
    loop {
        let Some(l) = live.upgrade() else {
            return;
        };
        let (stop, every) = {
            let l = l.lock().unwrap();
            (l.stop.clone(), l.sample_every)
        };
        if stop.load(Ordering::SeqCst) {
            return;
        }
        std::thread::sleep(every);
        let Some(l) = live.upgrade() else {
            return;
        };
        let mut l = l.lock().unwrap();
        if l.stop.load(Ordering::SeqCst) {
            return;
        }
        l.take_sample();

        if last_partial.elapsed() < PARTIAL_GAP || l.elapsed() < l.min_duration_sec as f32 {
            continue;
        }
        last_partial = Instant::now();
        // Nhả khoá `Live` TRƯỚC khi ghi đĩa: chỉ giữ bản sao, để thread ping và `finish`
        // không phải chờ I/O.
        let duration_sec = l.elapsed() as u64;
        let mut snapshot = l.report.clone();
        drop(l);
        snapshot.ended_at = String::new();
        snapshot.summary = summarize(&snapshot, duration_sec);
        save_partial(&snapshot);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Thư mục tạm riêng cho một test (xoá trước khi dùng).
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gamemode-report-test-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn fake_partial(id: &str, samples: usize) -> GameReport {
        let mut report = GameReport {
            id: id.to_string(),
            game: "test.exe".to_string(),
            started_at: "2026-10-22 15:22:30".to_string(),
            ended_at: String::new(),
            ..Default::default()
        };
        for i in 0..samples {
            report.samples.push(Sample {
                t: (i as f32 + 1.0) * 5.0,
                avail_gb: 4.0,
                commit_pct: 70.0,
                ..Default::default()
            });
        }
        report
    }

    #[test]
    fn recover_partial_tao_bao_cao_chinh_va_xoa_ban_tam() {
        let dir = temp_dir("recover");
        let id = "20261022-152230";
        let partial = fake_partial(id, 12); // mẫu cuối t = 60
        save_partial_in(&dir, &partial).unwrap();
        assert!(partial_path_in(&dir, id).is_file());

        let lines = recover_partials_in(&dir);

        // Bản tạm đã bị xoá, bản chính đã có.
        assert!(!partial_path_in(&dir, id).is_file());
        let saved = dir.join(format!("{}.json", id));
        assert!(saved.is_file());
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains(id), "dòng log phải có id: {}", lines[0]);

        let report = read_report(&saved).unwrap();
        assert_eq!(report.summary.duration_sec, 60);
        assert_eq!(report.ended_at, INTERRUPTED_ENDED_AT);
        assert_eq!(report.samples.len(), 12);
        let ev = report
            .events
            .iter()
            .find(|e| e.kind == "interrupted")
            .expect("phải có event interrupted");
        assert!(ev.text.contains("báo cáo khôi phục từ bản lưu tạm"));

        // Bản chính đã có: bản tạm mới chỉ bị xoá, không ghi đè báo cáo cũ.
        save_partial_in(&dir, &fake_partial(id, 2)).unwrap();
        assert!(recover_partials_in(&dir).is_empty());
        assert!(!partial_path_in(&dir, id).is_file());
        assert_eq!(read_report(&saved).unwrap().samples.len(), 12);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ban_lu_tam_khong_phai_bao_cao() {
        let dir = temp_dir("list");
        // Tên file bản tạm -> id chứa '.' nên `valid_id` loại.
        assert!(!valid_id("20261022-152230.partial"));
        assert!(partial_path_in(&dir, "20261022-152230")
            .to_string_lossy()
            .ends_with("20261022-152230.partial.json"));
        assert!(valid_id("20261022-152230"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `summarize` tính `fps_avg` có trọng số theo thời gian mỗi mẫu đại diện, và bỏ qua
    /// mẫu không có FPS (menu / alt-tab).
    #[test]
    fn summarize_tinh_fps_co_trong_so() {
        let report = GameReport {
            id: "20261022-152230".to_string(),
            samples: vec![
                // Khoảng 5 giây, 100 FPS.
                Sample { t: 5.0, fps: Some(100.0), fps_low1: Some(50.0), frametime_max_ms: Some(20.0), ..Default::default() },
                // Khoảng 5 giây, 60 FPS, có 2 frame khựng.
                Sample { t: 10.0, fps: Some(60.0), fps_low1: Some(30.0), frametime_max_ms: Some(150.0), hitches: 2, ..Default::default() },
                // Khoảng 15 giây (mẫu bị trễ), 180 FPS.
                Sample { t: 25.0, fps: Some(180.0), fps_low1: Some(90.0), frametime_max_ms: Some(12.0), ..Default::default() },
                // Menu loading: không có frame nào ⇒ không được tính vào FPS.
                Sample { t: 30.0, ..Default::default() },
            ],
            ..Default::default()
        };

        let s = summarize(&report, 30);

        // (100×5 + 60×5 + 180×15) / 25 = 140 — trung bình thường (không trọng số) là 113,3.
        assert_eq!(s.fps_avg, Some(140.0), "fps_avg = {:?}", s.fps_avg);
        assert_eq!(s.fps_low1, Some((50.0 + 30.0 + 90.0) / 3.0));
        assert_eq!(s.frametime_max_ms, Some(150.0));
        assert_eq!(s.hitches, 2);
        // Không có histogram ETW ⇒ không bịa 0,1% low / p99.
        assert_eq!(s.fps_low01, None);
        assert_eq!(s.frametime_p99_ms, None);
    }

    /// Báo cáo cũ (chưa có field FPS) vẫn parse được nhờ `#[serde(default)]`.
    #[test]
    fn bao_cao_cu_khong_co_fps_van_doc_duoc() {
        let old = r#"{
            "id": "20261022-152230",
            "game": "test.exe",
            "started_at": "2026-10-22 15:22:30",
            "ended_at": "2026-10-22 15:52:30",
            "samples": [
                {"t": 5.0, "avail_gb": 4.0, "ram_used_pct": 70.0, "commit_gb": 10.0,
                 "commit_limit_gb": 20.0, "commit_pct": 50.0, "cpu_pct": 30.0,
                 "loss_gw_pct": 0.0, "loss_net_pct": 0.0}
            ],
            "events": [{"t": 3.0, "kind": "optimize", "text": "Tối ưu"}],
            "summary": {"duration_sec": 1800, "ram_avail_min_gb": 3.5, "commit_peak_pct": 60.0}
        }"#;

        let report: GameReport = serde_json::from_str(old).expect("báo cáo cũ phải đọc được");
        assert_eq!(report.samples.len(), 1);
        assert_eq!(report.samples[0].fps, None);
        assert_eq!(report.samples[0].fps_low1, None);
        assert_eq!(report.samples[0].hitches, 0);
        assert_eq!(report.summary.fps_avg, None);
        assert_eq!(report.summary.fps_low1, None);
        assert_eq!(report.summary.frametime_max_ms, None);
        assert_eq!(report.summary.hitches, 0);
        assert_eq!(report.summary.duration_sec, 1800);

        // Danh sách bên trái cũng đọc được báo cáo cũ (ReportMeta không có field FPS).
        let meta: ReportMeta = serde_json::from_str(
            r#"{"id":"20261022-152230","game":"test.exe","started_at":"2026-10-22 15:22:30",
                "duration_sec":1800,"commit_peak_pct":60.0,"ping_net_avg_ms":38.0}"#,
        )
        .unwrap();
        assert_eq!(meta.fps_avg, None);
        assert_eq!(meta.duration_sec, 1800);
    }

    /// Khựng trùng lúc xả standby: event `hitch` cách event `purge` ≤ 5 giây.
    #[test]
    fn verdict_noi_khung_trung_luc_xa_standby() {
        let s = Summary {
            fps_avg: Some(180.0),
            fps_low1: Some(150.0),
            hitches: 3,
            ..Default::default()
        };

        // Purge ở t = 60, hitch ở t = 63 (cách 3 giây).
        let gan = vec![
            Event { t: 60.0, kind: "purge".to_string(), text: "Xả standby".to_string() },
            Event { t: 63.0, kind: "hitch".to_string(), text: "Khựng 240 ms".to_string() },
        ];
        let out = verdicts(&s, &gan);
        let line = out
            .iter()
            .find(|l| l.contains("trùng lúc xả standby"))
            .unwrap_or_else(|| panic!("phải có dòng khựng trùng xả standby: {out:?}"));
        assert!(line.contains("3 lần"), "phải nêu số lần khựng: {line}");

        // Cách xa (60 vs 90 giây): chỉ là nhận xét khựng chung.
        let xa = vec![
            Event { t: 60.0, kind: "purge".to_string(), text: "Xả standby".to_string() },
            Event { t: 90.0, kind: "hitch".to_string(), text: "Khựng 240 ms".to_string() },
        ];
        let out_xa = verdicts(&s, &xa);
        assert!(!out_xa.iter().any(|l| l.contains("trùng lúc xả standby")), "{out_xa:?}");

        // Không có số FPS vì không mở được session ETW ⇒ gợi ý mở bằng quyền admin.
        let khong_fps = Summary::default();
        let ev = vec![Event {
            t: 0.0,
            kind: "fps_off".to_string(),
            text: "StartTraceW lỗi Win32 5 — cần quyền admin".to_string(),
        }];
        assert!(verdicts(&khong_fps, &ev)
            .iter()
            .any(|l| l.contains("quyền admin")));
    }
}
