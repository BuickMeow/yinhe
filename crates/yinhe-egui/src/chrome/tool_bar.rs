//! PR 顶部工具栏 + 钢琴/滚动条交界处的自动化宽按钮。
//!
//! 工具按钮横向常驻在钢琴卷帘顶部（取代原左侧竖条），点击即切换；
//! 悬停提示走底部讲解行。自动化开关/新增放在键盘列与底部滚动条交界的
//! 空白矩形里，做成两个宽按钮，避免再占用工具区。

use eframe::egui;
use egui_material_icons::icons::{ICON_ADD_CHART, ICON_BAR_CHART, ICON_MORE_HORIZ};
use rust_i18n::t;

use crate::widgets::tools_panel::{ALL_TOOLS, Tool};
use yinhe_types::{AutomationPanelView, PianoRollView};

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
) {
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
        let avail = (rect.width() - pad_x * 2.0).max(0.0);
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
            // ── 右侧视图开关：键数 / 调式折叠 ──
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(pad_x);
                let w = 40.0;
                let size = egui::vec2(w, btn_size.y);

                // 键数：88 → 128 → 256 → 88 循环。
                let label = view.layout.count.to_string();
                let resp = text_button(ui, &label, size, false, "pr_key_count");
                crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.pr.key_count"));
                if resp.clicked() {
                    let next = match view.layout.count {
                        88 => 128,
                        128 => 256,
                        _ => 88,
                    };
                    view.set_key_count(next);
                }

                // FOLD：按当前调号折叠。
                let fold_on = view.layout.fold_mask != 0;
                let enabled = fold_mask.is_some();
                let resp = text_button(ui, "FOLD", size, fold_on, "pr_fold");
                crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.pr.fold"));
                if resp.clicked() {
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
                if !enabled {
                    hovered_hint = Some(t!("hint.pr.fold_unavailable").to_string());
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

/// 纯文字小按钮（工具栏右侧的键数 / FOLD）。
fn text_button(
    ui: &mut egui::Ui,
    label: &str,
    size: egui::Vec2,
    active: bool,
    id_salt: &str,
) -> egui::Response {
    let color = if active {
        crate::theme::accent_active()
    } else {
        crate::theme::text_primary()
    };
    let sel_bg = active.then(crate::theme::selected_bg);
    ui.push_id(id_salt, |ui| {
        crate::widgets::flat::flat_button_filled_ghost(
            ui,
            egui::RichText::new(label)
                .size(crate::theme::PR_TOOL_BTN_FONT - 1.0)
                .color(color),
            size,
            sel_bg,
            true,
        )
    })
    .inner
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
