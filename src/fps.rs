//! Đo FPS / frametime của game bằng ETW (Event Tracing for Windows).
//!
//! Vì sao không dùng PresentMon: bản PresentMon thử trên máy này không ghi được gì
//! (tuỳ máy/driver). Ta tự mở session ETW real-time rồi đọc thẳng sự kiện của
//! provider **Microsoft-Windows-DXGI**:
//!
//! - Provider GUID `CA11C036-0102-4A2D-A6AD-F03CFED5D3C9`.
//! - Event **178** = `IDXGISwapChain_Present` (task 80, opcode 1 = start) — game gọi
//!   Present một lần mỗi frame, nên khoảng cách giữa hai event 178 liên tiếp của cùng
//!   một PID chính là frametime.
//! - Đã kiểm chứng trên máy thật: Delta Force (DX12) phát 1177 event trong 6,16 s
//!   ⇒ 191 FPS. Phải bật provider bằng level 5 + MatchAnyKeyword `0xFFFF...` mới có 178.
//! - Event **180** = `IDXGISwapChain1_Present1` (task 81): cũng Delta Force nhưng trên laptop
//!   Optimus thì CHỈ phát 180 (~30 FPS ở sảnh, không có 178 nào) — 1.3.3–1.4.1 vì chỉ đếm 178
//!   nên báo cáo không có FPS. Giờ đếm cả hai; PID đã phát 180 thì bỏ 178 để khỏi đếm đôi.
//!
//! Lọc PID ngay trong callback (không đặt filter phía kernel): lưu lượng DXGI của cả
//! máy chỉ vài nghìn event/giây, rẻ hơn nhiều so với dựng filter phức tạp.
//!
//! Toàn bộ phần tính toán (`FpsWindow` / `FpsTotal`) là hàm thuần trên danh sách
//! frametime và trên histogram, không đụng ETW — nhờ vậy test được trên Linux.
//! Phần gọi API Windows nằm trong `mod win` với `#[cfg(windows)]`.
//!
//! # Chống "meter chết im lặng" và số rác (1.5.3)
//!
//! Session ETW có thể chết mà KHÔNG ai biết: một tiến trình khác mở session cùng tên
//! (`start_inner` dừng session trùng tên trước khi mở — xem `win::start_inner`), thread
//! `ProcessTrace` trả về vì lỗi, hoặc danh sách PID không còn khớp game. Trước 1.5.3 hậu quả
//! là cả trận không có frame nào mà báo cáo vẫn "im lặng" (không event, không FPS) — báo cáo
//! `20261023-105444.json` chỉ có 2 frame trong 4 giờ 45 phút. Ba lớp chặn mới:
//!
//! 1. `check_health`: game đang chạy mà im lặng >= `STALE_RESTART_SEC` thì tự mở lại session
//!    (tối đa `MAX_RESTARTS` lần, giãn cách `RESTART_GAP_SEC`) — caller ghi event `fps_restart`;
//!    hết lượt thì trả `FpsHealth::Dead` — caller ghi event `fps_off` kèm lý do.
//! 2. Lọc số rác: bỏ frametime < `MIN_FRAMETIME_MS`, cửa sổ phải đủ `MIN_WINDOW_FRAMES` frame,
//!    cả trận phải đủ `MIN_TOTAL_FRAMES` frame, và tổng kết phải nhất quán
//!    (`fps_low01 <= fps_low1 <= fps_avg`, `frametime_max_ms >= frametime_p99_ms`).
//! 3. Cửa sổ tính trên MỘT tiến trình Present nhiều nhất (`window_from_events`): nhận frame
//!    của mọi PID cùng tên exe, nhưng không gộp hai luồng Present vào cùng một cửa sổ.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

/// Trần frametime (ms): khoảng dài hơn mức này là game bị alt-tab/treo, không phải
/// một frame — ghi vào là hỏng cả fps_avg lẫn p99.
const GAP_MAX_MS: f32 = 2000.0;
/// Bước histogram frametime (ms) — 0,1 ms đủ mịn cho p99/p99.9 mà vẫn nhẹ.
const BIN_MS: f32 = 0.1;
/// Số ô histogram: 0..1000 ms theo bước 0,1 ms.
const BINS: usize = 10_001;
/// Ô cuối (chỉ số `BINS - 1`) gom mọi frametime >= 1000 ms.
const OVERFLOW_BIN: usize = BINS - 1;
/// Ngưỡng "hitch" tuyệt đối (ms); ngưỡng thực tế còn so với 2,5 × trung vị.
const HITCH_MS: f32 = 50.0;
/// Hệ số so với trung vị để coi một frame là hitch (máy FPS thấp vẫn bắt được giật).
const HITCH_FACTOR: f32 = 2.5;
/// Frametime nhỏ hơn mức này (ms) là hai event Present của CÙNG một frame (runtime phát đôi)
/// chứ không phải một frame thật: báo cáo trận 2026-10-23 có frametime 0,0443 ms ⇒ 40404 FPS.
/// (`report.rs` dùng chung hằng này khi lọc lại số của báo cáo cũ.)
pub const MIN_FRAMETIME_MS: f32 = 0.2;
/// Cửa sổ mẫu phải có ít nhất ngần này frame mới ghi FPS. Cửa sổ 1–2 frame (vừa vào game,
/// vừa alt-tab, session vừa chết) mà vẫn chia ra FPS chính là nguồn số rác 40404 FPS.
pub const MIN_WINDOW_FRAMES: u32 = 10;
/// Cả trận phải có ít nhất ngần này frame mới chốt số tổng kết từ histogram.
pub const MIN_TOTAL_FRAMES: u64 = 1000;
/// Trần FPS hợp lý: cao hơn là số rác (bản cũ ghi 40404 FPS) — coi như không đo được.
pub const MAX_PLAUSIBLE_FPS: f32 = 2000.0;
/// Game đang chạy mà meter im lặng quá ngần này giây thì coi như session ETW đã chết.
pub const STALE_RESTART_SEC: f32 = 30.0;
/// Số lần mở lại session tối đa cho một meter (mỗi lần cách nhau `RESTART_GAP_SEC`).
pub const MAX_RESTARTS: u32 = 3;
/// Giãn cách tối thiểu giữa hai lần mở lại session (giây).
pub const RESTART_GAP_SEC: f32 = 30.0;

/// Số liệu FPS của một cửa sổ lấy mẫu (một lần `take_window`).
#[derive(Debug, Clone, Copy, Default)]
pub struct FpsWindow {
    /// Số frame (khoảng frametime) trong cửa sổ.
    pub frames: u32,
    /// FPS trung bình = frames / tổng thời gian.
    pub fps_avg: f32,
    /// 1% low: 1000 / trung bình 1% frametime lớn nhất (ít nhất 1 frame).
    pub fps_low1: f32,
    pub frametime_avg_ms: f32,
    pub frametime_p99_ms: f32,
    pub frametime_max_ms: f32,
    /// Số frame có frametime > max(50 ms, 2,5 × trung vị).
    pub hitches: u32,
}

/// Số liệu FPS cộng dồn cả trận (tính từ histogram, không lưu từng frame).
#[derive(Debug, Clone, Copy, Default)]
pub struct FpsTotal {
    pub frames: u64,
    pub fps_avg: f32,
    /// 1% low.
    pub fps_low1: f32,
    /// 0,1% low.
    pub fps_low01: f32,
    pub frametime_p99_ms: f32,
    pub frametime_max_ms: f32,
    pub hitches: u64,
}

/// Trạng thái dùng chung giữa callback ETW và `FpsMeter`.
///
/// Chỉ tồn tại MỘT `FpsMeter` tại một thời điểm (xem `start`), nên một `static` là đủ
/// và tránh phải truyền con trỏ qua `Context` của `EVENT_TRACE_LOGFILEW` (callback chỉ
/// nhận `*mut EVENT_RECORD`, không có chỗ nào cầm context riêng).
#[derive(Default)]
struct State {
    /// Các PID trùng tên exe của game; rỗng = chưa biết thì bỏ qua mọi event.
    /// Nhiều PID vì có game/app vẽ hình từ tiến trình con (vd tiến trình GPU của Chromium),
    /// chỉ tiến trình thật sự gọi Present mới phát event 178.
    pids: Vec<u32>,
    /// Timestamp QPC của event Present trước đó, theo (PID, ID event).
    last_ts: HashMap<(u32, u16), i64>,
    /// PID đã từng phát event 180 (`Present1`): từ đó chỉ đếm 180, bỏ 178 của PID đó để một
    /// frame không bị đếm hai lần nếu runtime phát cả hai.
    present1: HashSet<u32>,
    /// Frametime (ms) gom được kể từ lần `take_window` trước, kèm PID phát ra nó.
    ///
    /// Giữ PID để cửa sổ chỉ tính trên MỘT tiến trình (xem `window_from_events`): gộp hai
    /// tiến trình cùng Present vào một cửa sổ thì "số frame / độ dài cửa sổ" bị nhân đôi.
    window: VecDeque<(u32, f32)>,
    /// Thời điểm (đồng hồ hệ thống) của frame cuối cùng ghi được — mốc phát hiện meter chết.
    last_frame_at: Option<Instant>,
    /// Thời điểm session hiện tại mở: mốc im lặng khi CHƯA nhận được frame nào.
    session_started_at: Option<Instant>,
    /// Số lần đã mở lại session thành công.
    restarts: u32,
    /// Lần mở lại session gần nhất (để giãn cách).
    last_restart_at: Option<Instant>,
    /// Số lần danh sách PID mục tiêu đổi (chẩn đoán `--fpstest`).
    pid_changes: u32,
    /// Số event 178 / 180 nhận được của PID mục tiêu (chẩn đoán `--fpstest`).
    evt_178: u64,
    evt_180: u64,
    /// Số event Present của MỌI PID trong máy (không chỉ PID mục tiêu).
    ///
    /// Vì sao cần: nếu tiến trình thật sự vẽ khung hình không khớp tên exe game (game có
    /// launcher riêng, exe đổi tên sau khi cập nhật, tiến trình con vẽ hình) thì mọi frame của
    /// nó bị lọc PID bỏ đi trong im lặng — đúng kiểu "cả trận không có FPS" của báo cáo
    /// 2026-10-23. Bảng này cho biết AI đang Present để soi lại danh sách game.
    pid_events: HashMap<u32, u64>,
    /// Số frametime bị bỏ vì < `MIN_FRAMETIME_MS` (Present trùng).
    dup_frames: u64,
    /// Histogram frametime cả trận (bước 0,1 ms).
    hist: Vec<u32>,
    /// Số frame cả trận (mỗi ô histogram chỉ đếm được tối đa `u32::MAX`).
    frames: u64,
    /// Tổng frametime cả trận (ms).
    sum_ms: f64,
    /// Frametime lớn nhất thật sự gặp (ô overflow chỉ biết ">= 1000 ms").
    max_ms: f32,
    /// Số lần giật cả trận.
    hitches: u64,
}

impl State {
    fn new() -> State {
        State {
            hist: vec![0; BINS],
            session_started_at: Some(Instant::now()),
            ..State::default()
        }
    }

    /// Giây kể từ frame cuối (chưa có frame nào thì tính từ lúc mở session).
    fn silent_secs(&self) -> f32 {
        self.last_frame_at
            .or(self.session_started_at)
            .map(|i| i.elapsed().as_secs_f32())
            .unwrap_or(0.0)
    }

    /// Giây kể từ lần mở lại session gần nhất.
    fn since_restart_secs(&self) -> Option<f32> {
        self.last_restart_at.map(|i| i.elapsed().as_secs_f32())
    }
}

/// `Mutex` hỏng (có thread panic khi đang giữ lock) không nên làm sập app: lấy lại giá trị.
fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(State::new()))
}

fn lock_state() -> std::sync::MutexGuard<'static, State> {
    state().lock().unwrap_or_else(|e| e.into_inner())
}

/// Xoá sạch số liệu đã gom. Gọi khi MỞ meter mới (`win::start`): trận mới không được cộng
/// dồn frame của trận trước vào histogram.
fn reset_stats() {
    *lock_state() = State::new();
}

/// Ghi một frametime (ms) của PID `pid` vào trạng thái chung. `None`/giá trị vô lý = bỏ qua.
///
/// Hàm này KHÔNG đụng ETW nên callback chỉ việc gọi nó — và cũng nhờ vậy test được.
fn record_frame_ms(pid: u32, ms: Option<f32>) {
    let Some(ms) = ms else { return };
    // 0/âm/NaN/quá dài đều không phải một frame thật.
    if !(ms.is_finite() && ms > 0.0 && ms <= GAP_MAX_MS) {
        return;
    }
    if ms < MIN_FRAMETIME_MS {
        // Hai Present sát nhau trong cùng một frame: đếm để chẩn đoán, KHÔNG vào thống kê.
        lock_state().dup_frames += 1;
        return;
    }
    let mut st = lock_state();
    let median = median_from_hist(&st.hist, st.frames, st.max_ms);
    let bin = bin_of(ms);
    st.hist[bin] = st.hist[bin].saturating_add(1);
    st.frames += 1;
    st.sum_ms += ms as f64;
    if ms > st.max_ms {
        st.max_ms = ms;
    }
    st.hitches += u64::from(is_hitch(ms, median));
    st.window.push_back((pid, ms));
    st.last_frame_at = Some(Instant::now());
}

/// Đếm một event Present: luôn vào bảng theo PID, và vào bộ đếm 178/180 nếu là PID mục tiêu.
/// Trả `true` nếu PID này đang là mục tiêu đo.
fn note_present(pid: u32, id: u16) -> bool {
    let mut st = lock_state();
    *st.pid_events.entry(pid).or_insert(0) += 1;
    if !st.pids.contains(&pid) {
        return false;
    }
    if id == EVT_PRESENT {
        st.evt_178 += 1;
    } else {
        st.evt_180 += 1;
    }
    true
}

/// Các PID Present nhiều nhất mà KHÔNG nằm trong danh sách mục tiêu (thuần, test được).
fn top_foreign_pids(events: &HashMap<u32, u64>, targets: &[u32], n: usize) -> Vec<(u32, u64)> {
    let mut out: Vec<(u32, u64)> = events
        .iter()
        .filter(|(pid, _)| !targets.contains(pid))
        .map(|(pid, count)| (*pid, *count))
        .collect();
    // Nhiều event trước; hoà thì PID nhỏ trước để kết quả ổn định.
    out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    out.truncate(n);
    out
}

/// Đổi PID game (watcher dò theo tên exe; game khởi động lại là PID mới).
///
/// Chỉ bỏ mốc thời gian của PID KHÔNG còn trong danh sách. Trước 1.5.3 đổi danh sách là xoá
/// sạch `last_ts`, nên mỗi lần danh sách đổi (launcher tắt, tiến trình con sinh ra) lại mất
/// một frame — danh sách đổi liên tục thì gần như không đo được frame nào.
fn set_pids(pids: &[u32]) {
    let mut st = lock_state();
    if st.pids == pids {
        return;
    }
    let keep: HashSet<u32> = pids.iter().copied().collect();
    st.last_ts.retain(|(pid, _), _| keep.contains(pid));
    st.present1.retain(|pid| keep.contains(pid));
    st.pids = pids.to_vec();
    st.pid_changes = st.pid_changes.saturating_add(1);
}

/// `IDXGISwapChain_Present` (task 80, opcode start).
const EVT_PRESENT: u16 = 178;
/// `IDXGISwapChain1_Present1` (task 81, opcode start). Delta Force trên laptop (Optimus,
/// DX12) chỉ phát event này, không có 178 — đo trên máy thật 2026-10-22.
const EVT_PRESENT1: u16 = 180;

/// Event Present có được dùng để đếm frame không.
fn is_present_event(id: u16) -> bool {
    id == EVT_PRESENT || id == EVT_PRESENT1
}

/// Lấy timestamp thô của event Present trước đó (cùng PID, cùng ID event) rồi lưu mốc mới.
/// Frame đầu tiên sau khi biết/đổi PID chưa có mốc để trừ (trả `None`); event 178 của PID
/// đã phát 180 bị bỏ qua (trả `None`).
fn take_prev_ts(pid: u32, id: u16, ts: i64) -> Option<i64> {
    let mut st = lock_state();
    if id == EVT_PRESENT1 {
        st.present1.insert(pid);
    } else if st.present1.contains(&pid) {
        return None;
    }
    st.last_ts.insert((pid, id), ts)
}

// ---------------------------------------------------------------------------
// Tính toán thuần (test được trên mọi HĐH)
// ---------------------------------------------------------------------------

fn mean(v: &[f32]) -> f32 {
    if v.is_empty() {
        0.0
    } else {
        v.iter().sum::<f32>() / v.len() as f32
    }
}

/// Giá trị tại phân vị `frac` (0..1], kiểu "nearest-rank" — cùng công thức với
/// `report::percentile` để hai chỗ báo cáo không lệch nhau.
///
/// Tính chỉ số bằng `f64`: với f32 thì `0,99 × 100 = 99,000001` rồi `.ceil()` thành
/// 100, tức phân vị 99% của 100 frame bị đẩy lên frame dài nhất — sai hẳn.
fn quantile_sorted(sorted: &[f32], frac: f32) -> f32 {
    let idx = (((frac as f64) * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len()) - 1;
    sorted[idx]
}

/// `FpsWindow` từ danh sách frametime (ms) và độ dài THẬT của cửa sổ (giây).
///
/// `None` khi cửa sổ chưa đủ `MIN_WINDOW_FRAMES` frame: 1–2 frame trong 5 giây không phải
/// "FPS" (chia ra 40404 FPS là số rác), và UI đã xử lý `null`.
fn window_from_frames(frames: &[f32], window_secs: f32) -> Option<FpsWindow> {
    if frames.len() < MIN_WINDOW_FRAMES as usize {
        return None;
    }
    let total: f32 = frames.iter().sum();
    let mut sorted = frames.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let med = quantile_sorted(&sorted, 0.5);
    let thresh = (HITCH_FACTOR * med).max(HITCH_MS);
    // 1% low: trung bình nhóm frametime lớn nhất, luôn lấy ít nhất 1 frame — cửa sổ
    // ngắn (vài giây) mà làm tròn xuống 0 frame thì low1 vô nghĩa.
    let n1 = (((frames.len() as f32) * 0.01).ceil() as usize).max(1);
    let worst1 = &sorted[sorted.len() - n1.min(sorted.len())..];
    // FPS = số frame / độ dài cửa sổ thật (không phải 1 / frametime trung bình) để một frame
    // bị bỏ (alt-tab, Present trùng đã lọc) không thổi phồng con số.
    let fps_avg = if window_secs > 0.0 {
        frames.len() as f32 / window_secs
    } else if total > 0.0 {
        frames.len() as f32 * 1000.0 / total
    } else {
        0.0
    };
    Some(FpsWindow {
        frames: frames.len() as u32,
        fps_avg,
        fps_low1: 1000.0 / mean(worst1),
        frametime_avg_ms: total / frames.len() as f32,
        frametime_p99_ms: quantile_sorted(&sorted, 0.99),
        frametime_max_ms: *sorted.last().unwrap(),
        hitches: frames.iter().filter(|v| **v > thresh).count() as u32,
    })
}

/// PID Present nhiều nhất trong cửa sổ (`None` khi cửa sổ rỗng).
fn primary_pid(events: &[(u32, f32)]) -> Option<u32> {
    let mut count: HashMap<u32, usize> = HashMap::new();
    for (pid, _) in events {
        *count.entry(*pid).or_insert(0) += 1;
    }
    // `>` (không phải `>=`) + duyệt theo thứ tự xuất hiện: hoà thì lấy PID xuất hiện trước,
    // kết quả ổn định giữa các lần chạy.
    let mut best: Option<(u32, usize)> = None;
    for (pid, _) in events {
        let n = count[pid];
        if best.map(|(_, bn)| n > bn).unwrap_or(true) {
            best = Some((*pid, n));
        }
    }
    best.map(|(pid, _)| pid)
}

/// `FpsWindow` của cửa sổ vừa qua: chỉ tính trên tiến trình Present NHIỀU NHẤT.
///
/// Nhận frame của mọi PID cùng tên exe game (không bỏ sót frame nào của game), nhưng cửa sổ
/// chỉ lấy một luồng: game có hai tiến trình cùng vẽ hình thì gộp chung sẽ nhân đôi FPS.
fn window_from_events(events: &[(u32, f32)], window_secs: f32) -> Option<FpsWindow> {
    let pid = primary_pid(events)?;
    let frames: Vec<f32> = events
        .iter()
        .filter(|(p, _)| *p == pid)
        .map(|(_, ms)| *ms)
        .collect();
    window_from_frames(&frames, window_secs)
}

/// Chỉ số ô histogram của một frametime (ms). Ô cuối gom mọi giá trị >= 1000 ms.
fn bin_of(ms: f32) -> usize {
    ((ms / BIN_MS) as usize).min(OVERFLOW_BIN)
}

/// Thời lượng đại diện của một ô histogram.
fn bin_ms(bin: usize) -> f32 {
    // Ô cuối là ">= 1000 ms": không có giá trị thật trong ô, phải dùng max đã ghi.
    if bin >= OVERFLOW_BIN {
        f32::NAN
    } else {
        (bin as f32 + 0.5) * BIN_MS
    }
}

/// Giá trị đại diện của ô `bin`, với `max_ms` thay cho ô overflow (>= 1000 ms).
fn representative_ms(bin: usize, max_ms: f32) -> f32 {
    let v = bin_ms(bin);
    if v.is_nan() {
        // Ô overflow chỉ biết "từ 1000 ms trở lên": lấy max thật đã ghi. Đây là chặn
        // trên của frame dài nhất nên FPS suy ra là chặn dưới — sai lệch chỉ ở frame
        // hiếm gặp và không bao giờ làm đẹp số liệu.
        max_ms.max(1000.0)
    } else {
        v
    }
}

/// Trung vị suy từ histogram (đủ chính xác cho ngưỡng hitch: sai số <= 0,1 ms).
fn median_from_hist(hist: &[u32], total: u64, max_ms: f32) -> f32 {
    if total == 0 {
        return 0.0;
    }
    let target = total.div_ceil(2).max(1) as f64;
    let mut acc: f64 = 0.0;
    for (i, c) in hist.iter().enumerate().take(BINS) {
        acc += *c as f64;
        if acc >= target {
            return representative_ms(i, max_ms);
        }
    }
    representative_ms(OVERFLOW_BIN, max_ms)
}

/// Frametime thứ `rank` tính từ lớn nhất (rank 1 = frame dài nhất) suy từ histogram.
fn value_at_rank_desc(hist: &[u32], rank: u64, max_ms: f32) -> f32 {
    let mut acc: u64 = 0;
    for i in (0..BINS.min(hist.len())).rev() {
        acc += hist[i] as u64;
        if acc >= rank {
            return representative_ms(i, max_ms);
        }
    }
    representative_ms(0, max_ms)
}

/// Trung bình `n` frametime lớn nhất suy từ histogram (dùng cho fps_low1 / fps_low01).
fn mean_top_n(hist: &[u32], n: u64, max_ms: f32) -> f32 {
    let n = n.max(1);
    let sum: f32 = (1..=n).map(|r| value_at_rank_desc(hist, r, max_ms)).sum();
    sum / n as f32
}

/// `FpsTotal` từ histogram + tổng thời gian + max thật đã ghi.
///
/// `None` khi chưa đủ frame để chốt (`MIN_TOTAL_FRAMES`) hoặc khi con số suy ra vô lý: thà
/// không có FPS (UI đã xử lý `null`) còn hơn chốt 40404 FPS vào bản tóm tắt.
fn total_from_hist(
    hist: &[u32],
    frames: u64,
    sum_ms: f64,
    max_ms: f32,
    hitches: u64,
) -> Option<FpsTotal> {
    if frames < MIN_TOTAL_FRAMES || sum_ms <= 0.0 || !max_ms.is_finite() || max_ms <= 0.0 {
        return None;
    }
    // Số frame của nhóm "x% tệ nhất", luôn ít nhất 1 frame.
    let group = |frac: f64| (((frames as f64) * frac).ceil() as u64).max(1);
    let mut t = FpsTotal {
        frames,
        fps_avg: (frames as f64 * 1000.0 / sum_ms) as f32,
        fps_low1: 1000.0 / mean_top_n(hist, group(0.01), max_ms),
        fps_low01: 1000.0 / mean_top_n(hist, group(0.001), max_ms),
        // p99 lấy từ tâm ô histogram nên có thể LỚN HƠN max thật (frame 0,21 ms rơi vào ô
        // 0,2–0,3 ms ⇒ p99 = 0,25 > max = 0,21): ép p99 <= max cho bản tóm tắt khỏi mâu thuẫn.
        frametime_p99_ms: value_at_rank_desc(hist, group(0.01), max_ms).min(max_ms),
        frametime_max_ms: max_ms,
        hitches,
    };
    if !(t.fps_avg.is_finite() && t.fps_avg > 0.0 && t.fps_avg <= MAX_PLAUSIBLE_FPS) {
        return None;
    }
    // Thứ tự bắt buộc: fps_low01 <= fps_low1 <= fps_avg — nhóm tệ nhất 0,1% luôn tệ hơn nhóm
    // 1%, và cả hai không thể cao hơn trung bình. Làm tròn histogram có thể phá thứ tự này.
    t.fps_low1 = t.fps_low1.min(t.fps_avg);
    t.fps_low01 = t.fps_low01.min(t.fps_low1);
    Some(t)
}

/// Một frame bị coi là "giật" khi vượt max(50 ms, 2,5 × trung vị).
fn is_hitch(ms: f32, median_ms: f32) -> bool {
    ms > (HITCH_FACTOR * median_ms).max(HITCH_MS)
}

/// Việc cần làm khi meter im lặng (thuần — test được trên Linux, không cần ETW).
#[derive(Debug, Clone, PartialEq, Eq)]
enum StaleAction {
    /// Không sao: game chưa chạy, hoặc chưa im lặng đủ lâu, hoặc đang chờ giãn cách.
    Nothing,
    /// Dừng session hiện tại rồi mở lại.
    Restart,
    /// Hết lượt mở lại: meter chết hẳn, kèm lý do để ghi event `fps_off`.
    GiveUp(String),
}

/// Quyết định khi meter không nhận frame nào trong `silent_sec` giây.
///
/// `game_running` = watcher đang thấy tiến trình game (im lặng lúc game chưa chạy là bình
/// thường, KHÔNG phải lỗi). `since_restart_sec` = giây kể từ lần mở lại gần nhất.
fn stale_action(
    silent_sec: f32,
    game_running: bool,
    restarts: u32,
    since_restart_sec: Option<f32>,
) -> StaleAction {
    if !game_running || !(silent_sec >= STALE_RESTART_SEC) {
        return StaleAction::Nothing;
    }
    if restarts >= MAX_RESTARTS {
        return StaleAction::GiveUp(format!(
            "không nhận được frame nào suốt {silent_sec:.0} giây dù game đang chạy, đã thử mở lại session ETW {restarts} lần"
        ));
    }
    if since_restart_sec.map(|s| s < RESTART_GAP_SEC).unwrap_or(false) {
        return StaleAction::Nothing; // giãn cách: chờ lần mở lại trước đó đủ lâu
    }
    StaleAction::Restart
}

/// Tên session ETW mặc định khi caller không truyền tên riêng.
pub const DEFAULT_SESSION: &str = "GameModeFps";

/// Session riêng cho `--fpstest`: mở session trùng tên sẽ DỪNG session đang chạy, nên dùng chung
/// `DEFAULT_SESSION` thì một lần `--fpstest` giữa trận làm báo cáo của app đang mở mất FPS.
pub const TEST_SESSION: &str = "GameModeFpsTest";

/// PID của MỌI tiến trình trùng tên exe game (không phân biệt hoa thường).
///
/// Đây ĐÚNG đường đi mà `report::Live` dùng mỗi lần lấy mẫu, để `--fpstest` không đo theo một
/// kiểu khác với báo cáo trận (trước 1.5.3 `--fpstest` nhận danh sách PID chụp một lần).
pub fn game_pids(exes: &[String]) -> Vec<u32> {
    if exes.is_empty() {
        return Vec::new();
    }
    crate::sys::list_processes()
        .into_iter()
        .filter(|p| exes.iter().any(|g| g.eq_ignore_ascii_case(&p.name)))
        .map(|p| p.pid)
        .collect()
}

/// Sức khoẻ của meter ETW, caller (`report::Live`) ghi thành event trong báo cáo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FpsHealth {
    /// Bình thường (hoặc chưa im lặng đủ lâu).
    Ok,
    /// Đã tự mở lại session thành công, đây là lần thứ n.
    Restarted(u32),
    /// Không cứu được: ghi event `fps_off` kèm lý do này.
    Dead(String),
}

/// Số liệu chẩn đoán của meter (`--fpstest` in ra để soi trên máy thật).
#[derive(Debug, Clone, Default)]
pub struct MeterStats {
    /// PID đang được đếm.
    pub pids: Vec<u32>,
    /// Số event 178 (`IDXGISwapChain_Present`) nhận được của các PID đó.
    pub evt_178: u64,
    /// Số event 180 (`IDXGISwapChain1_Present1`).
    pub evt_180: u64,
    /// Số frametime bị bỏ vì < `MIN_FRAMETIME_MS` (Present trùng).
    pub dup_frames: u64,
    /// Số frame đã vào thống kê.
    pub frames: u64,
    /// Số lần mở lại session thành công.
    pub restarts: u32,
    /// Số lần danh sách PID mục tiêu đổi.
    pub pid_changes: u32,
    /// Giây kể từ frame cuối (`None`: chưa nhận frame nào).
    pub silent_sec: Option<f32>,
}

// ---------------------------------------------------------------------------
// ETW: mở session real-time, đọc event 178 của Microsoft-Windows-DXGI
// ---------------------------------------------------------------------------

/// Meter đang chạy. `stop`/`Drop` dừng session, đóng trace và join thread.
pub struct FpsMeter {
    /// Còn phải dừng session không (thread đã join rồi thì thôi).
    #[cfg(windows)]
    active: bool,
}

impl FpsMeter {
    /// Mở session ETW real-time tên `session_name` và bật provider DXGI.
    ///
    /// Trả `Err` khi không đủ quyền (ETW session cần admin), khi đã có một `FpsMeter`
    /// đang chạy (chỉ hỗ trợ một meter vì trạng thái là `static`), hoặc khi API lỗi.
    #[cfg(windows)]
    pub fn start(session_name: &str) -> Result<FpsMeter, String> {
        win::start(session_name)
    }

    /// Bản không phải Windows: không có ETW nên báo rõ, không giả lập số liệu.
    #[cfg(not(windows))]
    pub fn start(_session_name: &str) -> Result<FpsMeter, String> {
        Err("ETW chỉ chạy trên Windows".to_string())
    }

    /// Đo mọi tiến trình trùng tên exe của game (danh sách PID dò từ tên).
    pub fn set_target_pids(&self, pids: &[u32]) {
        set_pids(pids);
    }

    /// Lấy và xoá các frametime gom được từ lần gọi trước.
    ///
    /// `window_secs` = độ dài THẬT của cửa sổ vừa qua (caller đo). `None` nếu cửa sổ không
    /// đủ frame (`MIN_WINDOW_FRAMES`) — UI hiển thị "không đo được".
    pub fn take_window(&self, window_secs: f32) -> Option<FpsWindow> {
        self.take_window_diag(window_secs).0
    }

    /// Như `take_window` nhưng trả thêm số frame thô của cửa sổ (chẩn đoán `--fpstest`:
    /// "3 frame < 10" khác hẳn "0 frame" — một bên là meter chết, một bên là game ở menu).
    pub fn take_window_diag(&self, window_secs: f32) -> (Option<FpsWindow>, u32) {
        let events: Vec<(u32, f32)> = {
            let mut st = lock_state();
            st.window.drain(..).collect()
        };
        let raw = events.len() as u32;
        (window_from_events(&events, window_secs), raw)
    }

    /// Cộng dồn cả trận (histogram). `None` nếu chưa đủ frame (`MIN_TOTAL_FRAMES`).
    pub fn total(&self) -> Option<FpsTotal> {
        let st = lock_state();
        total_from_hist(&st.hist, st.frames, st.sum_ms, st.max_ms, st.hitches)
    }

    /// Số liệu chẩn đoán (PID đang đếm, số event 178/180, số lần mở lại session...).
    pub fn stats(&self) -> MeterStats {
        let st = lock_state();
        MeterStats {
            pids: st.pids.clone(),
            evt_178: st.evt_178,
            evt_180: st.evt_180,
            dup_frames: st.dup_frames,
            frames: st.frames,
            restarts: st.restarts,
            pid_changes: st.pid_changes,
            silent_sec: st.last_frame_at.map(|i| i.elapsed().as_secs_f32()),
        }
    }

    /// `n` PID Present nhiều nhất KHÔNG khớp tên exe game, kèm tên exe — dùng để chỉ ra tiến
    /// trình nào thật sự đang vẽ khung hình (game có launcher/exe khác trong danh sách game).
    pub fn foreign_presenters(&self, n: usize) -> Vec<(u32, u64, String)> {
        let (events, targets) = {
            let st = lock_state();
            (st.pid_events.clone(), st.pids.clone())
        };
        let names: HashMap<u32, String> = crate::sys::list_processes()
            .into_iter()
            .map(|p| (p.pid, p.name))
            .collect();
        top_foreign_pids(&events, &targets, n)
            .into_iter()
            .map(|(pid, count)| {
                let name = names.get(&pid).cloned().unwrap_or_else(|| "?".to_string());
                (pid, count, name)
            })
            .collect()
    }

    /// Lý do meter chết + gợi ý ai đang Present mà không khớp tên exe game.
    fn describe_stall(&self, why: &str) -> String {
        let foreign = self.foreign_presenters(3);
        if foreign.is_empty() {
            // Không PID nào Present: session ETW không nhận được event (bị dừng từ ngoài,
            // thiếu quyền, hoặc game đang ở màn hình không vẽ gì).
            return format!("{why}; không nhận được event Present nào của bất kỳ tiến trình nào");
        }
        let list: Vec<String> = foreign
            .iter()
            .map(|(pid, count, name)| format!("{pid} ({name}, {count} event)"))
            .collect();
        format!(
            "{why}; các tiến trình ĐANG Present nhưng không khớp tên exe game: {} — kiểm tra danh sách game trong config",
            list.join("; ")
        )
    }

    /// Meter còn sống không; nếu chết im lặng thì tự mở lại session.
    ///
    /// Gọi mỗi lần lấy mẫu (5 giây) với `game_running` = watcher có thấy tiến trình game.
    /// Session ETW có thể bị tiến trình khác mở trùng tên dừng mất, hoặc thread `ProcessTrace`
    /// trả về vì lỗi — cả hai đều không để lại dấu vết nào trong báo cáo nếu không có hàm này.
    #[cfg(windows)]
    pub fn check_health(&self, game_running: bool) -> FpsHealth {
        // Đọc trạng thái rồi NHẢ khoá trước khi mở lại session: `win::restart` cần khoá SESSION,
        // không được giữ khoá STATE lúc đó (callback ETW cũng đang chờ khoá STATE).
        let (silent_sec, restarts, since_restart) = {
            let st = lock_state();
            (st.silent_secs(), st.restarts, st.since_restart_secs())
        };
        match stale_action(silent_sec, game_running, restarts, since_restart) {
            StaleAction::Nothing => FpsHealth::Ok,
            StaleAction::GiveUp(why) => FpsHealth::Dead(self.describe_stall(&why)),
            StaleAction::Restart => {
                let n = restarts + 1;
                {
                    let mut st = lock_state();
                    st.restarts = n;
                    st.last_restart_at = Some(Instant::now());
                    // Session mới bắt đầu tính lại từ đầu, không thừa hưởng khoảng im lặng cũ
                    // (nếu không sẽ đòi mở lại tiếp ngay lần lấy mẫu sau).
                    st.session_started_at = Some(Instant::now());
                    st.last_frame_at = None;
                }
                match win::restart() {
                    Ok(()) => FpsHealth::Restarted(n),
                    Err(e) => FpsHealth::Dead(format!(
                        "mở lại session ETW thất bại ({e}); đã nhận {} frame trước khi mất",
                        lock_state().frames
                    )),
                }
            }
        }
    }

    /// Bản không phải Windows: không có meter nào tồn tại để mà hỏng.
    #[cfg(not(windows))]
    pub fn check_health(&self, _game_running: bool) -> FpsHealth {
        FpsHealth::Ok
    }

    /// Dừng session ETW, đóng trace, join thread đọc event.
    #[cfg(windows)]
    pub fn stop(mut self) {
        win::stop(&mut self);
    }

    /// Bản không phải Windows: không có gì để dừng.
    #[cfg(not(windows))]
    pub fn stop(self) {}
}

#[cfg(windows)]
impl Drop for FpsMeter {
    fn drop(&mut self) {
        win::stop(self);
    }
}

/// Nguồn danh sách PID cho `--fpstest`.
enum PidSource {
    /// Danh sách cố định truyền vào (chữ ký `self_test` cũ, main.rs 1.5.3 vẫn gọi).
    Fixed(Vec<u32>),
    /// Tên exe game: dò lại PID mỗi giây — giống hệt `report::Live::game_pids`.
    Exes(Vec<String>),
}

impl PidSource {
    fn label(&self) -> &'static str {
        match self {
            PidSource::Fixed(_) => "danh sách PID truyền vào",
            PidSource::Exes(_) => "dò lại theo tên exe mỗi giây (giống báo cáo trận)",
        }
    }

    fn current(&self) -> Vec<u32> {
        match self {
            PidSource::Fixed(v) => v.clone(),
            PidSource::Exes(e) => game_pids(e),
        }
    }
}

/// Tự kiểm tra theo danh sách PID cố định. Giữ chữ ký cũ để `main.rs` hiện tại vẫn biên dịch;
/// `--fpstest` nên gọi `self_test_exes` để dùng đúng đường đi của báo cáo trận.
#[allow(dead_code)] // `--fpstest` dùng `self_test_exes`; giữ cho tiện thử theo PID cố định
pub fn self_test(pids: &[u32], secs: u64) -> String {
    self_test_inner(PidSource::Fixed(pids.to_vec()), secs)
}

/// Tự kiểm tra theo TÊN EXE game — cùng cách chọn PID, cùng bộ lọc như báo cáo trận.
pub fn self_test_exes(exes: &[String], secs: u64) -> String {
    self_test_inner(PidSource::Exes(exes.to_vec()), secs)
}

/// Chạy meter `secs` giây, mỗi giây in một dòng (kèm chẩn đoán khi KHÔNG có frame).
fn self_test_inner(source: PidSource, secs: u64) -> String {
    let mut out = String::new();
    let meter = match FpsMeter::start(TEST_SESSION) {
        Ok(m) => m,
        Err(e) => return format!("Không mở được session ETW: {e}\n"),
    };
    out.push_str(&format!(
        "Đo FPS trong {secs} giây — PID {} ({}); session ETW {:?}.\n",
        format!("{:?}", source.current()),
        source.label(),
        TEST_SESSION
    ));
    let mut last_stats = meter.stats();
    for i in 1..=secs {
        std::thread::sleep(std::time::Duration::from_secs(1));
        let pids = source.current();
        meter.set_target_pids(&pids);
        // Cùng watchdog như báo cáo trận: im lặng 30 giây mà game vẫn chạy thì mở lại session.
        match meter.check_health(!pids.is_empty()) {
            FpsHealth::Ok => {}
            FpsHealth::Restarted(n) => out.push_str(&format!(
                "      ↻ meter im lặng — đã mở lại session ETW (lần {n}/{MAX_RESTARTS})\n"
            )),
            FpsHealth::Dead(why) => out.push_str(&format!("      ✖ meter chết: {why}\n")),
        }
        let (win, raw) = meter.take_window_diag(1.0);
        let now = meter.stats();
        let line = match win {
            Some(w) => format!(
                "{:>6.1} FPS | 1% low {:>6.1} | avg {:>6.2} ms | p99 {:>6.1} ms | max {:>6.1} ms | {} frame | giật {}",
                w.fps_avg, w.fps_low1, w.frametime_avg_ms, w.frametime_p99_ms, w.frametime_max_ms, w.frames, w.hitches
            ),
            // Nói rõ vì sao trống: 0 frame (meter chết / game ở menu) hay chỉ vài frame
            // (dưới MIN_WINDOW_FRAMES nên cố tình không ghi FPS).
            None if raw == 0 => "không có frame nào (game chưa Present?)".to_string(),
            None => format!("chỉ {raw} frame (< {MIN_WINDOW_FRAMES}) — không ghi FPS"),
        };
        out.push_str(&format!(
            "[{i:>3}s] {line} | PID {:?} | 178:{} 180:{} | frame tổng {} | bỏ trùng {}\n",
            now.pids,
            now.evt_178,
            now.evt_180,
            now.frames,
            now.dup_frames
        ));
        last_stats = now;
    }
    match meter.total() {
        Some(t) => out.push_str(&format!(
            "Tổng: {} frame | {:.1} FPS | 1% low {:.1} | 0,1% low {:.1} | p99 {:.1} ms | max {:.1} ms | giật {}\n",
            t.frames, t.fps_avg, t.fps_low1, t.fps_low01, t.frametime_p99_ms, t.frametime_max_ms, t.hitches
        )),
        // Chưa đủ MIN_TOTAL_FRAMES frame: nói rõ chứ không in một con số vô nghĩa.
        None => out.push_str(&format!(
            "Tổng: chỉ {} frame (< {MIN_TOTAL_FRAMES}) — không chốt số tổng kết.\n",
            last_stats.frames
        )),
    }
    out.push_str(&format!(
        "Chẩn đoán: PID đang đếm {:?} | event 178 = {}, 180 = {} | frame vào thống kê {} | bỏ vì Present trùng {} | lần đổi danh sách PID {} | mở lại session {}/{} lần\n",
        last_stats.pids,
        last_stats.evt_178,
        last_stats.evt_180,
        last_stats.frames,
        last_stats.dup_frames,
        last_stats.pid_changes,
        last_stats.restarts,
        MAX_RESTARTS
    ));
    // Ai đang Present mà KHÔNG khớp tên exe game: đây là manh mối để thêm đúng exe vào config.
    let foreign = meter.foreign_presenters(3);
    if !foreign.is_empty() {
        let list: Vec<String> = foreign
            .iter()
            .map(|(pid, count, name)| format!("{pid} ({name}, {count} event)"))
            .collect();
        out.push_str(&format!(
            "PID ĐANG Present nhưng không khớp tên exe game: {} — nếu game của bạn nằm trong danh sách này thì thêm exe đó vào danh sách game.\n",
            list.join("; ")
        ));
    }
    meter.stop();
    out
}

#[cfg(windows)]
mod win {
    //! Phần gọi API Windows. Tách riêng để `src/fps.rs` vẫn biên dịch trên Linux.

    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Mutex, OnceLock};

    use windows_sys::core::GUID;
    use windows_sys::Win32::Foundation::{
        ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_SUCCESS, ERROR_WMI_INSTANCE_NOT_FOUND,
    };
    use windows_sys::Win32::System::Diagnostics::Etw::{
        CloseTrace, ControlTraceW, EnableTraceEx2, OpenTraceW, ProcessTrace, StartTraceW,
        CONTROLTRACE_HANDLE, EVENT_CONTROL_CODE_ENABLE_PROVIDER, EVENT_RECORD,
        EVENT_TRACE_CONTROL_STOP, EVENT_TRACE_LOGFILEW, EVENT_TRACE_PROPERTIES,
        EVENT_TRACE_REAL_TIME_MODE, PROCESSTRACE_HANDLE, PROCESS_TRACE_MODE_EVENT_RECORD,
        PROCESS_TRACE_MODE_RAW_TIMESTAMP, PROCESS_TRACE_MODE_REAL_TIME, WNODE_FLAG_TRACED_GUID,
    };
    use windows_sys::Win32::System::Performance::QueryPerformanceFrequency;

    use super::{is_present_event, record_frame_ms, take_prev_ts, FpsMeter};

    /// Provider `Microsoft-Windows-DXGI` (event 178 = IDXGISwapChain_Present).
    const DXGI_PROVIDER: GUID = GUID::from_u128(0xCA11C036_0102_4A2D_A6AD_F03CFED5D3C9);
    /// Handle rỗng: `ControlTraceW` hiểu handle 0 là "dừng theo tên session".
    const CONTROL_NONE: CONTROLTRACE_HANDLE = CONTROLTRACE_HANDLE { Value: 0 };
    /// Timeout của `EnableTraceEx2` (0 = không chờ thêm).
    const ENABLE_TIMEOUT: u32 = 0;

    /// So GUID theo từng trường: `windows_sys::core::GUID` không có `PartialEq`.
    fn same_guid(a: &GUID, b: &GUID) -> bool {
        a.data1 == b.data1 && a.data2 == b.data2 && a.data3 == b.data3 && a.data4 == b.data4
    }

    /// Session ETW đang mở (chỉ một `FpsMeter` mỗi lúc).
    ///
    /// Chỉ giữ số/địa chỉ thay vì con trỏ: `Session` nhờ vậy là `Send` mà không cần
    /// `unsafe impl` — khối nhớ `props` do ta cấp và chỉ giải phóng SAU khi join thread.
    /// (`windows-sys` 0.59 để handle trong struct `{ Value: u64 }`, nên giữ số cho tiện.)
    struct Session {
        /// `CONTROLTRACE_HANDLE.Value` của session.
        control: u64,
        /// `PROCESSTRACE_HANDLE.Value` do `OpenTraceW` trả về.
        trace: u64,
        /// Địa chỉ khối `EVENT_TRACE_PROPERTIES` (phải sống tới lúc session dừng).
        props: usize,
        /// Layout khối đó (size, align) để giải phóng đúng cách.
        props_layout: (usize, usize),
        /// Tên session — cần lại khi mở lại session sau khi meter chết (`restart`).
        name: String,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    static SESSION: OnceLock<Mutex<Option<Session>>> = OnceLock::new();
    /// Cờ cho callback: session đã dừng thì không ghi số liệu nữa.
    static RUNNING: AtomicBool = AtomicBool::new(false);

    /// `EVENT_TRACE_PROPERTIES` + tên session nằm ngay sau struct (một khối nhớ).
    ///
    /// Vì sao gộp một khối: `StartTraceW`/`ControlTraceW` đọc tên session theo
    /// `LoggerNameOffset` tính từ đầu struct, nên tên phải nằm liền sau nó.
    struct PropsBlock {
        ptr: *mut u8,
        layout: std::alloc::Layout,
    }

    impl PropsBlock {
        fn new(session_name: &str) -> Result<PropsBlock, String> {
            let mut name: Vec<u16> = session_name.encode_utf16().collect();
            name.push(0);
            // Căn 2 byte cho tên (wchar) nằm sau struct.
            let offset = std::mem::size_of::<EVENT_TRACE_PROPERTIES>().div_ceil(2) * 2;
            let size = offset + name.len() * 2;
            let layout = std::alloc::Layout::from_size_align(
                size,
                std::mem::align_of::<EVENT_TRACE_PROPERTIES>(),
            )
            .map_err(|e| format!("không cấp được bộ nhớ cho session ETW: {e}"))?;
            // SAFETY: layout có size > 0 (đã gồm struct + tên).
            let ptr = unsafe { std::alloc::alloc_zeroed(layout) };
            if ptr.is_null() {
                return Err("không cấp được bộ nhớ cho session ETW".to_string());
            }
            // SAFETY: vùng nhớ vừa cấp đủ size/align cho EVENT_TRACE_PROPERTIES.
            unsafe {
                let props = ptr as *mut EVENT_TRACE_PROPERTIES;
                (*props).Wnode.BufferSize = size as u32;
                // ClientContext = 1: timestamp của event là QPC (khớp QueryPerformanceFrequency).
                (*props).Wnode.ClientContext = 1;
                (*props).Wnode.Flags = WNODE_FLAG_TRACED_GUID;
                (*props).LogFileMode = EVENT_TRACE_REAL_TIME_MODE;
                // Có tên session thì `stop` mới dừng được session mồ côi của lần crash trước.
                (*props).LoggerNameOffset = offset as u32;
                std::ptr::copy_nonoverlapping(
                    name.as_ptr() as *const u8,
                    ptr.add(offset),
                    name.len() * 2,
                );
            }
            Ok(PropsBlock { ptr, layout })
        }

        fn as_props(&self) -> *mut EVENT_TRACE_PROPERTIES {
            self.ptr as *mut EVENT_TRACE_PROPERTIES
        }

        /// Chuyển quyền sở hữu khối nhớ sang `Session` (giải phóng trong `stop`).
        fn into_raw(self) -> (usize, (usize, usize)) {
            let raw = (self.ptr as usize, (self.layout.size(), self.layout.align()));
            std::mem::forget(self);
            raw
        }

        /// Trả lại khối nhớ đã gửi cho `Session` (đúng layout lúc cấp).
        fn free_raw(ptr: usize, layout: (usize, usize)) {
            if ptr == 0 {
                return;
            }
            if let Ok(layout) = std::alloc::Layout::from_size_align(layout.0, layout.1) {
                // SAFETY: đúng con trỏ + layout do PropsBlock::new cấp, chỉ gọi một lần.
                unsafe { std::alloc::dealloc(ptr as *mut u8, layout) };
            }
        }
    }

    impl Drop for PropsBlock {
        fn drop(&mut self) {
            // SAFETY: cùng layout đã dùng khi alloc, và chỉ drop một lần.
            unsafe { std::alloc::dealloc(self.ptr, self.layout) };
        }
    }

    /// Mở session ETW. Lỗi trả về là thông điệp tiếng Việt đọc được.
    pub fn start(session_name: &str) -> Result<FpsMeter, String> {
        let mut name: Vec<u16> = session_name.encode_utf16().collect();
        name.push(0);

        // Chỉ một meter: trạng thái số liệu là `static` dùng chung.
        let slot = SESSION.get_or_init(|| Mutex::new(None));
        {
            let mut guard = slot.lock().unwrap_or_else(|e| e.into_inner());
            if guard.is_some() {
                return Err("đã có FpsMeter đang chạy (chỉ hỗ trợ một meter)".to_string());
            }
            // Giữ chỗ trước khi mở session: hai lần `start` song song thì lần sau thấy
            // `Some` và trả Err. Mọi nhánh lỗi bên dưới phải nhả chỗ này.
            *guard = Some(Session {
                control: 0,
                trace: 0,
                props: 0,
                props_layout: (0, 0),
                name: session_name.to_string(),
                thread: None,
            });
        }
        // Meter MỚI: xoá số liệu của trận trước. Trước 1.5.3 histogram là biến toàn cục không
        // bao giờ reset, nên trận sau bị cộng dồn frame của trận trước vào bản tóm tắt.
        super::reset_stats();
        let result = start_inner(session_name, &name, slot);
        if result.is_err() {
            let mut guard = slot.lock().unwrap_or_else(|e| e.into_inner());
            *guard = None;
        }
        result
    }

    fn start_inner(
        session_name: &str,
        name: &[u16],
        slot: &'static Mutex<Option<Session>>,
    ) -> Result<FpsMeter, String> {
        let props = PropsBlock::new(session_name)?;

        // Session sót lại từ lần app crash trước: phải STOP rồi mới StartTraceW được.
        // Chưa từng có session thì Windows trả ERROR_WMI_INSTANCE_NOT_FOUND — vẫn bình thường.
        // SAFETY: handle 0 + tên session hợp lệ; props đúng layout.
        let stale = unsafe {
            ControlTraceW(
                CONTROL_NONE,
                name.as_ptr(),
                props.as_props(),
                EVENT_TRACE_CONTROL_STOP,
            )
        };
        if stale != ERROR_SUCCESS && stale != ERROR_WMI_INSTANCE_NOT_FOUND {
            return Err(describe("ControlTraceW(dừng session cũ)", stale));
        }

        let mut control = CONTROL_NONE;
        // SAFETY: `props` sống tới lúc session dừng (Session giữ khối nhớ).
        let rc = unsafe { StartTraceW(&mut control, name.as_ptr(), props.as_props()) };
        if rc != ERROR_SUCCESS {
            return Err(describe("StartTraceW", rc));
        }

        // Bật provider DXGI ở level verbose + mọi keyword: chỉ vậy mới thấy event 178.
        // `enableparameters = null` = không cần filter/EnableCallback, vì lọc PID ở phía
        // kernel là không cần thiết (lưu lượng DXGI cả máy chỉ vài nghìn event/giây).
        // SAFETY: providerid trỏ tới hằng const; control là handle vừa mở.
        let rc = unsafe {
            EnableTraceEx2(
                control,
                &DXGI_PROVIDER,
                EVENT_CONTROL_CODE_ENABLE_PROVIDER,
                5,
                u64::MAX,
                0,
                ENABLE_TIMEOUT,
                std::ptr::null(),
            )
        };
        if rc != ERROR_SUCCESS {
            // Bật provider hỏng thì dọn session luôn, không để nó mồ côi.
            // SAFETY: control hợp lệ, props đúng layout.
            unsafe {
                ControlTraceW(
                    control,
                    name.as_ptr(),
                    props.as_props(),
                    EVENT_TRACE_CONTROL_STOP,
                );
            }
            return Err(describe("EnableTraceEx2(provider DXGI)", rc));
        }

        // OpenTraceW cần tên session là con trỏ sống trong lúc gọi (chỉ dùng ở đây).
        let mut logfile: EVENT_TRACE_LOGFILEW = unsafe { std::mem::zeroed() };
        logfile.LoggerName = name.as_ptr() as *mut u16;
        logfile.Anonymous1.ProcessTraceMode = PROCESS_TRACE_MODE_REAL_TIME
            | PROCESS_TRACE_MODE_EVENT_RECORD
            | PROCESS_TRACE_MODE_RAW_TIMESTAMP;
        logfile.Anonymous2.EventRecordCallback = Some(on_event);
        // SAFETY: logfile hợp lệ; callback đúng chữ ký PEVENT_RECORD_CALLBACK.
        let trace = unsafe { OpenTraceW(&mut logfile) };
        if trace.Value == 0 {
            // SAFETY: dọn session vừa mở.
            unsafe {
                ControlTraceW(
                    control,
                    name.as_ptr(),
                    props.as_props(),
                    EVENT_TRACE_CONTROL_STOP,
                );
            }
            return Err("OpenTraceW thất bại (không mở được trace real-time)".to_string());
        }

        RUNNING.store(true, Ordering::SeqCst);
        // Session vừa mở: mốc tính "im lặng bao lâu" bắt đầu từ đây (không thừa hưởng mốc của
        // session trước, nếu không `check_health` sẽ đòi mở lại ngay).
        {
            let now = std::time::Instant::now();
            let mut st = super::lock_state();
            st.session_started_at = Some(now);
            st.last_frame_at = None;
        }

        // ProcessTrace chặn tới khi session bị STOP → phải nằm ở thread riêng.
        let (props_raw, props_layout) = props.into_raw();
        let trace_value = trace.Value;
        let thread = std::thread::Builder::new()
            .name("etw-fps".to_string())
            .spawn(move || {
                let handles = [PROCESSTRACE_HANDLE { Value: trace_value }];
                // SAFETY: mảng handle hợp lệ; NULL start/end = đọc tới khi session dừng.
                unsafe { ProcessTrace(handles.as_ptr(), 1, std::ptr::null(), std::ptr::null()) };
                // Session dừng (do `stop`) → callback thôi ghi số liệu.
                RUNNING.store(false, Ordering::SeqCst);
            });
        let thread = match thread {
            Ok(t) => t,
            Err(e) => {
                // Không tạo được thread thì phải tự dọn: trace, session, khối nhớ.
                RUNNING.store(false, Ordering::SeqCst);
                // SAFETY: trace + control + props vừa tạo, chưa đưa cho ai khác.
                unsafe {
                    CloseTrace(trace);
                    ControlTraceW(
                        control,
                        name.as_ptr(),
                        props_raw as *mut EVENT_TRACE_PROPERTIES,
                        EVENT_TRACE_CONTROL_STOP,
                    );
                }
                PropsBlock::free_raw(props_raw, props_layout);
                return Err(format!("không tạo được thread đọc ETW: {e}"));
            }
        };

        let mut guard = slot.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = guard.as_mut() {
            s.control = control.Value;
            s.trace = trace_value;
            s.props = props_raw;
            s.props_layout = props_layout;
            s.thread = Some(thread);
        }
        Ok(FpsMeter { active: true })
    }

    /// Dừng session + đóng trace + join thread. Gọi nhiều lần vô hại.
    pub fn stop(meter: &mut FpsMeter) {
        if !meter.active {
            return; // đã dừng ở lần gọi trước (ví dụ `stop` rồi tới `Drop`)
        }
        meter.active = false;
        let Some(slot) = SESSION.get() else { return };
        let mut guard = slot.lock().unwrap_or_else(|e| e.into_inner());
        let Some(session) = guard.take() else { return };
        stop_session(session);
    }

    /// Dừng session ETW đang mở rồi mở lại NGAY với đúng tên cũ (meter chết im lặng).
    ///
    /// Vì sao cần: session ETW có thể bị dừng từ bên ngoài (một tiến trình khác mở session
    /// cùng tên — `start_inner` dừng session trùng tên trước khi mở) hoặc thread `ProcessTrace`
    /// trả về vì lỗi. Khi đó `RUNNING = false` và callback bỏ qua mọi event: meter im lặng vĩnh
    /// viễn mà báo cáo không có dấu vết nào (báo cáo 2026-10-23: 2 frame rồi hết).
    ///
    /// Số liệu đã gom (histogram) KHÔNG bị xoá — trận vẫn đang diễn ra, chỉ session là mới.
    pub fn restart() -> Result<(), String> {
        let Some(slot) = SESSION.get() else {
            return Err("chưa từng mở session ETW".to_string());
        };
        // Lấy session cũ ra và dọn hẳn TRƯỚC khi mở lại: `StartTraceW` cùng tên sẽ thất bại
        // nếu session cũ còn sống.
        let session_name = {
            let mut guard = slot.lock().unwrap_or_else(|e| e.into_inner());
            let Some(session) = guard.take() else {
                return Err("session ETW đã đóng".to_string());
            };
            let name = session.name.clone();
            stop_session(session);
            name
        };
        let mut wide: Vec<u16> = session_name.encode_utf16().collect();
        wide.push(0);
        match start_inner(&session_name, &wide, slot) {
            // `start_inner` trả handle mới; handle này CHỈ để đóng session lúc kết thúc trận,
            // mà chủ sở hữu thật là `FpsMeter` đang nằm ở `report::Live`. Bỏ handle này đi
            // nhưng ĐỪNG để `Drop` của nó chạy (Drop sẽ dừng luôn session vừa mở).
            Ok(meter) => {
                std::mem::forget(meter);
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// STOP session + đóng trace + join thread đọc event + giải phóng khối props.
    fn stop_session(mut session: Session) {
        RUNNING.store(false, Ordering::SeqCst);
        if session.control != 0 && session.props != 0 {
            // STOP trước: ProcessTrace chỉ trả về khi session đã dừng. Truyền tên = NULL
            // vì đã có handle; khối props phải còn sống (giải phóng sau khi join).
            // SAFETY: control + props còn hợp lệ tới hết hàm này.
            unsafe {
                ControlTraceW(
                    CONTROLTRACE_HANDLE {
                        Value: session.control,
                    },
                    std::ptr::null(),
                    session.props as *mut EVENT_TRACE_PROPERTIES,
                    EVENT_TRACE_CONTROL_STOP,
                );
            }
        }
        if session.trace != 0 {
            // SAFETY: handle do OpenTraceW trả về, đóng đúng một lần.
            unsafe { CloseTrace(PROCESSTRACE_HANDLE { Value: session.trace }) };
        }
        if let Some(t) = session.thread.take() {
            // Thread đã thoát (ProcessTrace đã trả về) thì join xong ngay.
            let _ = t.join();
        }
        PropsBlock::free_raw(session.props, session.props_layout);
    }

    /// Callback ETW: chạy trên thread `ProcessTrace`, phải nhanh và không được panic.
    ///
    /// SAFETY: `record` do ETW cấp và chỉ đọc trong thân hàm này.
    unsafe extern "system" fn on_event(record: *mut EVENT_RECORD) {
        if record.is_null() || !RUNNING.load(Ordering::Relaxed) {
            return;
        }
        let header = &(*record).EventHeader;
        // Chỉ event Present (178) / Present1 (180) của provider DXGI.
        if !same_guid(&header.ProviderId, &DXGI_PROVIDER)
            || !is_present_event(header.EventDescriptor.Id)
        {
            return;
        }
        let pid = header.ProcessId;
        if pid == 0 {
            return;
        }
        // Đếm event thô theo PID rồi mới lọc: 0 event = session chết / sai PID; nhiều event mà
        // 0 frame = sai đơn vị timestamp; event của PID KHÁC = tiến trình vẽ hình không khớp
        // tên exe game (đây là thứ cần biết để soi lại danh sách game).
        if !super::note_present(pid, header.EventDescriptor.Id) {
            return;
        }
        let ts = header.TimeStamp;
        // Frame đầu sau khi biết/đổi PID chưa có mốc để trừ.
        let Some(prev) = take_prev_ts(pid, header.EventDescriptor.Id, ts) else {
            return;
        };
        let freq = qpc_freq();
        if freq <= 0.0 {
            return; // không đọc được tần số QPC thì không quy đổi được ra ms
        }
        // Timestamp thô là QPC tick (Wnode.ClientContext = 1) → đổi sang ms.
        let dt_ms = (ts - prev) as f64 * 1000.0 / freq;
        if dt_ms > super::GAP_MAX_MS as f64 {
            return; // alt-tab / game treo: không phải một frame
        }
        record_frame_ms(pid, Some(dt_ms as f32));
    }

    /// Tần số QPC, đo một lần rồi cache (không đổi trong vòng đời tiến trình).
    fn qpc_freq() -> f64 {
        static FREQ: OnceLock<f64> = OnceLock::new();
        *FREQ.get_or_init(|| {
            let mut f: i64 = 0;
            // SAFETY: con trỏ tới biến cục bộ hợp lệ.
            let ok = unsafe { QueryPerformanceFrequency(&mut f) };
            if ok == 0 || f <= 0 {
                0.0
            } else {
                f as f64
            }
        })
    }

    /// Thông điệp lỗi tiếng Việt kèm mã lỗi Win32.
    fn describe(what: &str, code: u32) -> String {
        let hint = match code {
            ERROR_ACCESS_DENIED => " — cần quyền admin để đo FPS (chạy Paroda GameMode bằng Administrator)",
            ERROR_ALREADY_EXISTS => " — session ETW cùng tên đang tồn tại",
            _ => "",
        };
        format!("{what} lỗi Win32 {code}{hint}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trạng thái số liệu là `static` dùng chung, mà `cargo test` chạy test SONG SONG: test nào
    /// đụng state phải giữ khoá này, không thì test này xoá số liệu của test kia (đã bị flaky).
    fn exclusive() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_stats();
        guard
    }

    #[test]
    fn dem_ca_present_va_present1() {
        assert!(is_present_event(178));
        assert!(is_present_event(180));
        assert!(!is_present_event(196), "196 là event của dwm, không phải frame của game");
    }

    /// 100 frame 10 ms trong cửa sổ 1 giây ⇒ 100 FPS, 1% low cũng 100, không giật.
    #[test]
    fn on_dinh_100fps() {
        let w = window_from_frames(&[10.0; 100], 1.0).unwrap();
        assert_eq!(w.frames, 100);
        assert!((w.fps_avg - 100.0).abs() < 0.01, "fps_avg = {}", w.fps_avg);
        assert!((w.fps_low1 - 100.0).abs() < 0.01, "low1 = {}", w.fps_low1);
        assert_eq!(w.hitches, 0);
        assert!((w.frametime_avg_ms - 10.0).abs() < 0.001);
        assert!((w.frametime_max_ms - 10.0).abs() < 0.001);
    }

    /// 99 frame 10 ms + 1 frame 120 ms ⇒ 1 hitch, 1% low ≈ 8,3 FPS.
    #[test]
    fn mot_frame_giat() {
        let mut v = vec![10.0f32; 99];
        v.push(120.0);
        // Cửa sổ thật dài 1,11 giây (tổng frametime) ⇒ 100 frame / 1,11 s ≈ 90 FPS.
        let w = window_from_frames(&v, 1.11).unwrap();
        assert_eq!(w.frames, 100);
        assert_eq!(w.hitches, 1, "chỉ frame 120 ms mới vượt ngưỡng");
        assert!((w.fps_low1 - 8.333).abs() < 0.05, "low1 = {}", w.fps_low1);
        assert!((w.frametime_max_ms - 120.0).abs() < 0.001);
        // nearest-rank: 99% của 100 frame rơi vào frame thứ 99 ⇒ đúng frame 120 ms.
        assert!((w.frametime_p99_ms - 120.0).abs() < 0.001);
        assert!((w.frametime_avg_ms - 11.1).abs() < 0.05);
        assert!((w.fps_avg - 90.09).abs() < 0.1, "fps_avg = {}", w.fps_avg);
    }

    /// Cửa sổ rỗng ⇒ None (không bịa số liệu 0 FPS).
    #[test]
    fn khong_co_frame() {
        assert!(window_from_frames(&[], 5.0).is_none());
        assert!(total_from_hist(&vec![0; BINS], 0, 0.0, 0.0, 0).is_none());
    }

    /// Cửa sổ ít hơn `MIN_WINDOW_FRAMES` frame ⇒ `None`: 1–2 frame trong 5 giây không phải
    /// "FPS" (đây chính là cách bản 1.5.2 sinh ra 40404 FPS).
    #[test]
    fn cua_so_it_frame_khong_ghi_fps() {
        // Đúng hai frame như báo cáo hỏng: 0,0443 ms và 0,0052 ms.
        let garbage = vec![0.0443_f32, 0.0052];
        assert!(
            window_from_frames(&garbage, 5.0).is_none(),
            "cửa sổ 2 frame không được thành FPS"
        );
        let nine = vec![33.0_f32; (MIN_WINDOW_FRAMES - 1) as usize];
        assert!(window_from_frames(&nine, 5.0).is_none());
        let ten = vec![33.0_f32; MIN_WINDOW_FRAMES as usize];
        let w = window_from_frames(&ten, 5.0).expect("đủ 10 frame thì có FPS");
        assert_eq!(w.frames, MIN_WINDOW_FRAMES);
        assert!((w.fps_avg - 2.0).abs() < 0.01, "10 frame / 5 s = 2 FPS: {}", w.fps_avg);
    }

    /// FPS của cửa sổ = số frame / độ dài cửa sổ THẬT, không phải 1 / frametime trung bình.
    #[test]
    fn fps_theo_do_dai_cua_so_that() {
        // 300 frame × 33 ms = 9,9 giây nhưng cửa sổ thật chỉ 5 giây (frame đến dồn).
        let w = window_from_frames(&vec![33.0_f32; 300], 5.0).unwrap();
        assert!((w.fps_avg - 60.0).abs() < 0.01, "fps_avg = {}", w.fps_avg);
    }

    /// Frametime < 0,2 ms là hai Present của cùng một frame: bỏ khỏi thống kê.
    #[test]
    fn bo_frametime_present_trung() {
        let _guard = exclusive();
        record_frame_ms(7, Some(0.0443));
        record_frame_ms(7, Some(0.0052));
        {
            let st = lock_state();
            assert_eq!(st.frames, 0, "không frame nào được vào histogram");
            assert_eq!(st.dup_frames, 2);
            assert!(st.window.is_empty());
        }
        // 0,2 ms là mốc: 0,19 bỏ, 0,2 nhận.
        record_frame_ms(7, Some(0.19));
        assert_eq!(lock_state().frames, 0);
        record_frame_ms(7, Some(MIN_FRAMETIME_MS));
        assert_eq!(lock_state().frames, 1);
    }

    /// Cửa sổ chỉ tính trên tiến trình Present nhiều nhất (không nhân đôi FPS khi có hai
    /// tiến trình cùng tên exe cùng vẽ hình).
    #[test]
    fn cua_so_chi_tinh_pid_present_nhieu_nhat() {
        // PID 100: 10 frame 33 ms; PID 200: 60 frame 33 ms ⇒ cửa sổ phải là 60 frame.
        let mut events: Vec<(u32, f32)> = vec![(100, 33.0); 10];
        events.extend(vec![(200, 33.0); 60]);
        let w = window_from_events(&events, 2.0).unwrap();
        assert_eq!(w.frames, 60, "chỉ lấy luồng Present nhiều nhất");
        assert!((w.fps_avg - 30.0).abs() < 0.01, "60 frame / 2 s = 30 FPS: {}", w.fps_avg);
        assert_eq!(primary_pid(&events), Some(200));
        assert_eq!(primary_pid(&[]), None);
    }

    /// Histogram: percentile, max thật và các mức low tính đúng.
    #[test]
    fn histogram_phan_vi() {
        let mut hist = vec![0u32; BINS];
        let mut sum = 0.0f64;
        // 990 frame 10 ms + 10 frame 50 ms ⇒ p99 = 50 ms, max = 50 ms.
        for _ in 0..990 {
            hist[bin_of(10.0)] += 1;
            sum += 10.0;
        }
        for _ in 0..10 {
            hist[bin_of(50.0)] += 1;
            sum += 50.0;
        }
        let t = total_from_hist(&hist, 1000, sum, 50.0, 0).expect("đủ 1000 frame");
        assert_eq!(t.frames, 1000);
        assert!(
            (t.frametime_p99_ms - 50.0).abs() < 0.11,
            "p99 = {}",
            t.frametime_p99_ms
        );
        assert!((t.frametime_max_ms - 50.0).abs() < 0.001);
        // 10 frame lớn nhất đều 50 ms ⇒ 1% low = 20 FPS.
        assert!((t.fps_low1 - 20.0).abs() < 0.1, "low1 = {}", t.fps_low1);
        // 0,1% low = 1 frame lớn nhất = 50 ms ⇒ 20 FPS.
        assert!((t.fps_low01 - 20.0).abs() < 0.1, "low01 = {}", t.fps_low01);
        // fps_avg = 1000 frame / (990×10 + 10×50 ms) = 1000/10400 ms ⇒ 96,15 FPS.
        assert!((t.fps_avg - 96.15).abs() < 0.1, "avg = {}", t.fps_avg);
    }

    /// Ô cuối gom mọi frametime >= 1000 ms (không tràn chỉ số).
    #[test]
    fn histogram_o_cuoi() {
        assert_eq!(bin_of(0.0), 0);
        assert_eq!(bin_of(999.95), OVERFLOW_BIN - 1);
        assert_eq!(bin_of(1000.0), OVERFLOW_BIN);
        assert_eq!(bin_of(5000.0), OVERFLOW_BIN);
        let mut hist = vec![0u32; BINS];
        let mut sum = 0.0f64;
        // 1 frame 1500 ms + 999 frame 16 ms (đủ MIN_TOTAL_FRAMES).
        hist[bin_of(1500.0)] += 1;
        sum += 1500.0;
        for _ in 0..999 {
            hist[bin_of(16.0)] += 1;
            sum += 16.0;
        }
        let t = total_from_hist(&hist, 1000, sum, 1500.0, 1).expect("đủ 1000 frame");
        assert_eq!(t.frames, 1000);
        assert!((t.frametime_max_ms - 1500.0).abs() < 0.001);
        // 0,1% low = 1 frame lớn nhất = 1500 ms ⇒ 0,667 FPS.
        assert!(
            (t.fps_low01 - 1000.0 / 1500.0).abs() < 0.05,
            "low01 = {}",
            t.fps_low01
        );
        assert_eq!(t.hitches, 1);
    }

    /// Tổng kết chỉ chốt khi đủ frame; số suy ra phải NHẤT QUÁN.
    #[test]
    fn tong_ket_du_frame_va_nhat_quan() {
        // 999 frame: chưa đủ ⇒ không có tổng kết (thà null còn hơn số rác).
        let mut hist = vec![0u32; BINS];
        hist[bin_of(33.0)] = 999;
        assert!(total_from_hist(&hist, 999, 33.0 * 999.0, 33.0, 0).is_none());

        // 1000 frame 1,01 ms: tâm ô histogram (1,05 ms) LỚN HƠN max thật ⇒ p99 phải bị ép
        // xuống bằng max, không được để p99 > max như báo cáo 2026-10-23 (0,05 > 0,0443).
        let mut hist = vec![0u32; BINS];
        hist[bin_of(1.01)] = 1000;
        let t = total_from_hist(&hist, 1000, 1.01 * 1000.0, 1.01, 0).unwrap();
        assert!(
            t.frametime_p99_ms <= t.frametime_max_ms,
            "p99 {} phải <= max {}",
            t.frametime_p99_ms,
            t.frametime_max_ms
        );

        // Thứ tự low01 <= low1 <= avg luôn đúng, kể cả khi histogram làm tròn.
        let mut hist = vec![0u32; BINS];
        for _ in 0..1000 {
            hist[bin_of(16.6)] += 1;
        }
        hist[bin_of(16.6)] -= 1;
        hist[bin_of(200.0)] += 1;
        let t = total_from_hist(&hist, 1000, 16.6 * 999.0 + 200.0, 200.0, 1).unwrap();
        assert!(t.fps_low01 <= t.fps_low1, "low01 {} <= low1 {}", t.fps_low01, t.fps_low1);
        assert!(t.fps_low1 <= t.fps_avg, "low1 {} <= avg {}", t.fps_low1, t.fps_avg);
        assert!(t.frametime_p99_ms <= t.frametime_max_ms);
    }

    /// FPS suy ra vô lý (> `MAX_PLAUSIBLE_FPS`) thì không chốt tổng kết.
    #[test]
    fn tong_ket_bo_fps_vo_ly() {
        let mut hist = vec![0u32; BINS];
        hist[bin_of(0.2)] = 1000; // 0,2 ms ⇒ 5000 FPS
        assert!(total_from_hist(&hist, 1000, 200.0, 0.2, 0).is_none());
    }

    /// Quyết định mở lại session ETW: chỉ khi game đang chạy VÀ im lặng đủ lâu VÀ còn lượt.
    #[test]
    fn quyet_dinh_mo_lai_session() {
        // Game chưa chạy: im lặng là bình thường.
        assert_eq!(stale_action(120.0, false, 0, None), StaleAction::Nothing);
        // Chưa đủ ngưỡng.
        assert_eq!(stale_action(29.0, true, 0, None), StaleAction::Nothing);
        // Đủ ngưỡng, còn lượt ⇒ mở lại.
        assert_eq!(stale_action(31.0, true, 0, None), StaleAction::Restart);
        // Vừa mở lại 5 giây trước: chờ cho đủ giãn cách.
        assert_eq!(stale_action(31.0, true, 1, Some(5.0)), StaleAction::Nothing);
        assert_eq!(stale_action(31.0, true, 1, Some(31.0)), StaleAction::Restart);
        // Hết lượt ⇒ bỏ cuộc kèm lý do để ghi event `fps_off`.
        match stale_action(301.0, true, MAX_RESTARTS, None) {
            StaleAction::GiveUp(why) => {
                assert!(why.contains("301"), "lý do phải nêu số giây im lặng: {why}");
                assert!(why.contains(&MAX_RESTARTS.to_string()), "và số lần đã thử: {why}");
            }
            other => panic!("phải bỏ cuộc, nhận {other:?}"),
        }
    }

    /// Danh sách PID đổi KHÔNG được xoá mốc timestamp của PID còn lại (mỗi lần đổi danh sách
    /// trước 1.5.3 là mất một frame; đổi liên tục thì không đo được gì).
    #[test]
    fn doi_danh_sach_pid_giu_moc_pid_cu() {
        let _guard = exclusive();
        set_pids(&[10, 20]);
        assert_eq!(take_prev_ts(10, EVT_PRESENT1, 1_000), None);
        assert_eq!(take_prev_ts(20, EVT_PRESENT1, 2_000), None);
        // Launcher (20) tắt: mốc của PID 10 phải còn nguyên.
        set_pids(&[10]);
        assert_eq!(take_prev_ts(10, EVT_PRESENT1, 1_500), Some(1_000));
        // PID mới xuất hiện cũng không xoá mốc của PID đang chạy.
        set_pids(&[10, 30]);
        assert_eq!(take_prev_ts(10, EVT_PRESENT1, 1_600), Some(1_500));
        assert_eq!(lock_state().pid_changes, 3);
    }

    /// PID Present mà KHÔNG khớp tên exe game phải được chỉ ra: báo cáo 2026-10-23 có 2 frame
    /// trong 4 giờ 45 phút — muốn biết ngay là "lọc sai PID" hay "session ETW chết".
    #[test]
    fn chi_ra_pid_present_khong_khop_ten_exe() {
        let _guard = exclusive();
        let mut events: HashMap<u32, u64> = HashMap::new();
        events.insert(11, 3000);
        events.insert(22, 50);
        events.insert(33, 5);
        // 22 là PID mục tiêu nên không nằm trong danh sách "lạ"; còn lại xếp theo số event.
        assert_eq!(top_foreign_pids(&events, &[22], 2), vec![(11, 3000), (33, 5)]);
        assert!(top_foreign_pids(&events, &[11, 22, 33], 3).is_empty());
        assert!(top_foreign_pids(&HashMap::new(), &[], 3).is_empty());

        // `note_present`: PID lạ vẫn được đếm vào bảng theo PID nhưng KHÔNG vào bộ đếm 178/180.
        set_pids(&[22]);
        assert!(note_present(22, EVT_PRESENT1), "PID mục tiêu");
        assert!(!note_present(11, EVT_PRESENT1), "PID lạ");
        let st = lock_state();
        assert_eq!(st.evt_180, 1);
        assert_eq!(st.evt_178, 0);
        assert_eq!(st.pid_events[&11], 1);
        assert_eq!(st.pid_events[&22], 1);
    }

    /// Frametime vô lý (0, âm, NaN, quá dài) không được vào số liệu.
    #[test]
    fn bo_qua_gia_tri_vo_ly() {        let _guard = exclusive();
        record_frame_ms(7, Some(f32::NAN));
        record_frame_ms(7, Some(-5.0));
        record_frame_ms(7, Some(0.0));
        record_frame_ms(7, Some(GAP_MAX_MS + 1.0));
        record_frame_ms(7, None);
        assert_eq!(lock_state().frames, 0);
        record_frame_ms(7, Some(12.5));
        let s = lock_state();
        assert_eq!(s.frames, 1);
        assert_eq!(s.hist[bin_of(12.5)], 1);
        assert!(s.silent_secs() < 1.0, "ghi được frame thì đồng hồ im lặng phải về 0");
    }
}
