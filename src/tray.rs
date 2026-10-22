//! Icon khay hệ thống (notification area, cạnh đồng hồ) + menu điều khiển GameMode.
//!
//! Vì cửa sổ đã ẩn thì eframe không còn gọi `update()`, mọi sự kiện menu / click icon
//! được xử lý ở thread riêng: thread này gọi thẳng `optimize` / `restore` / lưu config và
//! hiện cửa sổ bằng Win32 trên HWND đã lấy sẵn, không phụ thuộc event loop của eframe.
//!
//! `TrayIcon` và các mục menu của muda đều dùng `Rc` nên không `Send`: chúng chỉ được
//! tạo và giữ ở main thread (trong `App`). Thread sự kiện chỉ nhận id của mục menu.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::{optimize, restore, sys, ui, Shared};

/// Id các mục menu (thread sự kiện so khớp theo id, không giữ con trỏ tới mục menu).
const ID_OPEN: &str = "gm_open";
const ID_BOOST: &str = "gm_boost";
const ID_RESTORE: &str = "gm_restore";
const ID_AUTO: &str = "gm_auto";
const ID_QUIT: &str = "gm_quit";

pub const TOOLTIP_READY: &str = "GameMode — SẴN SÀNG";
pub const TOOLTIP_BUSY: &str = "GameMode — ĐANG THANH LỌC";

/// Bật khi người dùng chọn "Thoát" ở menu khay: `App::update` thấy cờ này thì không
/// chặn yêu cầu đóng nữa mà bỏ icon khay rồi thoát hẳn.
pub static QUIT: AtomicBool = AtomicBool::new(false);

/// Icon khay + các mục menu cần đồng bộ trạng thái. Giữ trong `App` để không bị drop.
pub struct Tray {
    icon: TrayIcon,
    boost: MenuItem,
    restore_item: MenuItem,
    check_auto: CheckMenuItem,
    /// Tooltip đang hiển thị, để chỉ gọi `set_tooltip` khi nội dung thật sự đổi.
    tooltip: String,
}

/// Tooltip theo trạng thái thật: đang tối ưu, hoặc commit gần chạm trần.
fn tooltip_text(sh: &Shared) -> String {
    if sh.busy.load(Ordering::SeqCst) {
        return TOOLTIP_BUSY.to_string();
    }
    if sh.commit_alert.load(Ordering::SeqCst) {
        return format!("{} · ⚠ Commit {:.0}%", TOOLTIP_READY, sh.commit_pct());
    }
    TOOLTIP_READY.to_string()
}

impl Tray {
    /// Tạo icon khay + menu chuột phải. Phải gọi trên main thread: trên Windows tray-icon
    /// cần message loop của thread đã tạo icon.
    ///
    /// Lưu ý: muda 0.21 không có API đặt mục "mặc định" (in đậm) cho menu khay Win32, nên
    /// "Mở GameMode" chỉ là mục đầu tiên, không in đậm.
    pub fn new(sh: &Shared) -> Result<Self, String> {
        let auto_on = sh.opts().auto_enabled;

        let menu = Menu::new();
        let open = MenuItem::with_id(ID_OPEN, "Mở GameMode", true, None);
        let sep1 = PredefinedMenuItem::separator();
        let boost = MenuItem::with_id(ID_BOOST, "Bật Game Boost", true, None);
        let restore_item = MenuItem::with_id(ID_RESTORE, "Khôi phục", true, None);
        // CheckMenuItem tự đổi dấu tick khi bấm; `sync` kéo nó về đúng `auto:enabled`.
        let check_auto =
            CheckMenuItem::with_id(ID_AUTO, "Tự động khi mở game", true, auto_on, None);
        let sep2 = PredefinedMenuItem::separator();
        let quit = MenuItem::with_id(ID_QUIT, "Thoát", true, None);
        menu.append_items(&[
            &open,
            &sep1,
            &boost,
            &restore_item,
            &check_auto,
            &sep2,
            &quit,
        ])
        .map_err(|e| format!("không thêm được mục menu: {e}"))?;

        let icon = tray_icon_from_png().ok_or_else(|| "không đọc/resize được icon.png".to_string())?;
        let tooltip = tooltip_text(sh);
        let icon = TrayIconBuilder::new()
            .with_tooltip(tooltip.clone())
            .with_icon(icon)
            .with_menu(Box::new(menu))
            // Click trái để hiện cửa sổ, menu chỉ mở khi bấm chuột phải.
            .with_menu_on_left_click(false)
            .with_menu_on_right_click(true)
            .build()
            .map_err(|e| format!("không tạo được icon khay: {e}"))?;

        Ok(Self {
            icon,
            boost,
            restore_item,
            check_auto,
            tooltip,
        })
    }

    /// Đồng bộ tooltip / dấu tick / trạng thái disable theo trạng thái hiện tại (gọi trong `update`).
    pub fn sync(&mut self, sh: &Shared) {
        let busy = sh.busy.load(Ordering::SeqCst);
        let tip = tooltip_text(sh);
        if tip != self.tooltip {
            self.tooltip = tip;
            let _ = self
                .icon
                .set_tooltip(Some(self.tooltip.clone()));
        }
        // Đang tối ưu/khôi phục thì hai lệnh này bị `busy` chặn -> disable cho khỏi bấm hụt.
        self.boost.set_enabled(!busy);
        self.restore_item.set_enabled(!busy);
        let auto_on = sh.opts().auto_enabled;
        if self.check_auto.is_checked() != auto_on {
            self.check_auto.set_checked(auto_on);
        }
    }

    /// Bỏ icon khỏi khay (dùng khi thoát hẳn).
    pub fn remove(&mut self) {
        let _ = self.icon.set_visible(false);
    }
}

/// Ảnh icon khay: `icon.png` (256×256) resize về 32×32.
fn tray_icon_from_png() -> Option<Icon> {
    let img = image::load_from_memory(include_bytes!("../icon.png"))
        .ok()?
        .into_rgba8();
    let small = image::imageops::resize(&img, 32, 32, image::imageops::FilterType::Lanczos3);
    let (w, h) = small.dimensions();
    Icon::from_rgba(small.into_raw(), w, h).ok()
}

/// Thread nhận sự kiện menu + click icon. Sống tới khi app thoát; mỗi loại sự kiện
/// một thread vì cả hai đều là kênh chặn (không cần thêm crate channel).
pub fn start_event_thread(sh: Shared, ctx: eframe::egui::Context, hwnd: isize) {
    {
        let sh = sh.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            for event in MenuEvent::receiver().iter() {
                on_menu(&sh, &ctx, hwnd, event.id().as_ref());
            }
        });
    }
    std::thread::spawn(move || {
        for event in TrayIconEvent::receiver().iter() {
            match event {
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
                | TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                } => {
                    sys::show_main_window(hwnd);
                    ctx.request_repaint();
                }
                _ => {}
            }
        }
    });
}

/// Xử lý một mục menu vừa được bấm (chạy ở thread sự kiện, cửa sổ có thể đang ẩn).
fn on_menu(sh: &Shared, ctx: &eframe::egui::Context, hwnd: isize, id: &str) {
    match id {
        ID_OPEN => {
            sys::show_main_window(hwnd);
            ctx.request_repaint();
        }
        ID_BOOST => {
            if sh.busy.load(Ordering::SeqCst) {
                return;
            }
            // Không log trước: `do_optimize` xoá nhật ký ở đầu nên dòng đó sẽ mất.
            optimize(sh.clone(), false);
            ctx.request_repaint();
        }
        ID_RESTORE => {
            if sh.busy.load(Ordering::SeqCst) {
                return;
            }
            // `do_restore` cũng xoá nhật ký ở đầu.
            restore(sh.clone(), false);
            ctx.request_repaint();
        }
        ID_AUTO => {
            let on = {
                let mut opts = sh.opts.lock().unwrap();
                opts.auto_enabled = !opts.auto_enabled;
                opts.auto_enabled
            };
            // Công tắc trên UI đọc lại từ `Shared` nên đổi ở đây là UI cũng đổi theo.
            if !sh.save_config() {
                sh.log(
                    "  ⚠ [TRAY] Không ghi được gamemode.cfg — lựa chọn sẽ mất khi thoát.".into(),
                    ui::DANGER,
                );
            }
            sh.log(
                format!(
                    "  • [TRAY] Tự động khi mở game: {}.",
                    if on { "BẬT" } else { "TẮT" }
                ),
                if on { ui::SUCCESS } else { ui::TEXT_MUTED },
            );
            ctx.request_repaint();
        }
        ID_QUIT => quit(ctx, hwnd),
        _ => {}
    }
}

/// Thoát hẳn: nhờ main thread bỏ icon khay rồi đóng app (WM_CLOSE vẫn tới được cửa sổ
/// đang ẩn). Nếu vì lý do nào đó không có frame nào chạy, thoát cứng sau 1,5 giây.
fn quit(ctx: &eframe::egui::Context, hwnd: isize) {
    QUIT.store(true, Ordering::SeqCst);
    sys::post_close(hwnd);
    ctx.request_repaint();
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(1500));
        std::process::exit(0);
    });
}

/// `--traytest`: tạo icon khay + menu, bơm message trong `ms` mili-giây rồi bỏ icon.
/// Trả về mô tả kết quả để ghi ra traytest.txt khi thử trên Windows.
pub fn test_tray(ms: u64) -> String {
    let mut lines: Vec<String> = Vec::new();

    let menu = Menu::new();
    let open = MenuItem::with_id(ID_OPEN, "Mở GameMode", true, None);
    let sep = PredefinedMenuItem::separator();
    let boost = MenuItem::with_id(ID_BOOST, "Bật Game Boost", true, None);
    let quit = MenuItem::with_id(ID_QUIT, "Thoát", true, None);
    match menu.append_items(&[&open, &sep, &boost, &quit]) {
        Ok(()) => lines.push("menu: OK (4 mục)".into()),
        Err(e) => lines.push(format!("menu: LỖI {e}")),
    }

    let icon = match tray_icon_from_png() {
        Some(i) => {
            lines.push("icon.png -> 32x32: OK".into());
            Some(i)
        }
        None => {
            lines.push("icon.png -> 32x32: LỖI (không đọc hoặc không resize được)".into());
            None
        }
    };

    let tray = match icon {
        Some(icon) => {
            match TrayIconBuilder::new()
                .with_tooltip(TOOLTIP_READY)
                .with_icon(icon)
                .with_menu(Box::new(menu))
                .with_menu_on_left_click(false)
                .build()
            {
                Ok(t) => {
                    lines.push("Shell_NotifyIcon (tạo icon khay): OK".into());
                    Some(t)
                }
                Err(e) => {
                    lines.push(format!("Shell_NotifyIcon (tạo icon khay): LỖI {e}"));
                    None
                }
            }
        }
        None => {
            lines.push("Shell_NotifyIcon (tạo icon khay): BỎ QUA (không có icon)".into());
            None
        }
    };

    let (pumped, menu_events, tray_events) = pump_messages(ms);
    lines.push(format!("đã bơm {pumped} message trong {ms} ms"));
    lines.push(format!("sự kiện menu nhận được: {menu_events}"));
    lines.push(format!("sự kiện click icon nhận được: {tray_events}"));

    drop(tray);
    lines.push("đã bỏ icon khỏi khay".into());
    lines.join("\n")
}

/// Bơm message của thread hiện tại (icon khay và menu Win32 cần message loop).
/// Trả về (số message đã bơm, số sự kiện menu, số sự kiện click icon).
fn pump_messages(ms: u64) -> (u64, usize, usize) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
    };
    let deadline = Instant::now() + Duration::from_millis(ms);
    let mut pumped = 0u64;
    let mut menu_events = 0usize;
    let mut tray_events = 0usize;
    while Instant::now() < deadline {
        unsafe {
            let mut msg: MSG = std::mem::zeroed();
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
                pumped += 1;
            }
        }
        while MenuEvent::receiver().try_recv().is_ok() {
            menu_events += 1;
        }
        while TrayIconEvent::receiver().try_recv().is_ok() {
            tray_events += 1;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    (pumped, menu_events, tray_events)
}
