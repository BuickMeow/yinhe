//! 左侧工具栏（Photoshop 风格）：单列竖排全部编辑工具。
//!
//! 取代原 transport bar 的「工具菜单 + 图钉工具按钮」：工具不再藏在下拉里，
//! 而是常驻钢琴卷帘左侧单列，点击即切换；悬停提示走底部讲解行（mode bar）。
//! 仅钢琴卷帘可见时显示——工具只对 PR/AM 生效，因此只占 PR 那条带，不影响走带视图。

use eframe::egui;
use egui_material_icons::icons::{ICON_ADD_CHART, ICON_BAR_CHART};
use rust_i18n::t;

use crate::widgets::tools_panel::{ALL_TOOLS, Tool};
use yinhe_types::AutomationPanelView;

/// 在给定矩形内渲染左侧工具栏（由 PR 布局扣出的左侧竖条）。
///
/// 顶部为编辑工具，底部两个为自动化（AM）控制：
/// 最下方 `bar_chart` 开关显示/隐藏自动化面板（选中态与工具一致），
/// 其上方 `addchart` 新增一个自动化面板。删除面板的按钮在各面板内部。
///
/// 悬停提示走统一讲解行（见 `widgets::hint`）；离开工具栏时显示面板说明。
pub fn show(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    active_tool: &mut Tool,
    show_panels: &mut bool,
    panels: &mut Vec<AutomationPanelView>,
    show_automation_buttons: bool,
) {
    let btn_size = egui::vec2(
        crate::theme::TRANSPORT_BTN_SIZE,
        crate::theme::TRANSPORT_BTN_SIZE,
    );
    let mut hovered_hint: Option<String> = None;
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui.set_clip_rect(rect.intersect(ui.clip_rect()));
        ui.painter()
            .rect_filled(rect, 0.0, crate::theme::track_bg());
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
            ui.add_space(6.0);
            for tool in ALL_TOOLS {
                if let Some(hint) = tool_button(ui, tool, active_tool, btn_size) {
                    hovered_hint = Some(hint);
                }
            }
        });
        // 自动化按钮固定贴底：最下方 = 开关，其上方 = 新增。
        if show_automation_buttons {
            // 贴住工具栏底缘（留 2px，避免圆角被裁）；add 在其正上方。
            let mut center_y = rect.max.y - 2.0 - btn_size.y * 0.5;
            if let Some(hint) = automation_toggle_button(
                ui,
                egui::pos2(rect.center().x, center_y),
                btn_size,
                show_panels,
                panels,
            ) {
                hovered_hint = Some(hint);
            }
            center_y -= btn_size.y + 2.0;
            if let Some(hint) = automation_add_button(
                ui,
                egui::pos2(rect.center().x, center_y),
                btn_size,
                show_panels,
                panels,
            ) {
                hovered_hint = Some(hint);
            }
        }
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
            crate::widgets::flat::flat_button_filled_ghost(
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

/// 固定中心点的图标按钮：与工具按钮同款选中底 + 强调色图标。
fn icon_button_at(
    ui: &mut egui::Ui,
    center: egui::Pos2,
    btn_size: egui::Vec2,
    id_salt: &str,
    icon: egui_material_icons::MaterialIcon,
    is_active: bool,
) -> egui::Response {
    let color = if is_active {
        crate::theme::accent_active()
    } else {
        crate::theme::text_primary()
    };
    let sel_bg = is_active.then(crate::theme::selected_bg);
    let rect = egui::Rect::from_center_size(center, btn_size);
    ui.push_id(id_salt, |ui| {
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            crate::widgets::flat::flat_button_filled_ghost(
                ui,
                icon.rich_text()
                    .size(crate::theme::TRANSPORT_BTN_FONT)
                    .color(color),
                btn_size,
                sel_bg,
                true,
            )
        })
        .inner
    })
    .inner
}

/// 自动化显示/隐藏开关（贴工具栏底部）：选中态与工具一致。
fn automation_toggle_button(
    ui: &mut egui::Ui,
    center: egui::Pos2,
    btn_size: egui::Vec2,
    show_panels: &mut bool,
    panels: &mut Vec<AutomationPanelView>,
) -> Option<String> {
    let resp = icon_button_at(
        ui,
        center,
        btn_size,
        "tool_bar_auto_toggle",
        ICON_BAR_CHART,
        *show_panels,
    );
    if resp.clicked() {
        *show_panels = !*show_panels;
        if *show_panels && panels.is_empty() {
            panels.push(AutomationPanelView::default());
        }
    }
    resp.hovered()
        .then(|| t!("hint.pr.auto_toggle").to_string())
}

/// 新增一个自动化面板（开关上方）。
fn automation_add_button(
    ui: &mut egui::Ui,
    center: egui::Pos2,
    btn_size: egui::Vec2,
    show_panels: &mut bool,
    panels: &mut Vec<AutomationPanelView>,
) -> Option<String> {
    let resp = icon_button_at(
        ui,
        center,
        btn_size,
        "tool_bar_auto_add",
        ICON_ADD_CHART,
        false,
    );
    if resp.clicked() {
        *show_panels = true;
        panels.push(AutomationPanelView::default());
    }
    resp.hovered().then(|| t!("hint.pr.auto_add").to_string())
}
