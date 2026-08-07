//! 左上角量化按钮（点击弹出量化预设选择 popup），PR/AR 共用。
//!
//! PR 位于时间标尺与键盘的交叉角落，AR 位于音轨面板上方，
//! 两者只差角落矩形与 id，绘制与弹窗逻辑完全相同。

use eframe::egui;

use yinhe_editor_core::quantize::QuantizePreset;
/// 量化按钮上下文。
pub struct QuantizeBtnCtx {
    /// 按钮所在的角落矩形（背景 + 分隔线绘制范围）。
    pub corner_rect: egui::Rect,
    /// 按钮交互 id（PR/AR 各用不同 salt，避免 id 冲突）。
    pub id_salt: &'static str,
    pub ppq: u32,
    pub quantize: QuantizePreset,
}

/// 绘制量化按钮并处理点击。返回用户新选的量化预设（若有）。
pub fn show(ui: &mut egui::Ui, ctx: QuantizeBtnCtx) -> Option<QuantizePreset> {
    let QuantizeBtnCtx {
        corner_rect,
        id_salt,
        ppq,
        quantize,
    } = ctx;

    // 背景矩形：与 ruler 带对齐
    ui.painter()
        .rect_filled(corner_rect, 0.0, crate::theme::app_bg());
    // 右侧分隔线（与 ruler 对齐）
    ui.painter().line_segment(
        [
            egui::pos2(corner_rect.max.x, corner_rect.min.y),
            egui::pos2(corner_rect.max.x, corner_rect.max.y),
        ],
        egui::Stroke::new(1.0, crate::theme::ruler_divider()),
    );

    let btn_size = 20.0;
    let btn_rect =
        egui::Rect::from_center_size(corner_rect.center(), egui::vec2(btn_size, btn_size));
    let btn_resp = ui.interact(btn_rect, egui::Id::new(id_salt), egui::Sense::click());
    let hovered = btn_resp.hovered();

    // 统一悬浮/按下底色（图标按钮与 hover_highlight 同款）
    if hovered {
        let bg = if btn_resp.is_pointer_button_down_on() {
            crate::theme::pressed_color(crate::theme::app_bg())
        } else {
            crate::theme::hover_color(crate::theme::app_bg())
        };
        ui.painter().rect_filled(btn_rect, 4.0, bg);
    }

    let icon_color = if hovered {
        crate::theme::contrast_fg()
    } else {
        crate::theme::text_muted()
    };
    ui.painter().text(
        btn_rect.center(),
        egui::Align2::CENTER_CENTER,
        quantize.label(),
        egui::FontId::proportional(crate::theme::SMALL_FONT),
        icon_color,
    );

    let mut pending_q = None;
    egui::Popup::from_toggle_button_response(&btn_resp)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            crate::widgets::quantize_popup::show(ui, ppq, quantize, &mut pending_q);
        });

    pending_q
}
