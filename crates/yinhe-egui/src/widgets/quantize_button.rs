//! 量化入口：传输栏时间码左侧的纯文字下拉。
//!
//! 不再使用图标/外框：只显示档位文字（如 `1/16`），右侧一个小三角表示可下拉。
//! 点击后由调用方挂量化弹窗。

use eframe::egui;
use rust_i18n::t;
use yinhe_editor_core::quantize::QuantizePreset;

/// 控件宽度（文字 + 下拉三角）。
pub const WIDTH: f32 = 54.0;
/// 控件高度。
const HEIGHT: f32 = crate::theme::TRANSPORT_BTN_SIZE;
/// 文字字号。
const TEXT_SIZE: f32 = 13.0;
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

/// 绘制量化文字下拉，返回响应。
pub(crate) fn quantize_button(ui: &mut egui::Ui, quantize: QuantizePreset) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(WIDTH, HEIGHT), egui::Sense::click());
    super::a11y::name_button(&resp, t!("hint.timecode.quantize"));
    if ui.is_rect_visible(rect) {
        draw(ui, rect, &resp, quantize);
    }
    resp
}

fn draw(ui: &egui::Ui, rect: egui::Rect, resp: &egui::Response, quantize: QuantizePreset) {
    let color = if resp.is_pointer_button_down_on() {
        crate::theme::accent_active()
    } else if resp.hovered() {
        crate::theme::text_primary()
    } else {
        crate::theme::text_secondary()
    };
    let painter = ui.painter();
    let text = label(quantize);
    let tri_left = rect.max.x - 8.0;
    let text_rect = egui::Rect::from_min_max(rect.min, egui::pos2(tri_left - 4.0, rect.max.y));
    painter.text(
        text_rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        egui::FontId::proportional(TEXT_SIZE),
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_is_plain_quantize_text() {
        assert_eq!(label(QuantizePreset::Auto), "AUTO");
        assert_eq!(label(QuantizePreset::Off), "OFF");
        assert_eq!(label(QuantizePreset::Absolute(1)), "OFF");
        assert_eq!(label(QuantizePreset::Absolute(3)), "3");
        assert_eq!(label(QuantizePreset::Fraction(1, 16)), "1/16");
    }
}
