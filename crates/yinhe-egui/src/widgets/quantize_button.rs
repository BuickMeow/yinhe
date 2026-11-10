//! 量化入口：传输栏时间码左侧的纯文字下拉。
//!
//! 不再使用图标/外框：只显示档位文字（如 `1/16`），右侧一个小三角表示可下拉。
//! 点击后由调用方挂量化弹窗。

use eframe::egui;
use rust_i18n::t;
use yinhe_editor_core::quantize::QuantizePreset;

/// 标尺右端紧凑控件的宽/高。
pub const RULER_W: f32 = 46.0;
pub const RULER_H: f32 = 18.0;
/// 下拉三角宽/高。
const TRI_W: f32 = 5.0;
const TRI_H: f32 = 3.0;

/// 档位 → 文字。
fn label(quantize: QuantizePreset) -> String {
    match quantize {
        QuantizePreset::Auto => "AUTO".to_string(),
        QuantizePreset::Off | QuantizePreset::Absolute(1) => "OFF".to_string(),
        QuantizePreset::Absolute(n) => n.to_string(),
        QuantizePreset::Fraction(num, den) => format!("{num}/{den}"),
    }
}

/// 标尺右端的紧凑量化入口：纯文字 + 下拉三角，点击弹出量化菜单。
///
/// 返回用户新选的档位；调用方负责写回 AR/PR 各自的量化状态。
#[allow(clippy::too_many_arguments)]
pub(crate) fn show_ruler(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    current: QuantizePreset,
    ppq: u32,
    auto_resolved: QuantizePreset,
    id_salt: &str,
) -> Option<QuantizePreset> {
    let resp = ui.interact(rect, ui.scope_id().with(id_salt), egui::Sense::click());
    super::a11y::name_button(&resp, t!("hint.timecode.quantize"));
    let color = if resp.is_pointer_button_down_on() {
        crate::theme::accent_active()
    } else if resp.hovered() {
        crate::theme::text_primary()
    } else {
        crate::theme::text_secondary()
    };
    let painter = ui.painter();
    let text = label(current);
    let tri_left = rect.max.x - 8.0;
    let text_rect = egui::Rect::from_min_max(rect.min, egui::pos2(tri_left - 4.0, rect.max.y));
    painter.text(
        text_rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        egui::FontId::proportional(10.5),
        color,
    );
    let center = egui::pos2(tri_left + TRI_W * 0.5, rect.center().y + 1.0);
    painter.add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(center.x - TRI_W * 0.5, center.y - TRI_H * 0.5),
            egui::pos2(center.x + TRI_W * 0.5, center.y - TRI_H * 0.5),
            egui::pos2(center.x, center.y + TRI_H * 0.5),
        ],
        color,
        egui::Stroke::NONE,
    ));

    let mut pending = None;
    egui::Popup::from_toggle_button_response(&resp)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            super::quantize_popup::show(ui, ppq, current, auto_resolved, &mut pending);
        });
    pending
}
