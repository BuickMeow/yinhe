//! Pianoroll 左上角量化按钮（点击弹出量化预设选择 popup）。
//!
//! 位于时间标尺与键盘的交叉角落，点击切换量化预设。

use eframe::egui;

use yinhe_editor_core::quantize::QuantizePreset;

use super::PianoViewEvent;

/// 量化按钮上下文。
pub struct QuantizeBtnCtx {
    pub rect_min_x: f32,
    pub ruler_band_y: f32,
    pub kb_w: f32,
    pub ppq: u32,
    pub quantize: QuantizePreset,
}

/// 绘制量化按钮并处理点击。返回新的量化预设（若用户选择了新值）。
pub fn show(ui: &mut egui::Ui, ctx: QuantizeBtnCtx) -> Option<PianoViewEvent> {
    let QuantizeBtnCtx {
        rect_min_x,
        ruler_band_y,
        kb_w,
        ppq,
        quantize,
    } = ctx;

    let corner_rect = egui::Rect::from_min_size(
        egui::pos2(rect_min_x, ruler_band_y),
        egui::vec2(kb_w, crate::theme::RULER_H),
    );
    // 背景矩形：与 ruler 带对齐，画在键盘之上
    ui.painter()
        .rect_filled(corner_rect, 0.0, crate::theme::RULER_BG);
    // 右侧分隔线（与 ruler 对齐）
    ui.painter().line_segment(
        [
            egui::pos2(corner_rect.max.x, corner_rect.min.y),
            egui::pos2(corner_rect.max.x, corner_rect.max.y),
        ],
        egui::Stroke::new(1.0, crate::theme::RULER_DIVIDER),
    );

    let btn_size = 20.0;
    let btn_rect =
        egui::Rect::from_center_size(corner_rect.center(), egui::vec2(btn_size, btn_size));
    let btn_resp = ui.interact(
        btn_rect,
        egui::Id::new("pr_quantize_btn"),
        egui::Sense::click(),
    );
    let hovered = btn_resp.hovered();

    let icon_color = if hovered {
        crate::theme::ACCENT_ACTIVE
    } else {
        crate::theme::TEXT_MUTED
    };
    ui.painter().text(
        btn_rect.center(),
        egui::Align2::CENTER_CENTER,
        quantize.label(),
        egui::FontId::proportional(11.0),
        icon_color,
    );

    let mut pending_q = None;
    egui::Popup::from_toggle_button_response(&btn_resp)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            crate::widgets::quantize_popup::show(ui, ppq, quantize, &mut pending_q);
        });

    pending_q.map(PianoViewEvent::QuantizePreset)
}
