//! Theme bảng màu và widget giao diện Tactical HUD cho GameMode (egui 0.29).

use eframe::egui::{
    self, pos2, vec2, viewport::ResizeDirection, Align2, Color32, CursorIcon, FontId, Margin, Rect,
    Rounding, Stroke, ViewportCommand,
};

// ==========================================
// BẢNG MÀU TACTICAL HUD
// ==========================================
/// Nền chính của ứng dụng (#07090C)
pub const BG: Color32 = Color32::from_rgb(7, 9, 12);
/// Nền thẻ/panel (#0D1116)
pub const CARD_BG: Color32 = Color32::from_rgb(13, 17, 22);
/// Nền thẻ nổi / hover / active (#131920)
pub const CARD_HOVER: Color32 = Color32::from_rgb(19, 25, 32);
/// Viền panel và phân cách (#1E2731)
pub const BORDER: Color32 = Color32::from_rgb(30, 39, 49);
/// Đường lưới mờ (#141B23)
pub const GRID: Color32 = Color32::from_rgb(20, 27, 35);

/// Chữ chính (#E8EDF2)
pub const TEXT_MAIN: Color32 = Color32::from_rgb(232, 237, 242);
/// Chữ phụ có độ tương phản cao (#8C99A6)
pub const TEXT_MUTED: Color32 = Color32::from_rgb(140, 153, 166);
/// Chữ mờ, ghi chú nhỏ (#55616D)
pub const TEXT_DIM: Color32 = Color32::from_rgb(85, 97, 109);

/// Accent chính: amber (#FFB020)
pub const ACCENT: Color32 = Color32::from_rgb(255, 176, 32);
/// Accent dữ liệu: cyan (#27E1C1)
pub const DATA_CYAN: Color32 = Color32::from_rgb(39, 225, 193);
/// Xanh lá thành công / sẵn sàng / giữ lại (#3DFF8A)
pub const SUCCESS: Color32 = Color32::from_rgb(61, 255, 138);
/// Cam cảnh báo (#FF8A1F)
pub const WARNING: Color32 = Color32::from_rgb(255, 138, 31);
/// Đỏ nguy hiểm / tắt / lỗi (#FF3B3B)
pub const DANGER: Color32 = Color32::from_rgb(255, 59, 59);

// ==========================================
// KHOẢNG CÁCH GIAO DIỆN (SPACING TOKENS)
// ==========================================
/// Padding của panel trái/phải: 14 -> 22
pub const SPACE_PANEL_PAD: f32 = 22.0;
/// Khoảng cách giữa các card: 10 -> 18
pub const SPACE_CARD_GAP: f32 = 18.0;
/// Padding trong card: 12 -> 18
pub const SPACE_CARD_PAD: f32 = 18.0;
/// Khoảng giữa header khối và nội dung: 12
pub const SPACE_HEADER_GAP: f32 = 12.0;
/// Item spacing mặc định: khoảng (10, 10)
pub const SPACE_ITEM_SPACING: egui::Vec2 = egui::vec2(10.0, 10.0);
/// Hàng app trong lưới: cao 56
pub const ROW_APP_HEIGHT: f32 = 56.0;
/// Hàng service trong lưới: cao 56
pub const ROW_SERVICE_HEIGHT: f32 = 56.0;
/// Khoảng cách giữa các ô trong lưới: 12 theo cả hai chiều
pub const SPACE_GRID_GAP: f32 = 12.0;
/// Nút Game Boost: cao 56
pub const BTN_BOOST_HEIGHT: f32 = 56.0;
/// Các nút phụ: cao 38
pub const BTN_SUB_HEIGHT: f32 = 38.0;
/// Khoảng cách giữa các nút trong toolbar của tab: 8
pub const SPACE_TOOLBAR_GAP: f32 = 8.0;
/// Toolbar cách danh sách: 14
pub const SPACE_TOOLBAR_MARGIN_BOTTOM: f32 = 14.0;
/// Line-height log thoáng hơn: thêm khoảng ~4px giữa các dòng
pub const SPACE_LOG_LINE_GAP: f32 = 4.0;
/// Chiều cao title bar tuỳ biến (~40px)
pub const TITLE_BAR_HEIGHT: f32 = 40.0;

// ==========================================
// CẤU HÌNH THEME VISUALS
// ==========================================
pub fn setup_visuals(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.override_text_color = Some(TEXT_MAIN);
    visuals.panel_fill = BG;
    visuals.window_fill = BG;
    visuals.faint_bg_color = CARD_BG;
    visuals.extreme_bg_color = BG; // Nền TextEdit

    visuals.hyperlink_color = ACCENT;
    visuals.selection.bg_fill = Color32::from_rgba_premultiplied(255, 176, 32, 45);
    visuals.selection.stroke = Stroke::new(1.0_f32, ACCENT);

    // Nút/ô không tương tác (disabled hoặc label nền)
    visuals.widgets.noninteractive.bg_fill = CARD_BG;
    visuals.widgets.noninteractive.weak_bg_fill = CARD_BG;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT_DIM);
    visuals.widgets.noninteractive.rounding = Rounding::ZERO;

    // Inactive (nút bình thường, textedit)
    visuals.widgets.inactive.bg_fill = CARD_HOVER;
    visuals.widgets.inactive.weak_bg_fill = CARD_HOVER;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT_MAIN);
    visuals.widgets.inactive.rounding = Rounding::ZERO;

    // Hovered
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(25, 34, 45);
    visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(25, 34, 45);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, TEXT_MAIN);
    visuals.widgets.hovered.rounding = Rounding::ZERO;

    // Active (nhấn giữ)
    visuals.widgets.active.bg_fill = ACCENT;
    visuals.widgets.active.weak_bg_fill = ACCENT;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, BG);
    visuals.widgets.active.rounding = Rounding::ZERO;

    // Open (popup, dropdown)
    visuals.widgets.open.bg_fill = CARD_HOVER;
    visuals.widgets.open.weak_bg_fill = CARD_HOVER;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    visuals.widgets.open.fg_stroke = Stroke::new(1.0_f32, TEXT_MAIN);
    visuals.widgets.open.rounding = Rounding::ZERO;

    ctx.set_visuals(visuals);
    ctx.style_mut(|s| {
        s.spacing.item_spacing = SPACE_ITEM_SPACING;
        s.spacing.scroll = egui::style::ScrollStyle::solid();
        s.spacing.scroll.bar_width = 6.0;
        s.spacing.scroll.bar_inner_margin = 2.0;
    });
}

// ==========================================
// NỀN CỬA SỔ: LƯỚI MỜ GRID 24PX
// ==========================================
/// Vẽ đường lưới mờ 24px màu `#141B23` ở nền panel.
pub fn paint_background_grid(ui: &mut egui::Ui) {
    let rect = ui.max_rect();
    if !rect.is_positive() {
        return;
    }
    let painter = ui.painter();
    let grid_size = 24.0_f32;
    let stroke = Stroke::new(1.0_f32, GRID);

    let start_x = (rect.left() / grid_size).floor() * grid_size;
    let mut x = start_x;
    while x <= rect.right() {
        if x >= rect.left() {
            painter.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], stroke);
        }
        x += grid_size;
    }

    let start_y = (rect.top() / grid_size).floor() * grid_size;
    let mut y = start_y;
    while y <= rect.bottom() {
        if y >= rect.top() {
            painter.line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], stroke);
        }
        y += grid_size;
    }
}

// ==========================================
// CARD KIỂU HUD: GÓC VÁT CHÉO + CORNER BRACKETS
// ==========================================
/// Tạo shape card HUD: vát góc 8px (trên-trái và dưới-phải), viền 1px, 4 khung góc L accent mờ.
pub fn hud_card_shape(rect: Rect, bg: Color32, border_color: Color32, accent_color: Color32) -> egui::Shape {
    let c = 8.0_f32; // Độ vát chamfer
    let points = vec![
        pos2(rect.left() + c, rect.top()),
        pos2(rect.right(), rect.top()),
        pos2(rect.right(), rect.bottom() - c),
        pos2(rect.right() - c, rect.bottom()),
        pos2(rect.left(), rect.bottom()),
        pos2(rect.left(), rect.top() + c),
    ];

    let base_poly = egui::Shape::convex_polygon(
        points,
        bg,
        Stroke::new(1.0_f32, border_color),
    );

    // Khung góc (corner brackets): 4 vạch L ở 4 góc, màu accent mờ
    let bracket_color = Color32::from_rgba_premultiplied(
        accent_color.r(),
        accent_color.g(),
        accent_color.b(),
        65,
    );
    let bracket_stroke = Stroke::new(1.5_f32, bracket_color);
    let arm = 7.0_f32;

    let brackets = vec![
        base_poly,
        // Góc trên-trái (vát): vạch dọc và ngang dọc theo mép
        egui::Shape::line_segment(
            [pos2(rect.left() + c, rect.top()), pos2(rect.left() + c + arm, rect.top())],
            bracket_stroke,
        ),
        egui::Shape::line_segment(
            [pos2(rect.left(), rect.top() + c), pos2(rect.left(), rect.top() + c + arm)],
            bracket_stroke,
        ),
        egui::Shape::line_segment(
            [pos2(rect.left(), rect.top() + c), pos2(rect.left() + c, rect.top())],
            bracket_stroke,
        ),
        // Góc trên-phải (vuông 90 độ)
        egui::Shape::line_segment(
            [pos2(rect.right() - arm, rect.top()), pos2(rect.right(), rect.top())],
            bracket_stroke,
        ),
        egui::Shape::line_segment(
            [pos2(rect.right(), rect.top()), pos2(rect.right(), rect.top() + arm)],
            bracket_stroke,
        ),
        // Góc dưới-trái (vuông 90 độ)
        egui::Shape::line_segment(
            [pos2(rect.left(), rect.bottom() - arm), pos2(rect.left(), rect.bottom())],
            bracket_stroke,
        ),
        egui::Shape::line_segment(
            [pos2(rect.left(), rect.bottom()), pos2(rect.left() + arm, rect.bottom())],
            bracket_stroke,
        ),
        // Góc dưới-phải (vát): vạch dọc và ngang dọc theo mép
        egui::Shape::line_segment(
            [pos2(rect.right() - c - arm, rect.bottom()), pos2(rect.right() - c, rect.bottom())],
            bracket_stroke,
        ),
        egui::Shape::line_segment(
            [pos2(rect.right(), rect.bottom() - c - arm), pos2(rect.right(), rect.bottom() - c)],
            bracket_stroke,
        ),
        egui::Shape::line_segment(
            [pos2(rect.right() - c, rect.bottom()), pos2(rect.right(), rect.bottom() - c)],
            bracket_stroke,
        ),
    ];

    egui::Shape::Vec(brackets)
}

/// Khung thẻ kiểu HUD: góc vát chéo, viền 1px, 4 khung góc corner brackets, padding 18.
pub fn card<R>(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    let bg_idx = ui.painter().add(egui::Shape::Noop);
    let resp = egui::Frame::none()
        .inner_margin(Margin::same(SPACE_CARD_PAD))
        .show(ui, add_contents);

    let rect = resp.response.rect;
    if rect.is_positive() {
        let shape = hud_card_shape(rect, CARD_BG, BORDER, ACCENT);
        ui.painter().set(bg_idx, shape);
    }
    resp
}

// ==========================================
// SECTION HEADER
// ==========================================
/// Tiêu đề khối: vạch accent dọc 3px, chữ in hoa letter-spacing rộng, mã nhỏ "// 01" bên phải.
pub fn section_header(ui: &mut egui::Ui, code_or_icon: &str, title: &str, subtitle: Option<&str>) {
    let right_code = if code_or_icon == "AUTO-SCROLL" {
        "AUTO-SCROLL".to_string()
    } else if code_or_icon.starts_with("//") {
        code_or_icon.to_string()
    } else if code_or_icon.chars().all(|c| c.is_ascii_digit()) && !code_or_icon.is_empty() {
        format!("// {}", code_or_icon)
    } else if title.contains("Tự động") {
        "// 01".to_string()
    } else if title.contains("Tối ưu") {
        "// 02".to_string()
    } else if title.contains("Terminal") || title.contains("Log") {
        "// LOG".to_string()
    } else if !code_or_icon.is_empty() && code_or_icon != "⚙" && code_or_icon != "⚡" {
        format!("// {}", code_or_icon)
    } else {
        "// SYS".to_string()
    };

    ui.horizontal(|ui| {
        // Vạch accent dọc 3px
        let (rect, _) = ui.allocate_exact_size(vec2(3.0, 14.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, Rounding::same(1.0), ACCENT);
        ui.add_space(4.0);

        ui.label(
            egui::RichText::new(title.to_uppercase())
                .size(12.5)
                .strong()
                .extra_letter_spacing(1.2)
                .color(TEXT_MAIN),
        );

        if !right_code.is_empty() {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(right_code)
                        .monospace()
                        .size(10.5)
                        .color(TEXT_DIM),
                );
            });
        }
    });

    if let Some(sub) = subtitle {
        ui.add_space(2.0);
        ui.label(
            egui::RichText::new(sub)
                .size(11.0)
                .color(TEXT_MUTED),
        );
    }
}

// ==========================================
// SEGMENTED PROGRESS BAR (RAM & PAGEFILE)
// ==========================================
/// Thanh chia đoạn (~20 ô nhỏ, cách nhau 2px).
/// Màu: < 60% cyan, 60–80% amber, > 80% đỏ.
pub fn segmented_bar(ui: &mut egui::Ui, ratio: f32) -> egui::Response {
    let height = 12.0_f32;
    let avail_w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(avail_w, height), egui::Sense::hover());
    let painter = ui.painter_at(rect);

    let segments = 20;
    let gap = 2.0_f32;
    let total_gaps = (segments - 1) as f32 * gap;
    let seg_w = ((rect.width() - total_gaps) / segments as f32).max(1.0);

    let clamped_ratio = ratio.clamp(0.0, 1.0);
    let filled_count = ((clamped_ratio * segments as f32).round() as usize).min(segments);

    let active_color = if clamped_ratio > 0.80 {
        DANGER
    } else if clamped_ratio >= 0.60 {
        ACCENT
    } else {
        DATA_CYAN
    };

    let inactive_fill = Color32::from_rgb(14, 19, 25);
    let inactive_stroke = Stroke::new(1.0_f32, Color32::from_rgb(26, 36, 47));

    for i in 0..segments {
        let x0 = rect.left() + i as f32 * (seg_w + gap);
        let seg_rect = Rect::from_min_size(pos2(x0, rect.top()), vec2(seg_w, height));
        if i < filled_count {
            painter.rect_filled(seg_rect, Rounding::ZERO, active_color);
        } else {
            painter.rect(seg_rect, Rounding::ZERO, inactive_fill, inactive_stroke);
        }
    }

    resp
}

// ==========================================
// SWITCH KIỂU KHỐI (BLOCK SWITCH)
// ==========================================
/// Vẽ công tắc kiểu "khối": hình chữ nhật góc vát, bên trong nút vuông, bật = amber.
pub fn draw_switch(painter: &egui::Painter, rect: Rect, on: bool, hovered: bool) {
    let c = 3.5_f32; // Độ vát góc
    let points = vec![
        pos2(rect.left() + c, rect.top()),
        pos2(rect.right(), rect.top()),
        pos2(rect.right(), rect.bottom() - c),
        pos2(rect.right() - c, rect.bottom()),
        pos2(rect.left(), rect.bottom()),
        pos2(rect.left(), rect.top() + c),
    ];

    let pad = 2.5_f32;
    let knob_side = (rect.height() - pad * 2.0).max(4.0);

    if on {
        let fill = if hovered {
            Color32::from_rgb(255, 195, 60)
        } else {
            ACCENT
        };
        painter.add(egui::Shape::convex_polygon(
            points,
            fill,
            Stroke::new(1.0_f32, Color32::from_rgb(255, 220, 120)),
        ));

        // Nút vuông bên trong (ở bên phải khi ON)
        let knob_rect = Rect::from_min_size(
            pos2(rect.right() - pad - knob_side, rect.top() + pad),
            vec2(knob_side, knob_side),
        );
        let knob_fill = Color32::from_rgb(10, 14, 21);
        painter.rect_filled(knob_rect, Rounding::ZERO, knob_fill);
    } else {
        let fill = if hovered {
            CARD_HOVER
        } else {
            Color32::from_rgb(11, 15, 20)
        };
        let border_stroke = if hovered {
            Stroke::new(1.0_f32, TEXT_MUTED)
        } else {
            Stroke::new(1.0_f32, BORDER)
        };
        painter.add(egui::Shape::convex_polygon(points, fill, border_stroke));

        // Nút vuông bên trong (ở bên trái khi OFF)
        let knob_rect = Rect::from_min_size(
            pos2(rect.left() + pad, rect.top() + pad),
            vec2(knob_side, knob_side),
        );
        let knob_fill = if hovered { TEXT_MAIN } else { TEXT_DIM };
        painter.rect_filled(knob_rect, Rounding::ZERO, knob_fill);
    }
}

/// Switch thu nhỏ độc lập (28×16) cho danh sách app khôi phục.
pub fn compact_switch(ui: &mut egui::Ui, value: &mut bool) -> bool {
    let size = vec2(28.0, 16.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    let changed = resp.clicked();
    if changed {
        *value = !*value;
    }
    draw_switch(&ui.painter(), rect, *value, resp.hovered());
    changed
}

/// Hàng toggle cài đặt: tiêu đề chữ chính, mô tả wrap, switch khối 36×20 bên phải.
pub fn toggle_row(ui: &mut egui::Ui, value: &mut bool, title: &str, desc: &str) -> bool {
    let switch_w = 36.0_f32;
    let switch_h = 20.0_f32;
    let pad_x = 8.0_f32;
    let pad_y = 6.0_f32;
    let spacing = 10.0_f32;

    let avail_w = ui.available_width();
    let text_w = (avail_w - pad_x * 2.0 - switch_w - spacing).max(60.0_f32);

    let title_galley = ui.painter().layout(
        title.to_string(),
        FontId::proportional(12.5),
        TEXT_MAIN,
        text_w,
    );
    let desc_galley = if !desc.is_empty() {
        Some(ui.painter().layout(
            desc.to_string(),
            FontId::proportional(11.0),
            TEXT_MUTED,
            text_w,
        ))
    } else {
        None
    };

    let title_h = title_galley.size().y;
    let desc_h = desc_galley.as_ref().map(|d| d.size().y + 2.0_f32).unwrap_or(0.0_f32);
    let content_h = title_h + desc_h;
    let total_h = (content_h + pad_y * 2.0_f32).max(switch_h + pad_y * 2.0_f32);

    let (rect, resp) = ui.allocate_exact_size(vec2(avail_w, total_h), egui::Sense::click());
    let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    let changed = resp.clicked();
    if changed {
        *value = !*value;
    }

    let hovered = resp.hovered();
    let painter = ui.painter_at(rect);

    if hovered {
        painter.rect_filled(rect, Rounding::ZERO, CARD_HOVER);
    }

    let text_x = rect.left() + pad_x;
    let text_y = rect.top() + (total_h - content_h) / 2.0_f32;
    painter.galley(pos2(text_x, text_y), title_galley, TEXT_MAIN);

    if let Some(dg) = desc_galley {
        painter.galley(pos2(text_x, text_y + title_h + 2.0_f32), dg, TEXT_MUTED);
    }

    let switch_rect = Rect::from_center_size(
        pos2(rect.right() - pad_x - switch_w / 2.0_f32, rect.center().y),
        vec2(switch_w, switch_h),
    );
    draw_switch(&painter, switch_rect, *value, hovered);

    changed
}

/// Dòng Service Windows: tên, tag exe, badge ĐANG CHẠY / ĐÃ DỪNG, mô tả và switch khối 36×20.
pub fn service_row(
    ui: &mut egui::Ui,
    on: &mut bool,
    svc_name: &str,
    display_name: &str,
    desc: &str,
    is_running: bool,
) -> bool {
    let switch_w = 36.0_f32;
    let switch_h = 20.0_f32;
    let pad_x = 8.0_f32;
    let spacing = 10.0_f32;

    let avail_w = ui.available_width();
    let text_w = (avail_w - pad_x * 2.0 - switch_w - spacing).max(60.0_f32);

    let title_text = format!("{} ({})", display_name, svc_name);
    let title_galley = ui.painter().layout_no_wrap(
        title_text,
        FontId::proportional(12.5),
        if *on { TEXT_MAIN } else { TEXT_MUTED },
    );

    let desc_galley = ui.painter().layout(
        desc.to_string(),
        FontId::proportional(11.0),
        TEXT_MUTED,
        text_w,
    );

    let title_h = title_galley.size().y;
    let desc_h = desc_galley.size().y + 2.0_f32;
    let content_h = title_h + desc_h;
    let total_h = ROW_SERVICE_HEIGHT;

    let (rect, resp) = ui.allocate_exact_size(vec2(avail_w, total_h), egui::Sense::click());
    let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    let changed = resp.clicked();
    if changed {
        *on = !*on;
    }

    let hovered = resp.hovered();
    let painter = ui.painter_at(rect);

    if hovered {
        painter.rect_filled(rect, Rounding::ZERO, CARD_HOVER);
    } else if *on {
        let bar = Rect::from_min_size(rect.min + vec2(0.0, 6.0), vec2(3.0, total_h - 12.0));
        painter.rect_filled(bar, Rounding::same(1.0), ACCENT);
    }

    let text_x = rect.left() + pad_x + if *on { 4.0 } else { 0.0 };
    let text_y = rect.top() + (total_h - content_h) / 2.0_f32;

    let title_w = title_galley.size().x;
    painter.galley(pos2(text_x, text_y), title_galley, TEXT_MAIN);

    let (badge_txt, badge_color) = if is_running {
        ("ĐANG CHẠY", SUCCESS)
    } else {
        ("ĐÃ DỪNG", TEXT_DIM)
    };
    if text_x + title_w + 8.0 + 55.0 < rect.right() - pad_x - switch_w {
        painter.text(
            pos2(text_x + title_w + 8.0, text_y + title_h / 2.0_f32),
            Align2::LEFT_CENTER,
            badge_txt,
            FontId::proportional(10.0),
            badge_color,
        );
    }

    painter.galley(pos2(text_x, text_y + title_h + 2.0_f32), desc_galley, TEXT_MUTED);

    let switch_rect = Rect::from_center_size(
        pos2(rect.right() - pad_x - switch_w / 2.0_f32, rect.center().y),
        vec2(switch_w, switch_h),
    );
    draw_switch(&painter, switch_rect, *on, hovered);

    changed
}

// ==========================================
// CALLOUT CẢNH BÁO
// ==========================================
/// Callout viền trái cam cảnh báo (#FF8A1F), góc vát nhẹ.
pub fn callout_warning(ui: &mut egui::Ui, text: &str) {
    let avail_w = ui.available_width();
    let text_galley = ui.painter().layout(
        text.to_string(),
        FontId::proportional(11.0),
        TEXT_MUTED,
        avail_w - 20.0,
    );
    let total_h = text_galley.size().y + 10.0;
    let (rect, _) = ui.allocate_exact_size(vec2(avail_w, total_h), egui::Sense::hover());
    let painter = ui.painter_at(rect);

    painter.rect_filled(rect, Rounding::ZERO, Color32::from_rgb(22, 18, 14));
    let bar = Rect::from_min_size(rect.min, vec2(3.0, total_h));
    painter.rect_filled(bar, Rounding::ZERO, WARNING);
    painter.galley(pos2(rect.left() + 10.0, rect.top() + 5.0), text_galley, TEXT_MUTED);
}

// ==========================================
// NÚT PHỤ (GHOST BUTTON): GÓC VÁT, HOVER AMBER
// ==========================================
pub struct GhostButton {
    text: String,
    size: f32,
}

impl GhostButton {
    pub fn new(text: impl Into<String>, size: f32) -> Self {
        Self {
            text: text.into(),
            size,
        }
    }
}

impl egui::Widget for GhostButton {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let galley = ui.painter().layout_no_wrap(
            self.text,
            FontId::proportional(self.size),
            TEXT_MAIN,
        );
        let pad_x = 10.0_f32;
        let pad_y = 5.0_f32;
        let text_size = galley.size();
        let desired_size = vec2(text_size.x + pad_x * 2.0, (text_size.y + pad_y * 2.0).max(28.0));

        let (rect, resp) = ui.allocate_at_least(desired_size, egui::Sense::click());
        let resp = resp.on_hover_cursor(CursorIcon::PointingHand);

        let hovered = resp.hovered();
        let painter = ui.painter_at(rect);

        let c = 4.0_f32;
        let points = vec![
            pos2(rect.left() + c, rect.top()),
            pos2(rect.right(), rect.top()),
            pos2(rect.right(), rect.bottom() - c),
            pos2(rect.right() - c, rect.bottom()),
            pos2(rect.left(), rect.bottom()),
            pos2(rect.left(), rect.top() + c),
        ];

        let bg_fill = if hovered {
            CARD_HOVER
        } else {
            Color32::from_rgb(13, 17, 22)
        };
        let border_stroke = if hovered {
            Stroke::new(1.0_f32, ACCENT)
        } else {
            Stroke::new(1.0_f32, BORDER)
        };

        painter.add(egui::Shape::convex_polygon(points, bg_fill, border_stroke));

        let text_pos = pos2(
            rect.center().x - text_size.x / 2.0,
            rect.center().y - text_size.y / 2.0,
        );
        painter.galley(text_pos, galley, if hovered { ACCENT } else { TEXT_MAIN });

        resp
    }
}

/// Nút ghost cỡ chữ 12.0
pub fn ghost_button(text: impl Into<String>) -> GhostButton {
    GhostButton::new(text, 12.0)
}

/// Nút ghost cỡ chữ tuỳ chỉnh
pub fn ghost_button_sized(text: impl Into<String>, size: f32) -> GhostButton {
    GhostButton::new(text, size)
}

// ==========================================
// NÚT CHÍNH: GAME BOOST (SỌC CHÉO CHẠY KHI BUSY)
// ==========================================
pub struct PrimaryButton {
    text: String,
    busy: bool,
}

impl PrimaryButton {
    pub fn new(text: impl Into<String>, busy: bool) -> Self {
        Self {
            text: text.into(),
            busy,
        }
    }
}

impl egui::Widget for PrimaryButton {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let desired_size = vec2(ui.available_width(), BTN_BOOST_HEIGHT);
        let (rect, resp) = ui.allocate_at_least(desired_size, egui::Sense::click());
        let resp = resp.on_hover_cursor(CursorIcon::PointingHand);

        let painter = ui.painter_at(rect);
        let c = 7.0_f32; // chamfer góc

        let points = vec![
            pos2(rect.left() + c, rect.top()),
            pos2(rect.right(), rect.top()),
            pos2(rect.right(), rect.bottom() - c),
            pos2(rect.right() - c, rect.bottom()),
            pos2(rect.left(), rect.bottom()),
            pos2(rect.left(), rect.top() + c),
        ];

        if self.busy {
            // Repaint liên tục khi busy để sọc chéo di chuyển
            ui.ctx().request_repaint();

            let bg_fill = Color32::from_rgb(32, 22, 6);
            let border_stroke = Stroke::new(1.0_f32, ACCENT);
            painter.add(egui::Shape::convex_polygon(points, bg_fill, border_stroke));

            // Hiệu ứng sọc chéo chạy (chuyển động theo thời gian)
            let stripe_painter = painter.with_clip_rect(rect);
            let time = ui.input(|i| i.time);
            let period = 20.0_f32;
            let offset = (time as f32 * 36.0) % period;
            let stripe_stroke = Stroke::new(6.0_f32, Color32::from_rgba_premultiplied(255, 176, 32, 85));

            let mut x = rect.left() - rect.height() - period + offset;
            while x < rect.right() + period {
                stripe_painter.line_segment(
                    [
                        pos2(x, rect.bottom()),
                        pos2(x + rect.height(), rect.top()),
                    ],
                    stripe_stroke,
                );
                x += period;
            }

            // Chữ "ĐANG THANH LỌC…"
            let text = "ĐANG THANH LỌC…";
            let galley = ui.painter().layout_no_wrap(
                text.to_string(),
                FontId::proportional(14.0),
                Color32::from_rgb(255, 235, 190),
            );
            let text_pos = pos2(
                rect.center().x - galley.size().x / 2.0,
                rect.center().y - galley.size().y / 2.0,
            );
            painter.text(
                pos2(text_pos.x + 1.0, text_pos.y + 1.0),
                Align2::LEFT_TOP,
                text,
                FontId::proportional(14.0),
                Color32::from_rgb(0, 0, 0),
            );
            painter.galley(text_pos, galley, Color32::from_rgb(255, 235, 190));
        } else {
            let hovered = resp.hovered();
            let bg_fill = if hovered {
                Color32::from_rgb(255, 195, 65)
            } else {
                ACCENT
            };
            let border_stroke = Stroke::new(1.0_f32, Color32::from_rgb(255, 225, 130));
            painter.add(egui::Shape::convex_polygon(points, bg_fill, border_stroke));

            // Corner bracket nhấn
            let bracket_stroke = Stroke::new(1.5_f32, Color32::from_rgb(30, 20, 5));
            painter.line_segment(
                [pos2(rect.right() - 8.0, rect.top() + 2.0), pos2(rect.right() - 2.0, rect.top() + 2.0)],
                bracket_stroke,
            );
            painter.line_segment(
                [pos2(rect.right() - 2.0, rect.top() + 2.0), pos2(rect.right() - 2.0, rect.top() + 8.0)],
                bracket_stroke,
            );

            let text = if self.text.is_empty() {
                "🚀  BẬT GAME BOOST"
            } else {
                &self.text
            };
            let text_color = Color32::from_rgb(10, 14, 21); // Chữ đen đậm in hoa
            let galley = ui.painter().layout_no_wrap(
                text.to_uppercase(),
                FontId::proportional(14.0),
                text_color,
            );
            let text_pos = pos2(
                rect.center().x - galley.size().x / 2.0,
                rect.center().y - galley.size().y / 2.0,
            );
            painter.galley(text_pos, galley, text_color);
        }

        resp
    }
}

pub fn primary_button(text: impl Into<String>, busy: bool) -> PrimaryButton {
    PrimaryButton::new(text, busy)
}

// ==========================================
// TAB BUTTON: KIỂU "TAB VÁT" + GẠCH CHÂN AMBER
// ==========================================
pub struct TabButton {
    text: String,
    active: bool,
    _active_color: Color32,
}

impl TabButton {
    pub fn new(text: impl Into<String>, active: bool, active_color: Color32) -> Self {
        Self {
            text: text.into(),
            active,
            _active_color: active_color,
        }
    }
}

impl egui::Widget for TabButton {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let text_color = if self.active {
            ACCENT
        } else {
            TEXT_MUTED
        };
        let galley = ui.painter().layout_no_wrap(
            self.text,
            FontId::proportional(12.0),
            text_color,
        );

        let pad_x = 14.0_f32;
        let h = 32.0_f32;
        let w = galley.size().x + pad_x * 2.0;

        let (rect, resp) = ui.allocate_at_least(vec2(w, h), egui::Sense::click());
        let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
        let hovered = resp.hovered();

        let painter = ui.painter_at(rect);

        // Tab vát: chamfer 4px ở góc trên-trái và trên-phải
        let c = 4.0_f32;
        let points = vec![
            pos2(rect.left(), rect.bottom()),
            pos2(rect.left(), rect.top() + c),
            pos2(rect.left() + c, rect.top()),
            pos2(rect.right() - c, rect.top()),
            pos2(rect.right(), rect.top() + c),
            pos2(rect.right(), rect.bottom()),
        ];

        let bg_fill = if self.active {
            CARD_HOVER
        } else if hovered {
            Color32::from_rgb(16, 21, 28)
        } else {
            Color32::from_rgb(10, 14, 18)
        };

        let border_stroke = if self.active {
            Stroke::new(1.0_f32, BORDER)
        } else {
            Stroke::new(1.0_f32, Color32::from_rgb(20, 27, 35))
        };

        painter.add(egui::Shape::convex_polygon(points, bg_fill, border_stroke));

        if self.active {
            // Gạch chân amber dày 2px ở tab đang chọn
            painter.line_segment(
                [
                    pos2(rect.left() + 2.0, rect.bottom() - 1.0),
                    pos2(rect.right() - 2.0, rect.bottom() - 1.0),
                ],
                Stroke::new(2.0_f32, ACCENT),
            );
        } else {
            painter.line_segment(
                [pos2(rect.left(), rect.bottom()), pos2(rect.right(), rect.bottom())],
                Stroke::new(1.0_f32, BORDER),
            );
        }

        let text_pos = pos2(
            rect.center().x - galley.size().x / 2.0,
            rect.center().y - galley.size().y / 2.0 - if self.active { 1.0 } else { 0.0 },
        );
        painter.galley(
            text_pos,
            galley,
            if self.active {
                ACCENT
            } else if hovered {
                TEXT_MAIN
            } else {
                TEXT_MUTED
            },
        );

        resp
    }
}

pub fn tab_button(text: impl Into<String>, active: bool, active_color: Color32) -> TabButton {
    TabButton::new(text, active, active_color)
}

// ==========================================
// BADGE TRẠNG THÁI (READY / PURGING) CÓ LED NHẤP NHÁY
// ==========================================
/// Chấm LED nhấp nháy khi busy, đứng yên khi ready; góc vát viền 1px.
pub fn status_badge(ui: &mut egui::Ui, busy: bool) {
    let (text, color) = if busy {
        ("PURGING...", DANGER)
    } else {
        ("SYSTEM READY", SUCCESS)
    };

    let font_id = FontId::proportional(10.5);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font_id, color);
    let led_radius = 3.5_f32;
    let pad_x = 8.0_f32;
    let total_w = pad_x * 2.0 + led_radius * 2.0 + 6.0 + galley.size().x;
    let total_h = 24.0_f32;

    let (rect, _) = ui.allocate_exact_size(vec2(total_w, total_h), egui::Sense::hover());
    let painter = ui.painter_at(rect);

    // Chamfer 4px
    let c = 4.0_f32;
    let points = vec![
        pos2(rect.left() + c, rect.top()),
        pos2(rect.right(), rect.top()),
        pos2(rect.right(), rect.bottom() - c),
        pos2(rect.right() - c, rect.bottom()),
        pos2(rect.left(), rect.bottom()),
        pos2(rect.left(), rect.top() + c),
    ];
    painter.add(egui::Shape::convex_polygon(
        points,
        CARD_HOVER,
        Stroke::new(1.0_f32, color),
    ));

    // LED dot: nhấp nháy khi busy
    let led_center = pos2(rect.left() + pad_x + led_radius, rect.center().y);
    let alpha = if busy {
        ui.ctx().request_repaint(); // Repaint chỉ khi busy
        let time = ui.input(|i| i.time);
        (((time * 6.0).sin().abs() * 200.0) as u8).max(45)
    } else {
        255
    };

    // Glow nhẹ ngoài LED
    let glow_color = Color32::from_rgba_premultiplied(
        color.r(),
        color.g(),
        color.b(),
        (alpha as f32 * 0.3) as u8,
    );
    painter.circle_filled(led_center, led_radius + 2.0, glow_color);

    // Lõi LED
    let led_color = Color32::from_rgba_premultiplied(color.r(), color.g(), color.b(), alpha);
    painter.circle_filled(led_center, led_radius, led_color);

    // Text
    let text_pos = pos2(
        rect.left() + pad_x + led_radius * 2.0 + 6.0,
        rect.center().y - galley.size().y / 2.0,
    );
    painter.galley(text_pos, galley, color);
}

// ==========================================
// TITLE BAR TUỲ BIẾN VÀ ĐIỀU KHIỂN CỬA SỔ
// ==========================================
/// Vẽ title bar tuỳ biến cao 40px phủ hết chiều rộng cửa sổ:
/// - Bên trái: icon tia sét amber, "DELTA FORCE" amber + "RAM PURGE" trắng, tag nhỏ
/// - Badge SYSTEM READY / PURGING di chuyển lên title bar
/// - Bên phải: 3 nút thu nhỏ / phóng to-khôi phục / đóng (vẽ bằng painter)
/// - Kéo cửa sổ (StartDrag) khi giữ vùng trống; double-click để phóng to/khôi phục
/// - Viền dưới mảnh + vạch accent amber phong cách Tactical HUD
pub fn render_title_bar(ctx: &egui::Context, busy: bool) {
    let is_maximized = ctx.input(|i| i.viewport().maximized).unwrap_or(false);

    egui::TopBottomPanel::top("custom_title_bar")
        .exact_height(TITLE_BAR_HEIGHT)
        .frame(
            egui::Frame::none()
                .fill(CARD_BG)
                .inner_margin(Margin::ZERO),
        )
        .show(ctx, |ui| {
            let panel_rect = ui.max_rect();
            let btn_w = 46.0_f32;
            let btn_h = TITLE_BAR_HEIGHT;

            // 1. Vùng kéo cửa sổ (StartDrag và Double-Click Maximize)
            let drag_rect = Rect::from_min_max(
                panel_rect.left_top(),
                pos2(panel_rect.right() - btn_w * 3.0, panel_rect.bottom()),
            );

            let drag_resp = ui.interact(
                drag_rect,
                ui.id().with("title_bar_drag_area"),
                egui::Sense::click_and_drag(),
            );

            if drag_resp.drag_started_by(egui::PointerButton::Primary) {
                ctx.send_viewport_cmd(ViewportCommand::StartDrag);
            }
            if drag_resp.double_clicked() {
                ctx.send_viewport_cmd(ViewportCommand::Maximized(!is_maximized));
            }

            // 2. Tương tác 3 nút điều khiển bên phải (Close, Maximize/Restore, Minimize)
            let close_rect = Rect::from_min_size(
                pos2(panel_rect.right() - btn_w, panel_rect.top()),
                vec2(btn_w, btn_h),
            );
            let max_rect = Rect::from_min_size(
                pos2(panel_rect.right() - btn_w * 2.0, panel_rect.top()),
                vec2(btn_w, btn_h),
            );
            let min_rect = Rect::from_min_size(
                pos2(panel_rect.right() - btn_w * 3.0, panel_rect.top()),
                vec2(btn_w, btn_h),
            );

            let close_resp = ui
                .interact(close_rect, ui.id().with("tb_close"), egui::Sense::click())
                .on_hover_cursor(CursorIcon::PointingHand);
            let max_resp = ui
                .interact(max_rect, ui.id().with("tb_max"), egui::Sense::click())
                .on_hover_cursor(CursorIcon::PointingHand);
            let min_resp = ui
                .interact(min_rect, ui.id().with("tb_min"), egui::Sense::click())
                .on_hover_cursor(CursorIcon::PointingHand);

            if close_resp.clicked() {
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
            if max_resp.clicked() {
                ctx.send_viewport_cmd(ViewportCommand::Maximized(!is_maximized));
            }
            if min_resp.clicked() {
                ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
            }

            // 3. Nội dung bên trái title bar
            let left_rect = Rect::from_min_max(
                panel_rect.left_top(),
                pos2(panel_rect.right() - btn_w * 3.0, panel_rect.bottom()),
            );
            let mut left_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(left_rect)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            left_ui.add_space(14.0);

            // Icon app: tia sét amber
            left_ui.label(
                egui::RichText::new("⚡")
                    .size(16.0)
                    .color(ACCENT),
            );
            left_ui.add_space(6.0);

            // Tên app
            left_ui.label(
                egui::RichText::new("DELTA FORCE")
                    .size(13.0)
                    .strong()
                    .extra_letter_spacing(1.0)
                    .color(ACCENT),
            );
            left_ui.add_space(3.0);
            left_ui.label(
                egui::RichText::new("RAM PURGE")
                    .size(13.0)
                    .strong()
                    .extra_letter_spacing(1.0)
                    .color(TEXT_MAIN),
            );
            left_ui.add_space(8.0);

            // Tag nhỏ
            left_ui.label(
                egui::RichText::new("TACTICAL MEMORY BOOSTER")
                    .monospace()
                    .size(9.5)
                    .color(TEXT_DIM),
            );

            left_ui.add_space(14.0);

            // Badge trạng thái di chuyển từ header cũ lên đây
            status_badge(&mut left_ui, busy);

            // 4. Painter vẽ các nút và đường viền
            let painter = ui.painter();

            // Nút Đóng (hover đỏ #E81123, icon trắng; thường: icon TEXT_MUTED)
            let close_hover = close_resp.hovered();
            if close_hover {
                painter.rect_filled(close_rect, Rounding::ZERO, Color32::from_rgb(232, 17, 35));
            }
            let close_color = if close_hover { Color32::WHITE } else { TEXT_MUTED };
            let cc = close_rect.center();
            let cd = 4.5_f32;
            let c_stroke = Stroke::new(1.1_f32, close_color);
            painter.line_segment([pos2(cc.x - cd, cc.y - cd), pos2(cc.x + cd, cc.y + cd)], c_stroke);
            painter.line_segment([pos2(cc.x + cd, cc.y - cd), pos2(cc.x - cd, cc.y + cd)], c_stroke);

            // Nút Phóng to / Khôi phục
            let max_hover = max_resp.hovered();
            if max_hover {
                painter.rect_filled(max_rect, Rounding::ZERO, CARD_HOVER);
            }
            let max_color = if max_hover { TEXT_MAIN } else { TEXT_MUTED };
            let mc = max_rect.center();
            let m_stroke = Stroke::new(1.0_f32, max_color);
            if is_maximized {
                // Restore icon: 2 ô vuông lồng nhau
                let back = Rect::from_min_size(pos2(mc.x - 2.0, mc.y - 5.0), vec2(7.5, 7.5));
                painter.rect_stroke(back, Rounding::ZERO, m_stroke);
                let front = Rect::from_min_size(pos2(mc.x - 5.0, mc.y - 2.0), vec2(7.5, 7.5));
                painter.rect_filled(
                    front,
                    Rounding::ZERO,
                    if max_hover { CARD_HOVER } else { CARD_BG },
                );
                painter.rect_stroke(front, Rounding::ZERO, m_stroke);
            } else {
                // Maximize icon: 1 ô vuông 9.5x9.5
                let sq = Rect::from_center_size(mc, vec2(9.5, 9.5));
                painter.rect_stroke(sq, Rounding::ZERO, m_stroke);
            }

            // Nút Thu nhỏ: gạch ngang
            let min_hover = min_resp.hovered();
            if min_hover {
                painter.rect_filled(min_rect, Rounding::ZERO, CARD_HOVER);
            }
            let min_color = if min_hover { TEXT_MAIN } else { TEXT_MUTED };
            let mnc = min_rect.center();
            painter.line_segment(
                [pos2(mnc.x - 5.0, mnc.y + 4.0), pos2(mnc.x + 5.0, mnc.y + 4.0)],
                Stroke::new(1.1_f32, min_color),
            );

            // 5. Viền phân cách dưới title bar + vạch accent amber phong cách HUD
            let y_bot = panel_rect.bottom() - 1.0;
            painter.line_segment(
                [pos2(panel_rect.left(), y_bot), pos2(panel_rect.right(), y_bot)],
                Stroke::new(1.0_f32, BORDER),
            );
            painter.line_segment(
                [pos2(panel_rect.left(), y_bot), pos2(panel_rect.left() + 90.0, y_bot)],
                Stroke::new(2.0_f32, ACCENT),
            );
        });
}

// ==========================================
// VIỀN CỬA SỔ & RESIZE BẰNG TỰ BẮT CHUỘT
// ==========================================
/// Khi không có viền Windows (decorations = false):
/// - Vẽ viền 1px màu BORDER quanh toàn bộ cửa sổ (khi không maximized) để không lẫn vào desktop
/// - Vùng bắt chuột ~6px quanh mép và 4 góc: đổi con trỏ resize và gửi BeginResize(direction) khi nhấn
pub fn handle_window_resize_and_border(ctx: &egui::Context) {
    let is_maximized = ctx.input(|i| i.viewport().maximized).unwrap_or(false);
    if is_maximized {
        return;
    }

    let screen_rect = ctx.screen_rect();
    if !screen_rect.is_positive() {
        return;
    }

    // 1. Viền 1px màu BORDER quanh cửa sổ
    let border_painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("window_outer_border"),
    ));
    border_painter.rect_stroke(
        screen_rect.shrink(0.5),
        Rounding::ZERO,
        Stroke::new(1.0_f32, BORDER),
    );

    // 2. Vùng bắt chuột resize ~6px mép và 12px ở góc
    let edge = 6.0_f32;
    let corner = 12.0_f32;

    if let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) {
        if screen_rect.contains(pos) {
            let dist_left = pos.x - screen_rect.left();
            let dist_right = screen_rect.right() - pos.x;
            let dist_top = pos.y - screen_rect.top();
            let dist_bottom = screen_rect.bottom() - pos.y;

            let on_edge_top = dist_top <= edge;
            let on_edge_bottom = dist_bottom <= edge;
            let on_edge_left = dist_left <= edge;
            let on_edge_right = dist_right <= edge;

            let near_corner_top = dist_top <= corner;
            let near_corner_bottom = dist_bottom <= corner;
            let near_corner_left = dist_left <= corner;
            let near_corner_right = dist_right <= corner;

            let resize_action = if near_corner_top && near_corner_left && (on_edge_top || on_edge_left) {
                Some((CursorIcon::ResizeNwSe, ResizeDirection::NorthWest))
            } else if near_corner_top && near_corner_right && (on_edge_top || on_edge_right) {
                Some((CursorIcon::ResizeNeSw, ResizeDirection::NorthEast))
            } else if near_corner_bottom && near_corner_left && (on_edge_bottom || on_edge_left) {
                Some((CursorIcon::ResizeNeSw, ResizeDirection::SouthWest))
            } else if near_corner_bottom && near_corner_right && (on_edge_bottom || on_edge_right) {
                Some((CursorIcon::ResizeNwSe, ResizeDirection::SouthEast))
            } else if on_edge_top {
                Some((CursorIcon::ResizeVertical, ResizeDirection::North))
            } else if on_edge_bottom {
                Some((CursorIcon::ResizeVertical, ResizeDirection::South))
            } else if on_edge_left {
                Some((CursorIcon::ResizeHorizontal, ResizeDirection::West))
            } else if on_edge_right {
                Some((CursorIcon::ResizeHorizontal, ResizeDirection::East))
            } else {
                None
            };

            if let Some((cursor, dir)) = resize_action {
                ctx.set_cursor_icon(cursor);
                if ctx.input(|i| i.pointer.primary_pressed()) {
                    ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
                }
            }
        }
    }
}
