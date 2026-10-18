//! Theme bảng màu và widget giao diện dùng chung cho GameMode (egui 0.29).

use eframe::egui::{self, pos2, vec2, Align2, Color32, CursorIcon, FontId, Margin, Rect, Rounding, Stroke};

// ==========================================
// BẢNG MÀU THỐNG NHẤT
// ==========================================
/// Nền chính của ứng dụng (#0A0E15)
pub const BG: Color32 = Color32::from_rgb(10, 14, 21);
/// Nền thẻ/card (#111823)
pub const CARD_BG: Color32 = Color32::from_rgb(17, 24, 35);
/// Nền thẻ khi hover hoặc active (#172130)
pub const CARD_HOVER: Color32 = Color32::from_rgb(23, 33, 48);
/// Viền thẻ và phân cách (#233042)
pub const BORDER: Color32 = Color32::from_rgb(35, 48, 66);

/// Chữ chính (#E6EDF5)
pub const TEXT_MAIN: Color32 = Color32::from_rgb(230, 237, 245);
/// Chữ phụ có độ tương phản cao (#93A4B8)
pub const TEXT_MUTED: Color32 = Color32::from_rgb(147, 164, 184);
/// Chữ mờ, ghi chú nhỏ (#64748B)
pub const TEXT_DIM: Color32 = Color32::from_rgb(100, 116, 139);

/// Màu nhấn chính / focus (#00E5FF)
pub const ACCENT: Color32 = Color32::from_rgb(0, 229, 255);
/// Xanh lá thành công / sẵn sàng (#00E0A0)
pub const SUCCESS: Color32 = Color32::from_rgb(0, 224, 160);
/// Cam cảnh báo (#FFA028)
pub const WARNING: Color32 = Color32::from_rgb(255, 160, 40);
/// Đỏ nguy hiểm / xoá (#FF6B6B)
pub const DANGER: Color32 = Color32::from_rgb(255, 107, 107);

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
    visuals.selection.bg_fill = Color32::from_rgba_premultiplied(0, 229, 255, 50);
    visuals.selection.stroke = Stroke::new(1.0_f32, ACCENT);

    // Nút/ô không tương tác (disabled hoặc label nền)
    visuals.widgets.noninteractive.bg_fill = CARD_BG;
    visuals.widgets.noninteractive.weak_bg_fill = CARD_BG;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(25, 34, 46));
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT_DIM);
    visuals.widgets.noninteractive.rounding = Rounding::same(6.0);

    // Inactive (nút bình thường, textedit)
    visuals.widgets.inactive.bg_fill = CARD_HOVER;
    visuals.widgets.inactive.weak_bg_fill = CARD_HOVER;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT_MAIN);
    visuals.widgets.inactive.rounding = Rounding::same(6.0);

    // Hovered
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(28, 40, 60);
    visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(28, 40, 60);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, TEXT_MAIN);
    visuals.widgets.hovered.rounding = Rounding::same(6.0);

    // Active (nhấn giữ)
    visuals.widgets.active.bg_fill = ACCENT;
    visuals.widgets.active.weak_bg_fill = ACCENT;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, BG);
    visuals.widgets.active.rounding = Rounding::same(6.0);

    // Open (popup, dropdown)
    visuals.widgets.open.bg_fill = CARD_HOVER;
    visuals.widgets.open.weak_bg_fill = CARD_HOVER;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    visuals.widgets.open.fg_stroke = Stroke::new(1.0_f32, TEXT_MAIN);
    visuals.widgets.open.rounding = Rounding::same(6.0);

    ctx.set_visuals(visuals);
    ctx.style_mut(|s| {
        s.spacing.scroll = egui::style::ScrollStyle::solid();
        s.spacing.scroll.bar_width = 8.0;
        s.spacing.scroll.bar_inner_margin = 3.0;
    });
}

// ==========================================
// CARD DÙNG CHUNG
// ==========================================
/// Khung thẻ thống nhất: fill CARD_BG, viền BORDER 1px, bo góc 10, padding 12.
pub fn card<R>(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    egui::Frame::none()
        .fill(CARD_BG)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .rounding(Rounding::same(10.0))
        .inner_margin(Margin::same(12.0))
        .show(ui, add_contents)
}

// ==========================================
// SECTION HEADER
// ==========================================
/// Tiêu đề khối: icon + title in hoa cỡ 13 màu accent, subtitle cỡ 11 màu chữ phụ (wrap).
pub fn section_header(ui: &mut egui::Ui, icon: &str, title: &str, subtitle: Option<&str>) {
    let header_text = if icon.is_empty() {
        title.to_uppercase()
    } else {
        format!("{} {}", icon, title.to_uppercase())
    };
    ui.label(
        egui::RichText::new(header_text)
            .size(13.0)
            .strong()
            .color(ACCENT),
    );
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
// SWITCH PAINTER & TOGGLE ROW
// ==========================================
/// Vẽ công tắc switch bằng painter.
/// Tiêu chuẩn (36×20) hoặc compact (28×16). Bật = ACCENT, tắt = viền xám / nút xám.
pub fn draw_switch(painter: &egui::Painter, rect: Rect, on: bool, hovered: bool) {
    let rounding = Rounding::same(rect.height() / 2.0);
    let pad = 2.5_f32;
    let knob_radius = (rect.height() - pad * 2.0) / 2.0;

    if on {
        let fill = if hovered {
            Color32::from_rgb(50, 240, 255)
        } else {
            ACCENT
        };
        painter.rect_filled(rect, rounding, fill);
        let knob_center = pos2(rect.right() - pad - knob_radius, rect.center().y);
        painter.circle_filled(knob_center, knob_radius, BG);
    } else {
        let border_stroke = if hovered {
            Stroke::new(1.5_f32, TEXT_MUTED)
        } else {
            Stroke::new(1.2_f32, BORDER)
        };
        let bg_fill = if hovered {
            CARD_HOVER
        } else {
            Color32::from_rgb(13, 19, 28)
        };
        painter.rect(rect, rounding, bg_fill, border_stroke);
        let knob_center = pos2(rect.left() + pad + knob_radius, rect.center().y);
        let knob_color = if hovered { TEXT_MAIN } else { TEXT_DIM };
        painter.circle_filled(knob_center, knob_radius, knob_color);
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

/// Hàng toggle cài đặt: tiêu đề chữ chính, mô tả cỡ 11 wrap, switch 36×20 bên phải.
/// Click cả hàng để đổi, hover sáng nhẹ.
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
        painter.rect_filled(rect, Rounding::same(6.0), CARD_HOVER);
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

/// Dòng Service Windows: tên, tag exe, badge ĐANG CHẠY / ĐÃ DỪNG, mô tả và switch 36×20.
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
    let pad_y = 6.0_f32;
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
    let total_h = (content_h + pad_y * 2.0_f32).max(42.0_f32);

    let (rect, resp) = ui.allocate_exact_size(vec2(avail_w, total_h), egui::Sense::click());
    let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    let changed = resp.clicked();
    if changed {
        *on = !*on;
    }

    let hovered = resp.hovered();
    let painter = ui.painter_at(rect);

    if hovered {
        painter.rect_filled(rect, Rounding::same(6.0), CARD_HOVER);
    } else if *on {
        let bar = Rect::from_min_size(rect.min + vec2(0.0, 4.0), vec2(3.0, total_h - 8.0));
        painter.rect_filled(bar, Rounding::same(1.5), ACCENT);
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
// CALLOUT & NÚT TIỆN ÍCH
// ==========================================
/// Callout viền trái màu cam cảnh báo, chữ cỡ 11 wrap.
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

    painter.rect_filled(rect, Rounding::same(4.0), Color32::from_rgb(26, 20, 14));
    let bar = Rect::from_min_size(rect.min, vec2(3.0, total_h));
    painter.rect_filled(bar, Rounding::same(1.5), WARNING);
    painter.galley(pos2(rect.left() + 10.0, rect.top() + 5.0), text_galley, TEXT_MUTED);
}

/// Nút kiểu ghost: nền CARD_HOVER, viền BORDER, chữ TEXT_MAIN, hover viền ACCENT.
pub fn ghost_button(text: impl Into<String>) -> egui::Button<'static> {
    let text = text.into();
    egui::Button::new(
        egui::RichText::new(text)
            .size(12.0)
            .color(TEXT_MAIN),
    )
    .fill(CARD_HOVER)
    .stroke(Stroke::new(1.0_f32, BORDER))
    .rounding(Rounding::same(6.0))
}

/// Nút ghost cỡ chữ tuỳ chỉnh.
pub fn ghost_button_sized(text: impl Into<String>, size: f32) -> egui::Button<'static> {
    let text = text.into();
    egui::Button::new(
        egui::RichText::new(text)
            .size(size)
            .color(TEXT_MAIN),
    )
    .fill(CARD_HOVER)
    .stroke(Stroke::new(1.0_f32, BORDER))
    .rounding(Rounding::same(6.0))
}

/// Nút chính "BẬT GAME BOOST": accent đặc, chữ đậm nổi bật.
pub fn primary_button(text: impl Into<String>, busy: bool) -> egui::Button<'static> {
    let text = text.into();
    if busy {
        egui::Button::new(
            egui::RichText::new(text)
                .size(14.0)
                .strong()
                .color(TEXT_MUTED),
        )
        .fill(Color32::from_rgb(20, 28, 38))
        .stroke(Stroke::new(1.0_f32, BORDER))
        .rounding(Rounding::same(8.0))
    } else {
        egui::Button::new(
            egui::RichText::new(text)
                .size(14.5)
                .strong()
                .color(BG),
        )
        .fill(ACCENT)
        .rounding(Rounding::same(8.0))
    }
}

/// Nút chọn Tab: active có viền màu nổi bật, inactive nền tối viền mờ.
pub fn tab_button(text: impl Into<String>, active: bool, active_color: Color32) -> egui::Button<'static> {
    let text = text.into();
    let (fill, stroke, text_color) = if active {
        (CARD_HOVER, Stroke::new(1.0_f32, active_color), active_color)
    } else {
        (Color32::from_rgb(13, 19, 28), Stroke::new(1.0_f32, BORDER), TEXT_MUTED)
    };
    egui::Button::new(
        egui::RichText::new(text)
            .size(11.5)
            .strong()
            .color(text_color),
    )
    .fill(fill)
    .stroke(stroke)
    .rounding(Rounding::same(6.0))
}
