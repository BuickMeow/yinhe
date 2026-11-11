use eframe::egui;
use egui_material_icons::icons::*;
use rust_i18n::t;

use yinhe_types::automation::{
    EASING_DIRECTIONS, EASING_FAMILIES, EASING_PRESETS, EasingDirection, EasingFamily,
    EasingPreset, SegmentShape,
};
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

/// combo 列通用的「图标按钮 + 下挂弹出菜单」外壳。
///
/// `content` 返回 `Some(v)` 表示已选择：关闭菜单并把 `v` 返回给调用方。
/// target 选择器和锚点插值选择器共用这套打开/关闭/点外部关闭逻辑。
fn combo_menu<T>(
    ui: &mut egui::Ui,
    icon: egui_material_icons::MaterialIcon,
    tooltip: &str,
    id_suffix: &'static str,
    content: impl FnOnce(&mut egui::Ui) -> Option<T>,
) -> Option<T> {
    let resp = crate::widgets::hover::hover_button(
        ui,
        icon.codepoint,
        egui::FontId::new(crate::theme::ICON_FONT, icon.font_family()),
        crate::theme::text_label(),
        false,
        tooltip,
    );
    crate::widgets::hint::hover(ui.ctx(), &resp, tooltip);
    let popup_id = ui.scope_id().with(id_suffix);
    let is_open = ui
        .data_mut(|d| d.get_persisted::<bool>(popup_id))
        .unwrap_or(false);
    if resp.clicked() {
        ui.data_mut(|d| d.insert_persisted(popup_id, !is_open));
    }
    if !is_open {
        return None;
    }
    let mut chosen = None;
    let area_resp = egui::Area::new(popup_id)
        .order(egui::Order::Foreground)
        .fixed_pos(egui::pos2(resp.rect.left(), resp.rect.bottom()))
        .show(ui.ctx(), |ui| {
            egui::Frame::menu(ui.style()).show(ui, |ui| {
                chosen = content(ui);
                if chosen.is_some() {
                    ui.ctx().data_mut(|d| d.insert_persisted(popup_id, false));
                }
            });
        });
    if ui.input(|i| i.pointer.any_pressed())
        && let Some(pos) = ui.input(|i| i.pointer.interact_pos())
        && !area_resp.response.rect.contains(pos)
        && !resp.rect.contains(pos)
    {
        ui.data_mut(|d| d.insert_persisted(popup_id, false));
    }
    chosen
}

/// 左侧 combo 列：上面是自动化目标，下面是锚点插值形状。
///
/// 返回需要应用的 `(tick, shape)`；有选中锚点时只改选中，否则改整条 lane。
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
pub(crate) fn show_target_combo(
    ui: &mut egui::Ui,
    panel: &mut AutomationPanelView,
    combo_rect: egui::Rect,
    panels_area_rect: egui::Rect,
    editing_is_conductor: bool,
    channel: Option<u8>,
    lane: Option<&AutomationLane>,
) -> Option<Vec<(u32, SegmentShape)>> {
    let mut applied: Option<Vec<(u32, SegmentShape)>> = None;
    // Conductor 无设备通道：只列 Tempo（非 Tempo 编辑由 dispatch 层禁用）。
    let channel = if editing_is_conductor { None } else { channel };
    ui.painter().rect_filled(combo_rect, 0.0, theme::app_bg());
    let combo_inner = combo_rect.shrink(4.0);
    ui.scope_builder(egui::UiBuilder::new().max_rect(combo_inner), |ui| {
        ui.set_clip_rect(combo_inner.intersect(panels_area_rect));
        let layout = egui::Layout::top_down(egui::Align::Center);
        ui.with_layout(layout, |ui| {
            combo_menu(
                ui,
                ICON_TIMELINE,
                t!("hint.pr.target_combo").as_ref(),
                "auto_target_popup",
                |ui| {
                    let ctx = ui.ctx().clone();
                    let scale = |v: f32| crate::scaling::scaled_font(&ctx, v);
                    let targets = automation_targets(channel);
                    // 动态宽度：按最宽条目测量（原固定 140 会把长名截断）。
                    let font = egui::TextStyle::Button.resolve(ui.style());
                    let measure = |s: &str| {
                        ctx.fonts_mut(|f| {
                            f.layout_no_wrap(s.to_owned(), font.clone(), egui::Color32::WHITE)
                                .size()
                                .x
                        })
                    };
                    let stepper_w = scale(96.0);
                    let mut content_w = measure(t!("automation.velocity").as_ref());
                    for target in &targets {
                        content_w = content_w.max(measure(&target.display_name()));
                    }
                    content_w = content_w.max(
                        measure(t!("automation.custom_cc").as_ref()) + stepper_w + scale(10.0),
                    );
                    content_w = content_w.max(
                        measure(t!("automation.custom_rpn").as_ref()) + stepper_w + scale(10.0),
                    );
                    let width = (content_w + scale(24.0)).clamp(scale(150.0), scale(380.0));
                    ui.set_min_width(width);
                    ui.set_max_width(width);

                    let mut picked = false;
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
                        picked = true;
                    }
                    ui.separator();
                    for target in &targets {
                        let name = target.display_name();
                        let selected = !panel.show_velocity && panel.selected_target == *target;
                        if ui
                            .add(crate::widgets::menu::menu_item_button(ui, selected, name))
                            .clicked()
                        {
                            panel.selected_target = target.clone();
                            panel.show_velocity = false;
                            panel.dirty = true;
                            picked = true;
                        }
                    }
                    ui.separator();
                    // 自定义 CC / RPN：目标名在左、数字步进器在右。
                    {
                        let mut v = match &panel.selected_target {
                            AutomationTarget::CC { controller } => *controller as i32,
                            _ => 0,
                        };
                        let old = v;
                        crate::widgets::rows::inline_row(ui, |ui| {
                            ui.label(t!("automation.custom_cc").as_ref());
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    crate::widgets::stepper::stepper(&mut v)
                                        .range(0..=127)
                                        .step(1.0)
                                        .width(stepper_w)
                                        .show(ui);
                                },
                            );
                        });
                        if v != old {
                            panel.selected_target = AutomationTarget::CC {
                                controller: v.clamp(0, 127) as u8,
                            };
                            panel.show_velocity = false;
                            panel.dirty = true;
                        }
                    }
                    {
                        let mut v = match &panel.selected_target {
                            AutomationTarget::Rpn { parameter } => *parameter as i32,
                            _ => 0,
                        };
                        let old = v;
                        crate::widgets::rows::inline_row(ui, |ui| {
                            ui.label(t!("automation.custom_rpn").as_ref());
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    crate::widgets::stepper::stepper(&mut v)
                                        .range(0..=127)
                                        .step(1.0)
                                        .width(stepper_w)
                                        .show(ui);
                                },
                            );
                        });
                        if v != old {
                            panel.selected_target = AutomationTarget::Rpn {
                                parameter: v.clamp(0, 127) as u16,
                            };
                            panel.show_velocity = false;
                            panel.dirty = true;
                        }
                    }
                    picked.then_some(())
                },
            );

            if let Some(lane) = lane.filter(|l| !l.events.is_empty()) {
                ui.add_space(2.0);
                let has_selection =
                    !panel.anchor_sel_rects.is_empty() || panel.anchor_members.is_some();
                // 有选中锚点时只改选中的；没有任何选中时改整条 lane。
                let ticks: Vec<u32> = if has_selection {
                    lane.events
                        .iter()
                        .filter(|e| panel.accepts_anchor(e))
                        .map(|e| e.tick)
                        .collect()
                } else {
                    lane.events.iter().map(|e| e.tick).collect()
                };
                let current = lane
                    .events
                    .iter()
                    .find(|e| panel.accepts_anchor(e))
                    .or_else(|| lane.events.first())
                    .map(|e| e.shape)
                    .unwrap_or_else(|| lane.target.default_shape());
                if let Some(shape) = combo_menu(
                    ui,
                    ICON_SHOW_CHART,
                    t!("hint.pr.anchor_shape").as_ref(),
                    "auto_shape_popup",
                    |ui| {
                        ui.set_min_width(crate::scaling::scaled_font(ui.ctx(), 210.0));
                        ui.label(if has_selection {
                            t!("automation.shape_selected", n = ticks.len()).to_string()
                        } else {
                            t!("automation.shape_all", n = ticks.len()).to_string()
                        });
                        ui.separator();
                        let mut chosen: Option<SegmentShape> = None;
                        if ui
                            .add(crate::widgets::menu::menu_item_button(
                                ui,
                                current == SegmentShape::Step,
                                t!("automation.step"),
                            ))
                            .clicked()
                        {
                            chosen = Some(SegmentShape::Step);
                        }
                        if ui
                            .add(crate::widgets::menu::menu_item_button(
                                ui,
                                current.is_linear(),
                                t!("automation.linear"),
                            ))
                            .clicked()
                        {
                            chosen = Some(SegmentShape::linear_curve());
                        }
                        ui.separator();
                        let mut sizes = vec![egui_extras::Size::exact(
                            crate::scaling::scaled_font(ui.ctx(), 52.0),
                        )];
                        sizes.extend(EASING_DIRECTIONS.iter().map(|_| {
                            egui_extras::Size::exact(crate::scaling::scaled_font(ui.ctx(), 44.0))
                        }));
                        for family in EASING_FAMILIES {
                            crate::widgets::grid::fixed_columns(ui, 2.0, 18.0, &sizes, |cells| {
                                cells.cell(|ui| {
                                    ui.label(family.label());
                                });
                                for direction in EASING_DIRECTIONS {
                                    cells.cell(|ui| {
                                        let Some(preset) = preset_of(family, direction) else {
                                            return;
                                        };
                                        if ui
                                            .add_sized(
                                                [ui.available_width().max(1.0), 18.0],
                                                egui::Button::new(direction.label())
                                                    .selected(current == preset.curve),
                                            )
                                            .clicked()
                                        {
                                            chosen = Some(preset.curve);
                                        }
                                    });
                                }
                            });
                        }
                        chosen
                    },
                ) {
                    applied = Some(ticks.iter().map(|&tick| (tick, shape)).collect());
                }
            }
            ui.add_space(4.0);
        });
    });
    applied
}

/// 在 24 个预设里按族/方向查一条（表很小，线性扫描即可）。
fn preset_of(family: EasingFamily, direction: EasingDirection) -> Option<&'static EasingPreset> {
    EASING_PRESETS
        .iter()
        .find(|p| p.family == family && p.direction == direction)
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
        let right_click_id = ui.scope_id().with("auto_right_click").with(i);
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
            let edit_tick_id = ui.scope_id().with("auto_right_tick").with(i);
            let edit_value_id = ui.scope_id().with("auto_right_value").with(i);
            let was_open_id = ui.scope_id().with("auto_right_was_open").with(i);
            ui.ctx().data_mut(|d| {
                d.remove::<interaction::RightClickAnchor>(right_click_id);
                d.remove::<f64>(edit_tick_id);
                d.remove::<f64>(edit_value_id);
                d.remove::<bool>(was_open_id);
            });
        }
    }
}

/// 面板左侧目标图标（combo 列）底部的关闭按钮：点谁删谁。
///
/// 位置在目标选择图标正下方（同一列），返回是否被点击
/// （调用方在遍历结束后从面板列表移除该索引）。
pub(crate) fn show_panel_close(ui: &mut egui::Ui, combo_rect: egui::Rect, index: usize) -> bool {
    const BTN: f32 = 18.0;
    let center = egui::pos2(combo_rect.center().x, combo_rect.max.y - BTN * 0.5 - 2.0);
    let btn_rect = egui::Rect::from_center_size(center, egui::vec2(BTN, BTN));
    let resp = ui.interact(
        btn_rect,
        ui.scope_id().with(("auto_panel_close", index)),
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
