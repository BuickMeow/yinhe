//! PR 顶部工具栏 + 钢琴/滚动条交界处的自动化宽按钮。
//!
//! 工具按钮横向常驻在钢琴卷帘顶部（取代原左侧竖条），点击即切换；
//! 悬停提示走底部讲解行。自动化开关/新增放在键盘列与底部滚动条交界的
//! 空白矩形里，做成两个宽按钮，避免再占用工具区。

use eframe::egui;
use egui_material_icons::icons::{
    ICON_ADD_CHART, ICON_BAR_CHART, ICON_KEYBOARD_ARROW_DOWN, ICON_KEYBOARD_ARROW_RIGHT,
    ICON_MORE_HORIZ,
};
use rust_i18n::t;

use crate::widgets::tools_panel::{ALL_TOOLS, Tool};
use yinhe_types::{AutomationPanelView, Orientation, PianoRollView};

/// 顶部横向工具栏（由 PR 布局扣出的整条顶部带）。
///
/// 右侧常驻两项视图开关：调式折叠（FOLD）与键数（88/128/256）。
/// `fold_mask` = 当前调号对应的 12 位音级掩码；`None` = 无调式信息（折叠禁用）。
pub fn show(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    active_tool: &mut Tool,
    view: &mut PianoRollView,
    fold_mask: Option<u16>,
    scale_label: &str,
    clip_mode: &mut bool,
) -> bool {
    let mut open_scale_dialog = false;
    let btn_size = egui::vec2(
        crate::theme::PR_TOOL_BTN_SIZE,
        crate::theme::PR_TOOL_BTN_SIZE,
    );
    let mut hovered_hint: Option<String> = None;
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui.set_clip_rect(rect.intersect(ui.clip_rect()));
        ui.painter().rect_filled(rect, 0.0, crate::theme::app_bg());
        ui.spacing_mut().item_spacing.x = 2.0;
        // 横向放得下多少工具就显示多少，放不下的收进末尾「…」。
        let pad_x = 6.0;
        let spacing_x = 2.0;
        let slot = btn_size.x + spacing_x;
        // 右侧常驻开关（键数分段 + Scale + 调式名）预留宽度，避免窄窗口重叠。
        let right_reserve = 30.0 * 3.0 + 42.0 * 2.0 + 48.0 + 120.0 + spacing_x * 6.0 + pad_x;
        let avail = (rect.width() - pad_x * 2.0 - right_reserve).max(0.0);
        let fit = (((avail + spacing_x) / slot).floor() as usize).max(1);
        let n_visible = if fit >= ALL_TOOLS.len() {
            ALL_TOOLS.len()
        } else {
            fit.saturating_sub(1)
        };
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.add_space(pad_x);
            for tool in ALL_TOOLS.iter().take(n_visible) {
                if let Some(hint) = tool_button(ui, *tool, active_tool, btn_size) {
                    hovered_hint = Some(hint);
                }
            }
            if n_visible < ALL_TOOLS.len() {
                let more = tool_more_button(ui, btn_size);
                if more.hovered() {
                    hovered_hint = Some(t!("hint.tool_bar.more").to_string());
                }
                egui::Popup::from_toggle_button_response(&more)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
                    .show(|ui| {
                        ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                            ui.spacing_mut().item_spacing.y = spacing_x;
                            for tool in ALL_TOOLS.iter().skip(n_visible) {
                                tool_button(ui, *tool, active_tool, btn_size);
                            }
                        });
                    });
            }
            // ── 右侧视图开关：键数 / Scale / 调式选择 ──
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(pad_x);
                let h = 22.0;
                // [88|128|256]：分段开关（带背景，中缝为直角）。
                let counts = [88u16, 128, 256];
                let active = counts
                    .iter()
                    .position(|&c| c == view.layout.count)
                    .unwrap_or(1);
                if let Some(i) = segmented(
                    ui,
                    &["88", "128", "256"],
                    active,
                    30.0,
                    h,
                    12.0,
                    "pr_key_seg",
                ) {
                    view.set_key_count(counts[i]);
                }

                // [→|↓]：PR 方向双联按钮（取代 transport bar 的单按钮切换）。
                let orient_active = usize::from(view.is_vertical());
                if let Some(i) = segmented_icons(
                    ui,
                    &[ICON_KEYBOARD_ARROW_RIGHT, ICON_KEYBOARD_ARROW_DOWN],
                    orient_active,
                    26.0,
                    h,
                    "pr_orientation",
                ) {
                    view.set_orientation(if i == 0 {
                        Orientation::Horizontal
                    } else {
                        Orientation::Vertical
                    });
                }

                // [CLIP|SONG]：分段开关，小字号，带背景。
                let scope_active = usize::from(!*clip_mode);
                if let Some(i) = segmented(
                    ui,
                    &["CLIP", "SONG"],
                    scope_active,
                    42.0,
                    h,
                    10.5,
                    "pr_scope_seg",
                ) {
                    *clip_mode = i == 0;
                }

                // Scale 开关（单段胶囊，矮按钮）。
                let fold_on = view.layout.fold_mask != 0;
                let scale_active = usize::from(fold_on);
                if segmented(ui, &["Scale"], scale_active, 48.0, h, 11.0, "pr_scale").is_some() {
                    if fold_on {
                        view.set_fold(0);
                    } else if let Some(mask) = fold_mask {
                        view.set_fold(mask);
                    }
                } else if fold_on
                    && let Some(mask) = fold_mask
                    && view.layout.fold_mask != mask
                {
                    // 调号变化时跟随（覆盖事件/手动调式都生效）。
                    view.set_fold(mask);
                }

                // 调式（根音 + 音阶）显示按钮（单段胶囊，矮按钮），点击打开音阶对话框。
                let scale_w = (scale_label.chars().count() as f32 * 6.5 + 16.0).clamp(60.0, 120.0);
                if segmented(ui, &[scale_label], 0, scale_w, h, 11.0, "pr_scale_select").is_some() {
                    open_scale_dialog = true;
                }
            });
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
    open_scale_dialog
}

/// 钢琴/滚动条交界处的两个宽按钮：左 = 显示自动化，右 = 新增自动化。
///
/// `rect` 为滚动条左侧空白区（键盘列宽 × 滚动条高）。
pub fn show_auto_buttons(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    show_panels: &mut bool,
    panels: &mut Vec<AutomationPanelView>,
) {
    if rect.width() < 16.0 || rect.height() < 8.0 {
        return;
    }
    // 外框区域只铺背景；两个按钮矩形内缩 2px，留出 padding。
    let pad = 2.0;
    ui.painter().rect_filled(rect, 0.0, crate::theme::app_bg());
    let gap = 2.0;
    let inner = egui::Rect::from_min_max(
        egui::pos2(rect.min.x + pad, rect.min.y + pad),
        egui::pos2(rect.max.x - pad, rect.max.y - pad),
    );
    let w = ((inner.width() - gap) * 0.5).max(1.0);
    let toggle_rect = egui::Rect::from_min_size(inner.min, egui::vec2(w, inner.height()));
    let add_rect =
        egui::Rect::from_min_max(egui::pos2(inner.min.x + w + gap, inner.min.y), inner.max);

    let toggle = icon_button_at(
        ui,
        toggle_rect.center(),
        toggle_rect.size(),
        "pr_auto_toggle",
        ICON_BAR_CHART,
        *show_panels,
    );
    crate::widgets::hint::hover(ui.ctx(), &toggle, t!("hint.pr.auto_toggle"));
    if toggle.clicked() {
        *show_panels = !*show_panels;
        if *show_panels && panels.is_empty() {
            panels.push(AutomationPanelView::default());
        }
    }

    let add = icon_button_at(
        ui,
        add_rect.center(),
        add_rect.size(),
        "pr_auto_add",
        ICON_ADD_CHART,
        false,
    );
    crate::widgets::hint::hover(ui.ctx(), &add, t!("hint.pr.auto_add"));
    if add.clicked() {
        *show_panels = true;
        panels.push(AutomationPanelView::default());
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
                    .size(crate::theme::PR_TOOL_BTN_FONT)
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
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.hovered()
        .then(|| crate::chrome::transport_bar_actions::tool_hint(tool))
}

/// 溢出「…」按钮：点击弹出子悬浮栏放剩余工具。
fn tool_more_button(ui: &mut egui::Ui, btn_size: egui::Vec2) -> egui::Response {
    ui.push_id("tool_bar_more", |ui| {
        crate::widgets::flat::flat_button_filled_ghost(
            ui,
            ICON_MORE_HORIZ
                .rich_text()
                .size(crate::theme::PR_TOOL_BTN_FONT)
                .color(crate::theme::text_primary()),
            btn_size,
            None,
            true,
        )
    })
    .inner
}

/// 双联图标分段开关（方向切换）。
#[allow(clippy::too_many_arguments)]
fn segmented_icons(
    ui: &mut egui::Ui,
    icons: &[egui_material_icons::MaterialIcon],
    active: usize,
    seg_w: f32,
    h: f32,
    id_salt: &str,
) -> Option<usize> {
    let mut clicked = None;
    ui.push_id(id_salt, |ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.horizontal(|ui| {
            for (i, icon) in icons.iter().enumerate() {
                let radius = egui::CornerRadius {
                    nw: if i == 0 { 3 } else { 0 },
                    sw: if i == 0 { 3 } else { 0 },
                    ne: if i + 1 == icons.len() { 3 } else { 0 },
                    se: if i + 1 == icons.len() { 3 } else { 0 },
                };
                let resp = ui.add(
                    egui::Button::new(icon.rich_text().size(14.0).color(if i == active {
                        crate::theme::accent_active()
                    } else {
                        crate::theme::text_primary()
                    }))
                    .selected(i == active)
                    .min_size(egui::vec2(seg_w, h))
                    .corner_radius(radius),
                );
                if resp.hovered() {
                    resp.clone().on_hover_cursor(egui::CursorIcon::PointingHand);
                }
                if resp.clicked() {
                    clicked = Some(i);
                }
            }
        });
    });
    clicked
}

/// 分段开关：整组带背景，选中段填 selected_bg；段间竖线为直角，不随圆角弯曲。
///
/// `labels` 从左到右排列；返回被点击的段下标。
#[allow(clippy::too_many_arguments)]
fn segmented(
    ui: &mut egui::Ui,
    labels: &[&str],
    active: usize,
    seg_w: f32,
    h: f32,
    font_size: f32,
    id_salt: &str,
) -> Option<usize> {
    let mut clicked = None;
    ui.push_id(id_salt, |ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.horizontal(|ui| {
            for (i, label) in labels.iter().enumerate() {
                let radius = egui::CornerRadius {
                    nw: if i == 0 { 3 } else { 0 },
                    sw: if i == 0 { 3 } else { 0 },
                    ne: if i + 1 == labels.len() { 3 } else { 0 },
                    se: if i + 1 == labels.len() { 3 } else { 0 },
                };
                let resp = ui.add(
                    egui::Button::new(egui::RichText::new(*label).size(font_size).color(
                        if i == active {
                            crate::theme::accent_active()
                        } else {
                            crate::theme::text_primary()
                        },
                    ))
                    .selected(i == active)
                    .min_size(egui::vec2(seg_w, h))
                    .corner_radius(radius),
                );
                if resp.hovered() {
                    resp.clone().on_hover_cursor(egui::CursorIcon::PointingHand);
                }
                if resp.clicked() {
                    clicked = Some(i);
                }
            }
        });
    });
    clicked
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
                    .size(crate::theme::PR_TOOL_BTN_FONT)
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
