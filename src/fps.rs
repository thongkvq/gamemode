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
//!
//! Lọc PID ngay trong callback (không đặt filter phía kernel): lưu lượng DXGI của cả
//! máy chỉ vài nghìn event/giây, rẻ hơn nhiều so với dựng filter phức tạp.
//!
//! Toàn bộ phần tính toán (`FpsWindow` / `FpsTotal`) là hàm thuần trên danh sách
//! frametime và trên histogram, không đụng ETW — nhờ vậy test được trên Linux.
//! Phần gọi API Windows nằm trong `mod win` với `#[cfg(windows)]`.

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};

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
    /// Timestamp QPC của event 178 trước đó, theo từng PID.
    last_ts: HashMap<u32, i64>,
    /// Frametime (ms) gom được kể từ lần `take_window` trước.
    window: VecDeque<f32>,
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
            ..State::default()
        }
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

/// Ghi một frametime (ms) vào trạng thái chung. `None`/giá trị vô lý = bỏ qua.
///
/// Hàm này KHÔNG đụng ETW nên callback chỉ việc gọi nó — và cũng nhờ vậy test được.
fn record_frame_ms(ms: Option<f32>) {
    let Some(ms) = ms else { return };
    // 0/âm/NaN/quá dài đều không phải một frame thật.
    if !(ms.is_finite() && ms > 0.0 && ms <= GAP_MAX_MS) {
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
    st.window.push_back(ms);
}

/// Đổi PID game (watcher dò theo tên exe; game khởi động lại là PID mới).
/// Reset mốc thời gian để không đo khoảng cách xuyên qua hai tiến trình.
fn set_pids(pids: &[u32]) {
    let mut st = lock_state();
    if st.pids != pids {
        st.last_ts.clear();
        st.pids = pids.to_vec();
    }
}

/// Lấy timestamp thô của event 178 trước đó (nếu cùng PID) rồi lưu timestamp mới.
/// Frame đầu tiên sau khi biết/đổi PID chưa có mốc để trừ (trả `None`).
fn take_prev_ts(pid: u32, ts: i64) -> Option<i64> {
    lock_state().last_ts.insert(pid, ts)
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

/// `FpsWindow` từ danh sách frametime (ms). `None` nếu không có frame nào.
fn window_from_frames(frames: &[f32]) -> Option<FpsWindow> {
    if frames.is_empty() {
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
    Some(FpsWindow {
        frames: frames.len() as u32,
        fps_avg: if total > 0.0 {
            frames.len() as f32 * 1000.0 / total
        } else {
            0.0
        },
        fps_low1: 1000.0 / mean(worst1),
        frametime_avg_ms: total / frames.len() as f32,
        frametime_p99_ms: quantile_sorted(&sorted, 0.99),
        frametime_max_ms: *sorted.last().unwrap(),
        hitches: frames.iter().filter(|v| **v > thresh).count() as u32,
    })
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
fn total_from_hist(hist: &[u32], frames: u64, sum_ms: f64, max_ms: f32, hitches: u64) -> FpsTotal {
    if frames == 0 || sum_ms <= 0.0 {
        return FpsTotal::default();
    }
    // Số frame của nhóm "x% tệ nhất", luôn ít nhất 1 frame.
    let group = |frac: f64| (((frames as f64) * frac).ceil() as u64).max(1);
    FpsTotal {
        frames,
        fps_avg: (frames as f64 * 1000.0 / sum_ms) as f32,
        fps_low1: 1000.0 / mean_top_n(hist, group(0.01), max_ms),
        fps_low01: 1000.0 / mean_top_n(hist, group(0.001), max_ms),
        frametime_p99_ms: value_at_rank_desc(hist, group(0.01), max_ms),
        frametime_max_ms: max_ms,
        hitches,
    }
}

/// Một frame bị coi là "giật" khi vượt max(50 ms, 2,5 × trung vị).
fn is_hitch(ms: f32, median_ms: f32) -> bool {
    ms > (HITCH_FACTOR * median_ms).max(HITCH_MS)
}

/// Tên session ETW mặc định khi caller không truyền tên riêng.
pub const DEFAULT_SESSION: &str = "GameModeFps";

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

    /// Lấy và xoá các frametime gom được từ lần gọi trước. `None` nếu chưa có frame.
    pub fn take_window(&self) -> Option<FpsWindow> {
        let frames: Vec<f32> = {
            let mut st = lock_state();
            st.window.drain(..).collect()
        };
        window_from_frames(&frames)
    }

    /// Cộng dồn cả trận (histogram). `None` nếu chưa có frame nào.
    pub fn total(&self) -> Option<FpsTotal> {
        let st = lock_state();
        if st.frames == 0 {
            return None;
        }
        Some(total_from_hist(
            &st.hist, st.frames, st.sum_ms, st.max_ms, st.hitches,
        ))
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

/// Tự kiểm tra: chạy meter `secs` giây cho `pid`, mỗi giây in một dòng kết quả.
/// `--fpstest` trong main.rs sẽ gọi hàm này (chưa nối).
pub fn self_test(pids: &[u32], secs: u64) -> String {
    let mut out = String::new();
    let meter = match FpsMeter::start(DEFAULT_SESSION) {
        Ok(m) => m,
        Err(e) => return format!("Không mở được session ETW: {e}\n"),
    };
    meter.set_target_pids(pids);
    out.push_str(&format!("Đo FPS cho PID {pids:?} trong {secs} giây...\n"));
    for i in 1..=secs {
        std::thread::sleep(std::time::Duration::from_secs(1));
        match meter.take_window() {
            Some(w) => out.push_str(&format!(
                "[{i:>3}s] {:>6.1} FPS | 1% low {:>6.1} | avg {:>6.2} ms | p99 {:>6.1} ms | max {:>6.1} ms | {} frame | giật {}\n",
                w.fps_avg, w.fps_low1, w.frametime_avg_ms, w.frametime_p99_ms, w.frametime_max_ms, w.frames, w.hitches
            )),
            // Không có frame nào: game chưa chạy, đang ở menu, hoặc vừa alt-tab.
            None => out.push_str(&format!("[{i:>3}s] không có frame nào (game chưa chạy?)\n")),
        }
    }
    if let Some(t) = meter.total() {
        out.push_str(&format!(
            "Tổng: {} frame | {:.1} FPS | 1% low {:.1} | 0,1% low {:.1} | p99 {:.1} ms | max {:.1} ms | giật {}\n",
            t.frames, t.fps_avg, t.fps_low1, t.fps_low01, t.frametime_p99_ms, t.frametime_max_ms, t.hitches
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

    use super::{record_frame_ms, take_prev_ts, FpsMeter};

    /// Provider `Microsoft-Windows-DXGI` (event 178 = IDXGISwapChain_Present).
    const DXGI_PROVIDER: GUID = GUID::from_u128(0xCA11C036_0102_4A2D_A6AD_F03CFED5D3C9);
    /// ID của `IDXGISwapChain_Present` (task 80, opcode start).
    const EVT_PRESENT: u16 = 178;
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
                thread: None,
            });
        }
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
        let Some(mut session) = guard.take() else { return };
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
        // Chỉ event 178 (IDXGISwapChain_Present) của provider DXGI.
        if !same_guid(&header.ProviderId, &DXGI_PROVIDER) || header.EventDescriptor.Id != EVT_PRESENT
        {
            return;
        }
        let pid = header.ProcessId;
        if pid == 0 {
            return;
        }
        // Lọc PID ở đây (không filter phía kernel): rỗng = chưa biết game.
        if !super::lock_state().pids.contains(&pid) {
            return;
        }
        let ts = header.TimeStamp;
        // Frame đầu sau khi biết/đổi PID chưa có mốc để trừ.
        let Some(prev) = take_prev_ts(pid, ts) else {
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
        record_frame_ms(Some(dt_ms as f32));
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

    /// 100 frame 10 ms ⇒ 100 FPS, 1% low cũng 100, không giật.
    #[test]
    fn on_dinh_100fps() {
        let w = window_from_frames(&[10.0; 100]).unwrap();
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
        let w = window_from_frames(&v).unwrap();
        assert_eq!(w.frames, 100);
        assert_eq!(w.hitches, 1, "chỉ frame 120 ms mới vượt ngưỡng");
        assert!((w.fps_low1 - 8.333).abs() < 0.05, "low1 = {}", w.fps_low1);
        assert!((w.frametime_max_ms - 120.0).abs() < 0.001);
        // nearest-rank: 99% của 100 frame rơi vào frame thứ 99 ⇒ đúng frame 120 ms.
        assert!((w.frametime_p99_ms - 120.0).abs() < 0.001);
        assert!((w.frametime_avg_ms - 11.1).abs() < 0.05);
    }

    /// Cửa sổ rỗng ⇒ None (không bịa số liệu 0 FPS).
    #[test]
    fn khong_co_frame() {
        assert!(window_from_frames(&[]).is_none());
        assert_eq!(total_from_hist(&vec![0; BINS], 0, 0.0, 0.0, 0).frames, 0);
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
        let t = total_from_hist(&hist, 1000, sum, 50.0, 0);
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
        hist[bin_of(1500.0)] += 1;
        for _ in 0..9 {
            hist[bin_of(16.0)] += 1;
        }
        let t = total_from_hist(&hist, 10, 16.0 * 9.0 + 1500.0, 1500.0, 1);
        assert_eq!(t.frames, 10);
        assert!((t.frametime_max_ms - 1500.0).abs() < 0.001);
        // 10 frame ⇒ nhóm 0,1% tệ nhất vẫn là 1 frame: frame 1500 ms ⇒ 0,667 FPS.
        assert!(
            (t.fps_low01 - 1000.0 / 1500.0).abs() < 0.05,
            "low01 = {}",
            t.fps_low01
        );
        assert_eq!(t.hitches, 1);
    }

    /// Frametime vô lý (0, âm, NaN, quá dài) không được vào số liệu.
    #[test]
    fn bo_qua_gia_tri_vo_ly() {
        {
            let mut s = lock_state();
            *s = State::new();
        }
        record_frame_ms(Some(f32::NAN));
        record_frame_ms(Some(-5.0));
        record_frame_ms(Some(0.0));
        record_frame_ms(Some(GAP_MAX_MS + 1.0));
        record_frame_ms(None);
        assert_eq!(lock_state().frames, 0);
        record_frame_ms(Some(12.5));
        let s = lock_state();
        assert_eq!(s.frames, 1);
        assert_eq!(s.hist[bin_of(12.5)], 1);
    }
}
