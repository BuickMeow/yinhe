//! 左侧工具栏（Photoshop 风格）：单列竖排全部编辑工具。
//!
//! 取代原 transport bar 的「工具菜单 + 图钉工具按钮」：工具不再藏在下拉里，
//! 而是常驻左侧单列，点击即切换；悬停提示走底部讲解行（mode bar）。
//! 仅钢琴卷帘可见时显示——工具只对 PR/AM 生效。

use eframe::egui;

use crate::widgets::tools_panel::{ALL_TOOLS, Tool};

/// 渲染左侧工具栏。
///
/// `status_hint`：悬停工具时写入讲解行提示；离开工具栏时清空（与 transport bar 同规则）。
pub fn show(ui: &mut egui::Ui, active_tool: &mut Tool, status_hint: &mut Option<String>) {
    let btn_size = egui::vec2(
        crate::theme::TRANSPORT_BTN_SIZE,
        crate::theme::TRANSPORT_BTN_SIZE,
    );
    let inner = egui::Panel::left("tool_bar")
        .exact_size(crate::theme::TOOL_BAR_W)
        .resizable(false)
        .show_separator_line(false)
        .frame(egui::Frame {
            fill: crate::theme::app_bg(),
            inner_margin: egui::Margin::symmetric(4, 6),
            ..Default::default()
        })
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let mut hovered_hint: Option<String> = None;
            for tool in ALL_TOOLS {
                if let Some(hint) = tool_button(ui, tool, active_tool, btn_size) {
                    hovered_hint = Some(hint);
                }
            }
            hovered_hint
        });

    let over_bar = ui
        .input(|i| i.pointer.hover_pos())
        .is_some_and(|p| inner.response.rect.contains(p));
    if let Some(hint) = inner.inner {
        *status_hint = Some(hint);
    } else if over_bar {
        *status_hint = None;
    }
}

/// 单个工具按钮：图标按钮，选中态用 `selected_bg` 底 + 强调色图标。
/// 返回悬停时的提示文案（未悬停 = None）。
fn tool_button(
    ui: &mut egui::Ui,
    tool: Tool,
    active_tool: &mut Tool,
    btn_size: egui::Vec2,
) -> Option<String> {
    let is_active = *active_tool == tool;
    let color = if is_active {
        crate::theme::accent_active()
    } else {
        crate::theme::text_primary()
    };
    let sel_bg = is_active.then(crate::theme::selected_bg);
    let resp = ui
        .push_id(("tool_bar_btn", tool.pin_index()), |ui| {
            crate::widgets::flat::flat_button_filled(
                ui,
                tool.icon()
                    .rich_text()
                    .size(crate::theme::TRANSPORT_BTN_FONT)
                    .color(color),
                btn_size,
                sel_bg,
                true,
            )
        })
        .inner;
    if resp.clicked() {
        *active_tool = tool;
    }
    resp.hovered()
        .then(|| crate::chrome::transport_bar_actions::tool_hint(tool))
}
