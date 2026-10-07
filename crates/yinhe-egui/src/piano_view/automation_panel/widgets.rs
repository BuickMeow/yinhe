use eframe::egui;
use egui_material_icons::icons::*;
use rust_i18n::t;

use yinhe_types::{AutomationLane, AutomationPanelView, AutomationTarget};

use crate::right_panel::{InfoContent, RightTab};
use crate::theme;

use super::constants::automation_targets;
use super::interaction;

/// 分割条拖拽：调整面板高度。写入下一帧生效，避免帧内布局抖动。
pub(crate) fn handle_split_drag(
    ui: &mut egui::Ui,
    panel: &mut AutomationPanelView,
    handle_rect: egui::Rect,
    index: usize,
) {
    let handle_resp =
        crate::widgets::split_handle::horizontal(ui, format!("auto_handle_{}", index), handle_rect);
    crate::widgets::hint::hover(ui.ctx(), &handle_resp, t!("hint.pr.auto_split"));
    let press_on_handle = ui
        .input(|i| i.pointer.press_origin())
        .is_some_and(|p| handle_rect.contains(p));
    if press_on_handle && handle_resp.dragged() {
        let delta = handle_resp.drag_delta().y;
        panel.panel_height = (panel.panel_height - delta).clamp(
            yinhe_types::automation_panel_view::MIN_PANEL_HEIGHT,
            yinhe_types::automation_panel_view::MAX_PANEL_HEIGHT,
        );
        panel.dirty = true;
        ui.ctx().request_repaint();
    }
    if handle_resp.double_clicked() {
        panel.panel_height = yinhe_types::automation_panel_view::DEFAULT_PANEL_HEIGHT;
        panel.dirty = true;
        ui.ctx().request_repaint();
    }
}

/// 左侧 target 选择器：图标按钮 + 弹出菜单（velocity / 设备参数 / 自定义 CC）。
pub(crate) fn show_target_combo(
    ui: &mut egui::Ui,
    panel: &mut AutomationPanelView,
    combo_rect: egui::Rect,
    panels_area_rect: egui::Rect,
    editing_is_conductor: bool,
    channel: Option<u8>,
) {
    // Conductor 无设备通道：只列 Tempo（非 Tempo 编辑由 dispatch 层禁用）。
    let channel = if editing_is_conductor { None } else { channel };
    ui.painter().rect_filled(combo_rect, 0.0, theme::app_bg());
    let combo_inner = combo_rect.shrink(4.0);
    ui.scope_builder(egui::UiBuilder::new().max_rect(combo_inner), |ui| {
        ui.set_clip_rect(combo_inner.intersect(panels_area_rect));
        let layout = egui::Layout::top_down(egui::Align::Center);
        ui.with_layout(layout, |ui| {
            let target_resp = crate::widgets::hover::hover_button(
                ui,
                ICON_TIMELINE.codepoint,
                egui::FontId::new(crate::theme::ICON_FONT, ICON_TIMELINE.font_family()),
                crate::theme::text_label(),
                false,
            );
            crate::widgets::hint::hover(ui.ctx(), &target_resp, t!("hint.pr.target_combo"));
            let popup_id = ui.id().with("auto_target_popup");
            let is_open = ui
                .data_mut(|d| d.get_persisted::<bool>(popup_id))
                .unwrap_or(false);
            if target_resp.clicked() {
                ui.data_mut(|d| d.insert_persisted(popup_id, !is_open));
            }
            if is_open {
                let popup_pos = egui::pos2(target_resp.rect.left(), target_resp.rect.bottom());
                let area_resp = egui::Area::new(popup_id)
                    .order(egui::Order::Foreground)
                    .fixed_pos(popup_pos)
                    .show(ui.ctx(), |ui| {
                        egui::Frame::menu(ui.style()).show(ui, |ui| {
                            ui.set_min_width(140.0);
                            ui.set_max_width(140.0);
                            // Conductor 与其他轨道显示范围一致（仅 Tempo 可编辑由 dispatch 层限制）
                            let vel_selected = panel.show_velocity;
                            if ui
                                .add(crate::widgets::menu::menu_item_button(
                                    ui,
                                    vel_selected,
                                    t!("automation.velocity"),
                                ))
                                .clicked()
                            {
                                panel.show_velocity = true;
                                panel.dirty = true;
                                ui.ctx().data_mut(|d| d.insert_persisted(popup_id, false));
                            }
                            ui.separator();
                            for target in &automation_targets(channel) {
                                let name = target.display_name();
                                let selected =
                                    !panel.show_velocity && panel.selected_target == *target;
                                if ui
                                    .add(crate::widgets::menu::menu_item_button(ui, selected, name))
                                    .clicked()
                                {
                                    panel.selected_target = target.clone();
                                    panel.show_velocity = false;
                                    panel.dirty = true;
                                    ui.ctx().data_mut(|d| d.insert_persisted(popup_id, false));
                                }
                            }
                            ui.separator();
                            ui.label(t!("automation.custom_cc").as_ref());
                            let mut cc_input = match &panel.selected_target {
                                AutomationTarget::CC { controller } => *controller as i32,
                                _ => 0,
                            };
                            let old_cc = cc_input;
                            let cc_resp = ui.add(
                                crate::widgets::numeric_input::decimal_drag_value(&mut cc_input)
                                    .range(0..=127)
                                    .speed(1),
                            );
                            crate::widgets::hint::hover(
                                ui.ctx(),
                                &cc_resp,
                                t!("hint.pr.custom_cc"),
                            );
                            if cc_input != old_cc {
                                panel.selected_target = AutomationTarget::CC {
                                    controller: cc_input as u8,
                                };
                                panel.show_velocity = false;
                                panel.dirty = true;
                            }
                        });
                    });
                if ui.input(|i| i.pointer.any_pressed())
                    && let Some(pos) = ui.input(|i| i.pointer.interact_pos())
                    && !area_resp.response.rect.contains(pos)
                    && !target_resp.rect.contains(pos)
                {
                    ui.data_mut(|d| d.insert_persisted(popup_id, false));
                }
            }
            ui.add_space(4.0);
        });
    });
}

/// 右键锚点：设置 info_content 打开信息面板，并清理 interaction 记录的 temp data。
pub(crate) fn apply_right_click_anchor(
    ui: &mut egui::Ui,
    panel_count: usize,
    automation_lanes: &[AutomationLane],
    info_content: &mut Option<InfoContent>,
    right_tab: &mut Option<RightTab>,
) {
    for i in 0..panel_count {
        let right_click_id = ui.id().with("auto_right_click").with(i);
        if let Some(anchor) = ui
            .ctx()
            .data(|d| d.get_temp::<interaction::RightClickAnchor>(right_click_id))
        {
            let event_idx = automation_lanes
                .iter()
                .find(|l| l.target == anchor.target)
                .and_then(|l| l.events.iter().position(|e| e.tick == anchor.old_tick))
                .unwrap_or(0);
            *info_content = Some(InfoContent::Anchor {
                track_idx: anchor.track_idx,
                lane_idx: anchor.lane_idx,
                event_idx,
                target: anchor.target.clone(),
            });
            *right_tab = Some(RightTab::Info);
            let edit_tick_id = ui.id().with("auto_right_tick").with(i);
            let edit_value_id = ui.id().with("auto_right_value").with(i);
            let was_open_id = ui.id().with("auto_right_was_open").with(i);
            ui.ctx().data_mut(|d| {
                d.remove::<interaction::RightClickAnchor>(right_click_id);
                d.remove::<f64>(edit_tick_id);
                d.remove::<f64>(edit_value_id);
                d.remove::<bool>(was_open_id);
            });
        }
    }
}

/// 面板底部中央的关闭按钮：点谁删谁。
///
/// 返回是否被点击（调用方在遍历结束后从面板列表移除该索引）。
pub(crate) fn show_panel_close(
    ui: &mut egui::Ui,
    panel_rect: egui::Rect,
    grid_area: egui::Rect,
    index: usize,
) -> bool {
    const BTN: f32 = 20.0;
    let center = egui::pos2(grid_area.center().x, panel_rect.max.y - BTN * 0.5 - 2.0);
    let btn_rect = egui::Rect::from_center_size(center, egui::vec2(BTN, BTN));
    let resp = ui.interact(
        btn_rect,
        ui.id().with(("auto_panel_close", index)),
        egui::Sense::click(),
    );
    let color = crate::widgets::hover::hover_button_color(&resp, theme::text_label(), false);
    ui.painter().text(
        btn_rect.center(),
        egui::Align2::CENTER_CENTER,
        ICON_CLOSE.codepoint,
        egui::FontId::new(crate::theme::ICON_FONT, ICON_CLOSE.font_family()),
        color,
    );
    crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.pr.auto_remove"));
    resp.clicked()
}
