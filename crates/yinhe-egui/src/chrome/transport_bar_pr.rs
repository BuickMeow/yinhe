//! transport bar 右侧的 PR 控制组：瀑布流方向（单按钮切换）/ 和弦指示器
//! （方向按钮左侧）。整组在 `right_to_left` 布局中绘制，贴右且不参与工具收拢。
//!
//! 主音轨切换 / 显示音轨勾选已移入右栏「图层」选项卡，不再在此渲染。

use eframe::egui;
use egui_material_icons::MaterialIcon;
use egui_material_icons::icons::ICON_DOUBLE_ARROW;
use rust_i18n::t;

use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_types::Orientation;

/// 右侧控制组元素间距（紧凑，与工具区的 add_space(2.0) 同量级）。
pub(crate) const GAP: f32 = 4.0;

/// PR 控制数据（只读：量化 / 和弦指示器）。
pub struct PrBarData {
    pub quantize: QuantizePreset,
    /// Auto（自适应）当前解析出的具体网格；非 Auto 时 == `quantize`。
    pub quantize_effective: QuantizePreset,
    /// 和弦指示器文本（实时 MIDI 按键优先，其次播放中光标处和弦）。
    pub chord: Option<String>,
}

/// 图标字体（transport 按钮统一字号）。
pub(crate) fn icon_font(icon: MaterialIcon) -> egui::FontId {
    egui::FontId::new(crate::theme::TRANSPORT_BTN_FONT, icon.font_family())
}

fn chord_font() -> egui::FontId {
    egui::FontId::proportional(crate::theme::BODY_FONT)
}

/// 绘制右侧控制组（整组在 `right_to_left` 布局中贴右；不收拢）。
pub(crate) fn show_right_group(
    ui: &mut egui::Ui,
    data: &PrBarData,
    vertical: bool,
    set_orientation: &mut Option<Orientation>,
    hovered_hint: &mut Option<String>,
) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        // ── 瀑布流方向：单按钮切换 ──
        // 向右双箭头 = 横向（时间向右）；顺时针 90°（向下）= 纵向瀑布流。
        // 不使用「强调色 = 已开启」语义：图标方向本身区分当前方向。
        let angle = if vertical {
            std::f32::consts::FRAC_PI_2
        } else {
            0.0
        };
        let resp = crate::widgets::hover::hover_button_rotated(
            ui,
            ICON_DOUBLE_ARROW.codepoint,
            icon_font(ICON_DOUBLE_ARROW),
            crate::theme::text_label(),
            false,
            angle,
        );
        if resp.clicked() {
            *set_orientation = Some(if vertical {
                Orientation::Horizontal
            } else {
                Orientation::Vertical
            });
        }
        if resp.hovered() {
            *hovered_hint = Some(t!("hint.orientation.toggle").to_string());
        }
        ui.add_space(GAP);

        // ── 和弦指示器（排在方向按钮之后，即右起排列区最左；只读文本）──
        if let Some(chord) = &data.chord {
            let galley = ui.painter().layout_no_wrap(
                chord.clone(),
                chord_font(),
                egui::Color32::PLACEHOLDER,
            );
            let (rect, resp) = ui.allocate_exact_size(galley.size(), egui::Sense::hover());
            ui.painter()
                .galley(rect.min, galley, crate::theme::text_primary());
            if resp.hovered() {
                *hovered_hint = Some(t!("hint.chord_indicator").to_string());
            }
        }
    });
}
