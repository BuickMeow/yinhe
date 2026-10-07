//! 左侧工具栏（Photoshop 风格）：单列竖排全部编辑工具。
//!
//! 取代原 transport bar 的「工具菜单 + 图钉工具按钮」：工具不再藏在下拉里，
//! 而是常驻钢琴卷帘左侧单列，点击即切换；悬停提示走底部讲解行（mode bar）。
//! 仅钢琴卷帘可见时显示——工具只对 PR/AM 生效，因此只占 PR 那条带，不影响走带视图。

use eframe::egui;
use rust_i18n::t;

use crate::widgets::tools_panel::{ALL_TOOLS, Tool};

/// 在给定矩形内渲染左侧工具栏（由 PR 布局扣出的左侧竖条）。
///
/// 悬停提示走统一讲解行（见 `widgets::hint`）；离开工具栏时显示面板说明。
pub fn show(ui: &mut egui::Ui, rect: egui::Rect, active_tool: &mut Tool) {
    let btn_size = egui::vec2(
        crate::theme::TRANSPORT_BTN_SIZE,
        crate::theme::TRANSPORT_BTN_SIZE,
    );
    let mut hovered_hint: Option<String> = None;
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui.set_clip_rect(rect.intersect(ui.clip_rect()));
        ui.painter().rect_filled(rect, 0.0, crate::theme::app_bg());
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
            ui.add_space(6.0);
            for tool in ALL_TOOLS {
                if let Some(hint) = tool_button(ui, tool, active_tool, btn_size) {
                    hovered_hint = Some(hint);
                }
            }
        });
    });

    let over_bar = ui
        .input(|i| i.pointer.hover_pos())
        .is_some_and(|p| rect.contains(p));
    if let Some(hint) = hovered_hint {
        crate::widgets::hint::set(ui.ctx(), hint);
    } else if over_bar {
        crate::widgets::hint::set_region(ui.ctx(), t!("hint.panel.tool_bar"));
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
