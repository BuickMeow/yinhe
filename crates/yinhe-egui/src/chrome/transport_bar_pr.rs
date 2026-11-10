//! transport bar 右侧的 PR 控制组：和弦指示器。
//! 方向切换已移到 PR 工具栏的 [→|↓] 双联按钮。
//!
//! 主音轨切换 / 显示音轨勾选已移入右栏「图层」选项卡，不再在此渲染。

use eframe::egui;
use rust_i18n::t;

use yinhe_editor_core::quantize::QuantizePreset;

/// PR 控制数据（只读：量化 / 和弦指示器）。
#[allow(dead_code)]
pub struct PrBarData {
    pub quantize: QuantizePreset,
    /// `Auto` 档在当前 PR 缩放下解析出的具体网格（量化弹窗内显示）。
    pub quantize_auto_resolved: QuantizePreset,
    /// 和弦指示器文本（实时 MIDI 按键优先，其次播放中光标处和弦）。
    pub chord: Option<String>,
}

fn chord_font() -> egui::FontId {
    egui::FontId::proportional(crate::theme::BODY_FONT)
}

/// 绘制右侧控制组（整组在 `right_to_left` 布局中贴右；不收拢）。
pub(crate) fn show_right_group(
    ui: &mut egui::Ui,
    data: &PrBarData,
    hovered_hint: &mut Option<String>,
) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
