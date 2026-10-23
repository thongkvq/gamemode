//! Phím tắt toàn cục (global hotkey) cho GameMode.
//!
//! `RegisterHotKey` với `hwnd = NULL` gắn tổ hợp vào **thread gọi nó**, và `WM_HOTKEY` được
//! gửi vào message queue của chính thread đó. Vì vậy đăng ký / huỷ / vòng lặp `GetMessageW`
//! phải nằm trên một thread riêng — cùng lý do như `tray.rs`: cửa sổ eframe ẩn xuống khay thì
//! `update()` không chạy nên main thread không nhận được phím tắt.
//!
//! Module tự đứng được (không `use crate::...`). Phần WinAPI nằm sau `#[cfg(windows)]`:
//! `HotkeyAction` / `Hotkey::parse` / `Hotkey::display` là code thuần (test được trên Linux),
//! còn `HotkeyService` có bản giả trên nền tảng khác để crate test vẫn biên dịch và chạy.

use std::sync::mpsc::{channel, sync_channel, Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

// ── Hằng số dùng chung ────────────────────────────────────────────────────────────────────
// Định nghĩa tại đây (không mượn windows-sys) để `parse`/`display` là code thuần, chạy được
// cả khi không build cho Windows. Giá trị đúng theo Win32.
/// `MOD_ALT`
pub const MOD_ALT: u32 = 0x0001;
/// `MOD_CONTROL`
pub const MOD_CONTROL: u32 = 0x0002;
/// `MOD_SHIFT`
pub const MOD_SHIFT: u32 = 0x0004;
/// `MOD_WIN`
pub const MOD_WIN: u32 = 0x0008;
/// `MOD_NOREPEAT`: giữ phím không sinh thêm `WM_HOTKEY` (luôn được bật khi đăng ký).
#[cfg_attr(not(windows), allow(dead_code))]
const MOD_NOREPEAT: u32 = 0x4000;

/// `WM_QUIT` — gửi vào thread phím tắt để `GetMessageW` trả 0 và thread thoát.
const WM_QUIT_MSG: u32 = 0x0012;
/// `WM_APP + 1` — đánh thức thread phím tắt khi cần đăng ký lại.
const WM_APP_REBIND: u32 = 0x8000 + 1;
/// Id phím tắt đầu tiên (mỗi binding một id tăng dần, chỉ cần khác nhau trong cùng thread).
#[cfg_attr(not(windows), allow(dead_code))]
const HOTKEY_ID_BASE: i32 = 0x4700;

// ── Hành động ─────────────────────────────────────────────────────────────────────────────

/// Việc GameMode làm khi người dùng bấm phím tắt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HotkeyAction {
    Boost,
    Restore,
    ToggleWindow,
}

impl HotkeyAction {
    /// Khoá trong `gamemode.cfg` (dùng cho phần ghép config sau này).
    pub fn config_key(self) -> &'static str {
        match self {
            HotkeyAction::Boost => "boost",
            HotkeyAction::Restore => "restore",
            HotkeyAction::ToggleWindow => "window",
        }
    }

    /// Nhãn tiếng Việt hiện trên UI.
    pub fn label(self) -> &'static str {
        match self {
            HotkeyAction::Boost => "Bật Game Boost",
            HotkeyAction::Restore => "Khôi phục",
            HotkeyAction::ToggleWindow => "Hiện/ẩn cửa sổ",
        }
    }
}

/// Phím tắt mặc định (chuỗi để phần config tự `Hotkey::parse`).
pub fn default_bindings() -> Vec<(HotkeyAction, &'static str)> {
    vec![
        (HotkeyAction::Boost, "Ctrl+Alt+B"),
        (HotkeyAction::Restore, "Ctrl+Alt+R"),
        (HotkeyAction::ToggleWindow, "Ctrl+Alt+G"),
    ]
}

// ── Tổ hợp phím ───────────────────────────────────────────────────────────────────────────

/// Một tổ hợp phím: `mods` là tổ hợp bit `MOD_*`, `vk` là mã phím ảo Win32.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Hotkey {
    pub mods: u32,
    pub vk: u32,
}

impl Hotkey {
    /// Đọc tổ hợp từ chuỗi người dùng, ví dụ `"Ctrl+Alt+B"`.
    ///
    /// Không phân biệt hoa thường, cho phép dấu cách quanh `+` (`"ctrl + alt + b"`).
    /// Modifier: `Ctrl|Control|Alt|Shift|Win`. Phím: `A–Z`, `0–9`, `F1–F24`,
    /// `Home End Insert Delete PageUp PageDown Pause ScrollLock Space`, `Num0–Num9`.
    /// Bắt buộc có ít nhất một modifier, trừ `F13–F24`, `Pause`, `ScrollLock`.
    /// Lỗi trả về câu tiếng Việt (không panic).
    pub fn parse(s: &str) -> Result<Hotkey, String> {
        let raw = s.trim();
        if raw.is_empty() {
            return Err("Phím tắt để trống — ví dụ hợp lệ: \"Ctrl+Alt+B\".".to_string());
        }

        let mut mods = 0u32;
        let mut key: Option<u32> = None;
        for part in raw.split('+') {
            let token = part.trim();
            if token.is_empty() {
                return Err(format!(
                    "Phím tắt \"{s}\" sai định dạng (thừa/thiếu dấu +). Ví dụ: \"Ctrl+Alt+B\"."
                ));
            }
            if let Some(m) = parse_modifier(token) {
                if key.is_some() {
                    return Err(format!(
                        "Phím tắt \"{s}\": modifier \"{token}\" phải đứng trước phím."
                    ));
                }
                mods |= m;
            } else if let Some(vk) = parse_key(token) {
                if key.is_some() {
                    return Err(format!("Phím tắt \"{s}\" có nhiều hơn một phím."));
                }
                key = Some(vk);
            } else {
                return Err(format!(
                    "Phím tắt \"{s}\": không hiểu \"{token}\". Modifier nhận Ctrl/Alt/Shift/Win; \
                     phím nhận A–Z, 0–9, F1–F24, Home/End/Insert/Delete/PageUp/PageDown/Pause/\
                     ScrollLock/Space, Num0–Num9."
                ));
            }
        }

        let vk = match key {
            Some(vk) => vk,
            None => return Err(format!("Phím tắt \"{s}\" thiếu phím (chỉ có modifier).")),
        };
        if mods == 0 && !standalone_ok(vk) {
            return Err(format!(
                "Phím tắt \"{s}\" cần ít nhất một modifier (Ctrl/Alt/Shift/Win); chỉ F13–F24, \
                 Pause và ScrollLock được dùng một mình."
            ));
        }
        Ok(Hotkey { mods, vk })
    }

    /// Chuỗi chuẩn hoá: thứ tự modifier cố định `Ctrl+Alt+Shift+Win` rồi tới tên phím.
    /// `Hotkey::parse(&hk.display())` luôn trả lại đúng `hk` (với tổ hợp đọc từ `parse`).
    pub fn display(&self) -> String {
        let mut out = String::new();
        for (flag, name) in [
            (MOD_CONTROL, "Ctrl"),
            (MOD_ALT, "Alt"),
            (MOD_SHIFT, "Shift"),
            (MOD_WIN, "Win"),
        ] {
            if self.mods & flag != 0 {
                out.push_str(name);
                out.push('+');
            }
        }
        out.push_str(&key_name(self.vk));
        out
    }
}

/// Modifier -> bit `MOD_*` (không phân biệt hoa thường).
fn parse_modifier(token: &str) -> Option<u32> {
    match token.to_ascii_lowercase().as_str() {
        "ctrl" | "control" => Some(MOD_CONTROL),
        "alt" => Some(MOD_ALT),
        "shift" => Some(MOD_SHIFT),
        "win" => Some(MOD_WIN),
        _ => None,
    }
}

/// Tên phím -> mã phím ảo Win32.
fn parse_key(token: &str) -> Option<u32> {
    let up = token.to_ascii_uppercase();

    // Một ký tự: A–Z hoặc 0–9 (mã VK trùng mã ASCII hoa / chữ số).
    let bytes = up.as_bytes();
    if bytes.len() == 1 {
        let c = bytes[0];
        if c.is_ascii_uppercase() || c.is_ascii_digit() {
            return Some(c as u32);
        }
    }

    // F1–F24 (VK_F1 = 0x70).
    if let Some(num) = up.strip_prefix('F').and_then(|n| n.parse::<u32>().ok()) {
        if (1..=24).contains(&num) {
            return Some(0x70 + num - 1);
        }
    }

    // Num0–Num9 (VK_NUMPAD0 = 0x60).
    if let Some(rest) = up.strip_prefix("NUM") {
        if rest.len() == 1 {
            let c = rest.as_bytes()[0];
            if c.is_ascii_digit() {
                return Some(0x60 + (c - b'0') as u32);
            }
        }
    }

    match up.as_str() {
        "HOME" => Some(0x24),
        "END" => Some(0x23),
        "INSERT" => Some(0x2D),
        "DELETE" => Some(0x2E),
        "PAGEUP" => Some(0x21),
        "PAGEDOWN" => Some(0x22),
        "PAUSE" => Some(0x13),
        "SCROLLLOCK" => Some(0x91),
        "SPACE" => Some(0x20),
        _ => None,
    }
}

/// Mã phím -> tên chuẩn (ngược của `parse_key`).
fn key_name(vk: u32) -> String {
    match vk {
        0x41..=0x5A => return ((vk as u8) as char).to_string(),
        0x30..=0x39 => return ((vk as u8) as char).to_string(),
        0x70..=0x87 => return format!("F{}", vk - 0x70 + 1),
        0x60..=0x69 => return format!("Num{}", vk - 0x60),
        0x24 => return "Home".to_string(),
        0x23 => return "End".to_string(),
        0x2D => return "Insert".to_string(),
        0x2E => return "Delete".to_string(),
        0x21 => return "PageUp".to_string(),
        0x22 => return "PageDown".to_string(),
        0x13 => return "Pause".to_string(),
        0x91 => return "ScrollLock".to_string(),
        0x20 => return "Space".to_string(),
        _ => {}
    }
    // Mã lạ (do người gọi tự dựng `Hotkey`): vẫn in ra được, nhưng `parse` sẽ không nhận.
    format!("VK{:02X}", vk)
}

/// Phím dùng một mình (không cần modifier) có hợp lệ không.
fn standalone_ok(vk: u32) -> bool {
    (0x7C..=0x87).contains(&vk) // F13–F24
        || vk == 0x13 // Pause
        || vk == 0x91 // ScrollLock
}

// ── Dịch vụ phím tắt (thread message loop) ────────────────────────────────────────────────

/// Dịch vụ phím tắt toàn cục: giữ thread message loop đã đăng ký các tổ hợp.
///
/// Bỏ struct này (drop) là thread nhận `WM_QUIT` rồi tự huỷ hết đăng ký và thoát.
pub struct HotkeyService {
    /// Id thread message loop (0 = thread không chạy, ví dụ trên nền tảng không phải Windows).
    thread_id: u32,
    /// Đăng ký mới nhất chờ thread xử lý (thread lấy ra khi nhận `WM_APP_REBIND`).
    pending: Arc<Mutex<Option<Vec<(HotkeyAction, Hotkey)>>>>,
    /// Lỗi đăng ký của lần gần nhất: thread ghi, UI đọc.
    errors: Arc<Mutex<Vec<(HotkeyAction, String)>>>,
    /// Thread báo đã thoát — để `drop` không phải join mù.
    done: Receiver<()>,
    join: Option<JoinHandle<()>>,
}

impl HotkeyService {
    /// Đăng ký `bindings` và chạy thread message loop; `on_fire` được gọi từ thread đó
    /// (nên phải `Send`, và tự lo việc chuyển tiếp sang UI).
    ///
    /// Không trả `Result`: lỗi (thread không tạo được, tổ hợp bị app khác chiếm) nằm trong
    /// [`HotkeyService::errors`].
    pub fn start(
        bindings: Vec<(HotkeyAction, Hotkey)>,
        on_fire: impl Fn(HotkeyAction) + Send + 'static,
    ) -> HotkeyService {
        let errors: Arc<Mutex<Vec<(HotkeyAction, String)>>> = Arc::new(Mutex::new(Vec::new()));
        let pending: Arc<Mutex<Option<Vec<(HotkeyAction, Hotkey)>>>> = Arc::new(Mutex::new(None));
        // Thread báo id sau khi đã tạo message queue -> `start` mới dám PostThreadMessageW.
        let (ready_tx, ready_rx) = sync_channel::<u32>(1);
        let (done_tx, done_rx) = channel::<()>();

        let worker_bindings = bindings.clone();
        let worker_pending = pending.clone();
        let worker_errors = errors.clone();
        let spawned = std::thread::Builder::new()
            .name("hotkey".to_string())
            .spawn(move || {
                worker(
                    worker_bindings,
                    Box::new(on_fire),
                    worker_pending,
                    worker_errors,
                    ready_tx,
                    done_tx,
                )
            });

        let (join, thread_id) = match spawned {
            Ok(join) => {
                // Chờ thread tạo xong queue + đăng ký xong; quá 3 giây thì coi như không có id.
                let tid = ready_rx.recv_timeout(Duration::from_secs(3)).unwrap_or(0);
                (Some(join), tid)
            }
            Err(e) => {
                if let Ok(mut slot) = errors.lock() {
                    *slot = bindings
                        .iter()
                        .map(|(action, hk)| {
                            (
                                *action,
                                format!("không tạo được thread phím tắt: {e} (\"{}\" chưa có tác dụng).", hk.display()),
                            )
                        })
                        .collect();
                }
                (None, 0)
            }
        };
        HotkeyService {
            thread_id,
            pending,
            errors,
            done: done_rx,
            join,
        }
    }

    /// Đổi danh sách phím tắt: nhờ thread message loop huỷ hết đăng ký cũ rồi đăng ký lại.
    /// Bất đồng bộ — kết quả (thành công/thất bại) đọc qua [`HotkeyService::errors`] sau đó.
    pub fn rebind(&self, bindings: Vec<(HotkeyAction, Hotkey)>) {
        let actions: Vec<HotkeyAction> = bindings.iter().map(|(a, _)| *a).collect();
        // Dữ liệu đi qua `Mutex`, message chỉ để đánh thức thread (queue của thread khác
        // không nhận được con trỏ/closure).
        match self.pending.lock() {
            Ok(mut slot) => *slot = Some(bindings),
            Err(_) => return,
        }
        if !post_thread(self.thread_id, WM_APP_REBIND) {
            self.fail(actions, "không gửi được yêu cầu đổi phím tắt (thread phím tắt đã dừng?).");
        }
    }

    /// Các binding đăng ký thất bại ở lần gần nhất, kèm lý do tiếng Việt.
    /// Rỗng nghĩa là mọi phím tắt đang hoạt động.
    pub fn errors(&self) -> Vec<(HotkeyAction, String)> {
        self.errors
            .lock()
            .map(|slot| slot.clone())
            .unwrap_or_default()
    }

    /// Ghi lỗi chung cho một loạt hành động (khi không đánh thức được thread).
    fn fail(&self, actions: Vec<HotkeyAction>, msg: &str) {
        if let Ok(mut slot) = self.errors.lock() {
            *slot = actions
                .into_iter()
                .map(|action| (action, msg.to_string()))
                .collect();
        }
    }
}

impl Drop for HotkeyService {
    fn drop(&mut self) {
        let join = match self.join.take() {
            Some(join) => join,
            None => return,
        };
        // WM_QUIT làm GetMessageW trả 0 -> thread ra khỏi vòng lặp, huỷ đăng ký rồi thoát.
        if !join.is_finished() {
            post_thread(self.thread_id, WM_QUIT_MSG);
        }
        // Chờ tối đa 1,5 giây; quá hạn thì bỏ mặc thread (detach) để không treo lúc thoát app.
        if self.done.recv_timeout(Duration::from_millis(1500)).is_ok() {
            let _ = join.join();
        }
    }
}

/// Gửi message vào queue của thread phím tắt. `false` = không gửi được.
#[cfg(windows)]
fn post_thread(thread_id: u32, msg: u32) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::PostThreadMessageW;
    if thread_id == 0 {
        return false;
    }
    unsafe { PostThreadMessageW(thread_id, msg, 0, 0) != 0 }
}

/// Nền tảng không phải Windows: không có Win32 để gửi message.
#[cfg(not(windows))]
fn post_thread(_thread_id: u32, _msg: u32) -> bool {
    false
}

/// Thân thread message loop (Windows).
#[cfg(windows)]
fn worker(
    bindings: Vec<(HotkeyAction, Hotkey)>,
    on_fire: Box<dyn Fn(HotkeyAction) + Send>,
    pending: Arc<Mutex<Option<Vec<(HotkeyAction, Hotkey)>>>>,
    errors: Arc<Mutex<Vec<(HotkeyAction, String)>>>,
    ready: SyncSender<u32>,
    done: Sender<()>,
) {
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::UnregisterHotKey;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetMessageW, MSG, PM_NOREMOVE, PeekMessageW, WM_HOTKEY, WM_USER,
    };

    let mut registered: Vec<(i32, HotkeyAction)> = Vec::new();
    unsafe {
        // Ép Windows tạo message queue cho thread NÀY trước khi báo id ra ngoài: chưa có
        // queue thì PostThreadMessageW từ thread khác sẽ thất bại.
        let mut msg: MSG = std::mem::zeroed();
        PeekMessageW(&mut msg, std::ptr::null_mut(), WM_USER, WM_USER, PM_NOREMOVE);
        let thread_id = GetCurrentThreadId();
        register_all(&bindings, &mut registered, &errors);
        let _ = ready.send(thread_id);

        loop {
            // GetMessageW: 0 = WM_QUIT, -1 = lỗi -> thoát ở cả hai ca (không panic).
            if GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) <= 0 {
                break;
            }
            match msg.message {
                WM_HOTKEY => {
                    let id = msg.wParam as i32;
                    if let Some((_, action)) = registered.iter().find(|(rid, _)| *rid == id) {
                        on_fire(*action);
                    }
                }
                WM_APP_REBIND => {
                    let fresh = pending.lock().ok().and_then(|mut slot| slot.take());
                    if let Some(fresh) = fresh {
                        for (id, _) in registered.drain(..) {
                            UnregisterHotKey(std::ptr::null_mut(), id);
                        }
                        register_all(&fresh, &mut registered, &errors);
                    }
                }
                _ => {}
            }
        }

        // Thoát: xả hết đăng ký để lần chạy sau (hoặc app khác) dùng lại được tổ hợp.
        for (id, _) in registered.drain(..) {
            UnregisterHotKey(std::ptr::null_mut(), id);
        }
    }
    let _ = done.send(());
}

/// Nền tảng không phải Windows: không đăng ký được, ghi lỗi rồi kết thúc ngay.
/// (Nhờ vậy `cargo test` thuần trên Linux vẫn biên dịch và chạy được toàn bộ module.)
#[cfg(not(windows))]
fn worker(
    bindings: Vec<(HotkeyAction, Hotkey)>,
    on_fire: Box<dyn Fn(HotkeyAction) + Send>,
    _pending: Arc<Mutex<Option<Vec<(HotkeyAction, Hotkey)>>>>,
    errors: Arc<Mutex<Vec<(HotkeyAction, String)>>>,
    ready: SyncSender<u32>,
    done: Sender<()>,
) {
    let _ = on_fire;
    if let Ok(mut slot) = errors.lock() {
        *slot = bindings
            .iter()
            .map(|(action, hk)| {
                (
                    *action,
                    format!("\"{}\" chỉ đăng ký được trên Windows.", hk.display()),
                )
            })
            .collect();
    }
    let _ = ready.send(0);
    let _ = done.send(());
}

/// Đăng ký toàn bộ `bindings` trên thread đang gọi (bắt buộc: `hwnd` null gắn vào thread
/// này) và ghi kết quả vào `errors`. `errors` được **thay mới** mỗi lần gọi nên nó luôn là
/// tình trạng của lần đăng ký gần nhất.
#[cfg(windows)]
fn register_all(
    bindings: &[(HotkeyAction, Hotkey)],
    registered: &mut Vec<(i32, HotkeyAction)>,
    errors: &Arc<Mutex<Vec<(HotkeyAction, String)>>>,
) {
    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::RegisterHotKey;

    let mut failed: Vec<(HotkeyAction, String)> = Vec::new();
    for (i, (action, hotkey)) in bindings.iter().enumerate() {
        let id = HOTKEY_ID_BASE + i as i32;
        // Hai hành động cùng tổ hợp: Windows cũng báo "đã bị chiếm", nhưng thủ phạm là
        // chính GameMode nên nói rõ cho người dùng.
        if bindings[..i].iter().any(|(_, other)| other == hotkey) {
            failed.push((
                *action,
                format!(
                    "\"{}\" bị trùng với phím tắt khác của GameMode.",
                    hotkey.display()
                ),
            ));
            continue;
        }
        // MOD_NOREPEAT: giữ phím không bắn liên tục.
        let ok = unsafe {
            RegisterHotKey(
                std::ptr::null_mut(),
                id,
                hotkey.mods | MOD_NOREPEAT,
                hotkey.vk,
            )
        };
        if ok != 0 {
            registered.push((id, *action));
        } else {
            let code = unsafe { GetLastError() };
            failed.push((*action, register_error(hotkey, code)));
        }
    }
    if let Ok(mut slot) = errors.lock() {
        *slot = failed;
    }
}

/// Câu lỗi tiếng Việt cho một lần `RegisterHotKey` thất bại.
#[cfg(windows)]
fn register_error(hotkey: &Hotkey, code: u32) -> String {
    use windows_sys::Win32::Foundation::ERROR_HOTKEY_ALREADY_REGISTERED;
    if code == ERROR_HOTKEY_ALREADY_REGISTERED {
        format!(
            "\"{}\" đã có app khác dùng tổ hợp này — chọn tổ hợp khác.",
            hotkey.display()
        )
    } else {
        format!(
            "\"{}\" không đăng ký được (Windows báo lỗi {}).",
            hotkey.display(),
            code
        )
    }
}

// ── Test ──────────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn hk(s: &str) -> Hotkey {
        Hotkey::parse(s).expect("chuỗi phải parse được")
    }

    #[test]
    fn parse_khong_phan_biet_hoa_thuong_va_khoang_trang() {
        let h = hk("  ctrl + ALT + b ");
        assert_eq!(h.mods, MOD_CONTROL | MOD_ALT);
        assert_eq!(h.vk, 0x42);
        assert_eq!(h.display(), "Ctrl+Alt+B");
        assert_eq!(hk("CONTROL+alt+B"), h);
    }

    #[test]
    fn display_chuan_hoa_thu_tu_modifier() {
        assert_eq!(hk("win+shift+ctrl+alt+f24").display(), "Ctrl+Alt+Shift+Win+F24");
        assert_eq!(hk("alt+num7").display(), "Alt+Num7");
        assert_eq!(hk("ctrl+0").display(), "Ctrl+0");
    }

    #[test]
    fn parse_phim_dac_biet() {
        assert_eq!(hk("Ctrl+Home").vk, 0x24);
        assert_eq!(hk("Ctrl+End").vk, 0x23);
        assert_eq!(hk("Ctrl+Insert").vk, 0x2D);
        assert_eq!(hk("Ctrl+Delete").vk, 0x2E);
        assert_eq!(hk("Ctrl+PageUp").vk, 0x21);
        assert_eq!(hk("Ctrl+PageDown").vk, 0x22);
        assert_eq!(hk("Ctrl+Space").vk, 0x20);
        assert_eq!(hk("Ctrl+ScrollLock").vk, 0x91);
        assert_eq!(hk("Ctrl+Num0").vk, 0x60);
        assert_eq!(hk("Ctrl+Num9").vk, 0x69);
        assert_eq!(hk("Ctrl+F1").vk, 0x70);
        assert_eq!(hk("Ctrl+F12").vk, 0x7B);
    }

    #[test]
    fn parse_phim_dung_mot_minh() {
        assert_eq!(hk("F13").mods, 0);
        assert_eq!(hk("f24").vk, 0x87);
        assert_eq!(hk("Pause").mods, 0);
        assert_eq!(hk("ScrollLock").mods, 0);
        // F12 / chữ cái thì vẫn phải có modifier.
        assert!(Hotkey::parse("F12").is_err());
        assert!(Hotkey::parse("B").is_err());
    }

    #[test]
    fn round_trip_parse_display() {
        for s in [
            "Ctrl+Alt+B",
            "Ctrl+Alt+R",
            "Ctrl+Alt+G",
            "F13",
            "Pause",
            "ScrollLock",
            "Shift+Win+Num5",
            "Ctrl+Alt+Shift+Win+PageDown",
            "Alt+Space",
            "Ctrl+0",
        ] {
            let h = hk(s);
            assert_eq!(h.display(), s, "display phải là dạng chuẩn của {s}");
            assert_eq!(hk(&h.display()), h, "parse(display()) phải trả lại chính nó");
        }
    }

    #[test]
    fn loi_thieu_modifier() {
        let e = Hotkey::parse("B").unwrap_err();
        assert!(e.contains("modifier"), "thông điệp lỗi: {e}");
    }

    #[test]
    fn loi_modifier_va_phim_khong_hop_le() {
        let e = Hotkey::parse("Ctr+Alt+B").unwrap_err();
        assert!(e.contains("không hiểu"), "thông điệp lỗi: {e}");
        assert!(Hotkey::parse("Ctrl+Alt+Foo").is_err());
        assert!(Hotkey::parse("Ctrl+F25").is_err());
        assert!(Hotkey::parse("Ctrl+Num10").is_err());
    }

    #[test]
    fn loi_dinh_dang() {
        assert!(Hotkey::parse("").is_err());
        assert!(Hotkey::parse("Ctrl++B").is_err());
        assert!(Hotkey::parse("Ctrl+B+Alt").is_err(), "modifier sau phím phải bị từ chối");
        assert!(Hotkey::parse("Ctrl+B+C").is_err(), "nhiều hơn một phím phải bị từ chối");
        let e = Hotkey::parse("Ctrl+Alt").unwrap_err();
        assert!(e.contains("thiếu phím"), "thông điệp lỗi: {e}");
        assert!(Hotkey::parse("+").is_err());
    }

    #[test]
    fn hanh_dong_co_khoa_config_va_nhan_tieng_viet() {
        assert_eq!(HotkeyAction::Boost.config_key(), "boost");
        assert_eq!(HotkeyAction::Restore.config_key(), "restore");
        assert_eq!(HotkeyAction::ToggleWindow.config_key(), "window");
        assert_eq!(HotkeyAction::Boost.label(), "Bật Game Boost");
        assert_eq!(HotkeyAction::Restore.label(), "Khôi phục");
        assert_eq!(HotkeyAction::ToggleWindow.label(), "Hiện/ẩn cửa sổ");
    }

    #[test]
    fn phim_tat_mac_dinh_parse_duoc() {
        let def = default_bindings();
        assert_eq!(def.len(), 3);
        assert_eq!(def[0], (HotkeyAction::Boost, "Ctrl+Alt+B"));
        assert_eq!(def[1], (HotkeyAction::Restore, "Ctrl+Alt+R"));
        assert_eq!(def[2], (HotkeyAction::ToggleWindow, "Ctrl+Alt+G"));
        for (_, s) in def {
            assert!(Hotkey::parse(s).is_ok(), "{s} phải parse được");
        }
    }

    /// Trên nền tảng không phải Windows, dịch vụ phải báo lỗi rõ ràng và drop không treo.
    #[cfg(not(windows))]
    #[test]
    fn service_tren_nen_tang_khac_bao_loi_va_khong_treo() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let fired = Arc::new(AtomicUsize::new(0));
        let counter = fired.clone();
        let svc = HotkeyService::start(
            vec![(HotkeyAction::Boost, hk("Ctrl+Alt+B"))],
            move |_| {
                counter.fetch_add(1, Ordering::SeqCst);
            },
        );
        let errors = svc.errors();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].0, HotkeyAction::Boost);
        assert!(errors[0].1.contains("Windows"), "thông điệp lỗi: {}", errors[0].1);
        svc.rebind(vec![(HotkeyAction::Restore, hk("Ctrl+Alt+R"))]);
        drop(svc); // không được panic/treo
    }
}
