//! Báo cáo trận đấu: ghi số liệu RAM / commit / CPU / ping trong lúc chơi,
//! tính nhận xét rồi lưu JSON vào thư mục `reports/` cạnh exe.
//!
//! Mọi số liệu đều là TOÀN HỆ THỐNG (`sys::memory`, `sys::commit`, `GetSystemTimes`,
//! ICMP tới gateway / internet): không mở handle tới process game hay anti-cheat.
//!
//! Vòng đời do watcher ở `main.rs` điều khiển: `start` khi game xuất hiện,
//! `push_event` trong lúc chơi, `finish` khi game tắt.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::sys;

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

/// Danh sách báo cáo trên đĩa, mới nhất trước (id sắp theo thời gian).
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

/// Ghi báo cáo ra `reports/<id>.json` (tạo thư mục nếu chưa có).
fn save_report(report: &GameReport) -> Option<PathBuf> {
    let dir = reports_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return None;
    }
    let path = report_path(&report.id);
    let text = serde_json::to_string_pretty(report).ok()?;
    std::fs::write(&path, text).ok()?;
    Some(path)
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
    sample_every: Duration,
    min_duration_sec: u64,
    /// Lần ghi event ping_spike gần nhất.
    last_spike: Option<Instant>,
    /// Giây kể từ đầu trận lúc watcher thấy game đã tắt (None: chưa tắt).
    /// Watcher chỉ chốt sau 15 giây, nên độ dài trận lấy mốc này cho khỏi cộng thêm 15 giây.
    game_end: Option<f32>,
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
/// `sample_secs` = chu kỳ lấy mẫu; `min_duration_sec` = độ dài tối thiểu để lưu file
/// (0 = luôn lưu, dùng cho `--reporttest`).
pub fn start(
    game: &str,
    ping_host: &str,
    sample_secs: u64,
    min_duration_sec: u64,
) -> Arc<Mutex<Live>> {
    // Đích internet: host trong config, không phân giải được thì lùi về mặc định.
    let net_target = sys::resolve_ipv4(ping_host)
        .or_else(|| sys::resolve_ipv4(PING_HOST_DEFAULT))
        .map(|(addr, _)| addr);

    let live = Arc::new(Mutex::new(Live {
        report: GameReport {
            id: report_id(),
            game: game.to_string(),
            started_at: sys::local_time_string(),
            ended_at: String::new(),
            samples: Vec::new(),
            events: Vec::new(),
            summary: Summary::default(),
        },
        started: Instant::now(),
        stop: Arc::new(AtomicBool::new(false)),
        ping: Arc::new(Mutex::new(PingState::default())),
        cpu: sys::CpuMeter::new(),
        gw_target: sys::gateway_ip(),
        net_target,
        sample_every: Duration::from_secs(sample_secs.max(1)),
        min_duration_sec,
        last_spike: None,
        game_end: None,
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

    /// Lấy một mẫu: RAM / commit / CPU toàn hệ thống + kết quả ping gom từ lần trước.
    fn take_sample(&mut self) {
        let mem = sys::memory();
        let lists = sys::memory_lists();
        let (commit_used, commit_limit) = sys::commit();
        let cpu = self.cpu.pct();
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

        self.report.samples.push(sample);
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
pub fn finish(live: Arc<Mutex<Live>>) -> Outcome {
    // Đặt cờ dừng trước rồi mới khoá lại: mẫu đang dở (nếu có) kết thúc xong mới tới lượt ta,
    // nhờ vậy bản tóm tắt không bỏ sót mẫu cuối.
    let stop = live.lock().unwrap().stop.clone();
    stop.store(true, Ordering::SeqCst);

    let (id, samples, saved, path, line) = {
        let mut l = live.lock().unwrap();
        // Độ dài trận tính tới lúc watcher thấy game tắt, không tính 15 giây chờ xác nhận.
        let duration_sec = l.game_end.unwrap_or_else(|| l.elapsed()) as u64;
        l.report.ended_at = sys::local_time_string();
        l.report.summary = summarize(&l.report, duration_sec);

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
            "▣ [BÁO CÁO] Không ghi được file báo cáo (kiểm tra quyền ghi thư mục reports).".to_string()
        } else {
            format!(
                "▣ [BÁO CÁO] Trận {} giây — ngắn hơn {} giây nên không lưu.",
                duration_sec, l.min_duration_sec
            )
        };
        (
            l.report.id.clone(),
            l.report.samples.len(),
            saved,
            path,
            line,
        )
    };

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
/// `▣ [BÁO CÁO] 42 phút · RAM trống thấp nhất 0.6 GB · commit đỉnh 95% · ping TB 38 ms (p95 120 ms, mất 1.2%)`
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
    summary.verdict = verdicts(&summary);
    summary
}

/// Nhận xét tự động (tiếng Việt, ngắn).
fn verdicts(s: &Summary) -> Vec<String> {
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
    if out.is_empty() {
        out.push("Trận ổn định, không phát hiện nút thắt.".to_string());
    }
    out
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
fn sample_loop(live: Weak<Mutex<Live>>) {
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
    }
}
