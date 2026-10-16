//! 量化按钮：传输栏时间码左侧的量化入口。
//!
//! 一颗圆角底按钮（样式与菜单类按钮一致），内容随当前档位变化：
//! - `Auto` → AUTO 图标（不显示数值）；
//! - `Off` / `Absolute(1)` → OFF 图标（不显示数值）；
//! - 其他手动档位 → 浅色 ON 图标 + 数值（分数上下叠 / tick 数字）。
//!
//! 点击后由调用方挂量化弹窗（`Popup::from_toggle_button_response`）。

use eframe::egui;
use egui_material_icons::MaterialIcon;
use egui_material_icons::icons::{
    ICON_MOTION_PHOTOS_AUTO, ICON_MOTION_PHOTOS_OFF, ICON_MOTION_PHOTOS_ON,
};
use yinhe_editor_core::quantize::QuantizePreset;

/// 按钮宽度（固定：档位切换时布局不跳动）。
pub const WIDTH: f32 = 56.0;
/// 按钮高度（与传输栏其它按钮一致）。
const HEIGHT: f32 = crate::theme::TRANSPORT_BTN_SIZE;
/// 圆角（与 `flat_button_filled` 的有底按钮一致）。
const RADIUS: f32 = 6.0;
/// 图标字号。
const ICON_SIZE: f32 = crate::theme::TRANSPORT_BTN_FONT;
/// 数值字号（分数/数字）。
const NUM_SIZE: f32 = 11.0;
/// 手动档位下图标中心相对按钮中心的水平偏移（图标在左）。
const ICON_DX: f32 = -9.0;
/// 手动档位下数值中心相对按钮中心的水平偏移（数值在右）。
const VALUE_DX: f32 = 12.0;
/// 分数上下两行相对按钮中心的垂直偏移。
const FRAC_DY: f32 = 6.0;

/// 数值展示形式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ValueDisplay {
    /// 分数（分子, 分母）：上下叠放。
    Fraction(u32, u32),
    /// tick 数（单数字）。
    Tick(u32),
}

/// 由档位推导按钮状态：`(图标, 数值)`。数值 `None` = 只显示图标。
fn button_state(quantize: QuantizePreset) -> (MaterialIcon, Option<ValueDisplay>) {
    match quantize {
        QuantizePreset::Auto => (ICON_MOTION_PHOTOS_AUTO, None),
        QuantizePreset::Off => (ICON_MOTION_PHOTOS_OFF, None),
        // Tick 1 视作"关闭吸附"状态：用 OFF 图标。
        QuantizePreset::Absolute(1) => (ICON_MOTION_PHOTOS_OFF, None),
        QuantizePreset::Absolute(n) => (ICON_MOTION_PHOTOS_ON, Some(ValueDisplay::Tick(n))),
        QuantizePreset::Fraction(num, den) => (
            ICON_MOTION_PHOTOS_ON,
            Some(ValueDisplay::Fraction(num, den)),
        ),
    }
}

/// 绘制量化按钮，返回响应。
pub(crate) fn quantize_button(ui: &mut egui::Ui, quantize: QuantizePreset) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(WIDTH, HEIGHT), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        draw(ui, rect, &resp, quantize);
    }
    resp
}

fn draw(ui: &egui::Ui, rect: egui::Rect, resp: &egui::Response, quantize: QuantizePreset) {
    // 底色：与 `flat_button_filled` 的菜单类按钮一致（常态 btn_bg，hover/pressed 派生）。
    let base = crate::theme::btn_bg();
    let bg = if resp.is_pointer_button_down_on() {
        crate::theme::pressed_color(base)
    } else if resp.hovered() {
        crate::theme::hover_color(base)
    } else {
        base
    };
    ui.painter().rect_filled(rect, RADIUS, bg);

    let (icon, value) = button_state(quantize);
    let painter = ui.painter();
    let icon_font = egui::FontId::new(ICON_SIZE, icon.font_family());
    let center = rect.center();
    match value {
        None => {
            painter.text(
                center,
                egui::Align2::CENTER_CENTER,
                icon.codepoint,
                icon_font,
                crate::theme::text_primary(),
            );
        }
        Some(display) => {
            // 浅色图标 + 数值同框（数值为主体）。
            painter.text(
                center + egui::vec2(ICON_DX, 0.0),
                egui::Align2::CENTER_CENTER,
                icon.codepoint,
                icon_font,
                crate::theme::text_secondary(),
            );
            let num_font = egui::FontId::proportional(NUM_SIZE);
            let value_center = center + egui::vec2(VALUE_DX, 0.0);
            let color = crate::theme::text_primary();
            match display {
                ValueDisplay::Fraction(num, den) => {
                    // 分子在上、分母在下（叠加的分数字形）。
                    painter.text(
                        value_center - egui::vec2(0.0, FRAC_DY),
                        egui::Align2::CENTER_CENTER,
                        num.to_string(),
                        num_font.clone(),
                        color,
                    );
                    painter.text(
                        value_center + egui::vec2(0.0, FRAC_DY),
                        egui::Align2::CENTER_CENTER,
                        den.to_string(),
                        num_font,
                        color,
                    );
                }
                ValueDisplay::Tick(n) => {
                    painter.text(
                        value_center,
                        egui::Align2::CENTER_CENTER,
                        n.to_string(),
                        num_font,
                        color,
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 档位 → 按钮状态映射：Auto/OFF/1t 只有图标；手动档位浅色图标 + 数值。
    #[test]
    fn button_state_mapping() {
        let codepoint = |state: (MaterialIcon, Option<ValueDisplay>)| (state.0.codepoint, state.1);
        assert_eq!(
            codepoint(button_state(QuantizePreset::Auto)),
            (ICON_MOTION_PHOTOS_AUTO.codepoint, None)
        );
        assert_eq!(
            codepoint(button_state(QuantizePreset::Off)),
            (ICON_MOTION_PHOTOS_OFF.codepoint, None)
        );
        assert_eq!(
            codepoint(button_state(QuantizePreset::Absolute(1))),
            (ICON_MOTION_PHOTOS_OFF.codepoint, None)
        );
        assert_eq!(
            codepoint(button_state(QuantizePreset::Absolute(3))),
            (ICON_MOTION_PHOTOS_ON.codepoint, Some(ValueDisplay::Tick(3)))
        );
        assert_eq!(
            codepoint(button_state(QuantizePreset::Fraction(1, 16))),
            (
                ICON_MOTION_PHOTOS_ON.codepoint,
                Some(ValueDisplay::Fraction(1, 16))
            )
        );
    }
}
