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

/// 右侧分段开关的字体与最小段宽。
/// 这些量同时用于绘制与宽度预留，避免两处数字漂移导致按钮溢出右缘。
const KEY_SEG_MIN_W: f32 = 30.0;
const KEY_SEG_FONT: f32 = 12.0;
const ORIENT_SEG_MIN_W: f32 = 26.0;
const ORIENT_ICON_FONT: f32 = 14.0;
const SCOPE_SEG_MIN_W: f32 = 42.0;
const SCOPE_SEG_FONT: f32 = 10.5;
const SCALE_SEG_MIN_W: f32 = 48.0;
const SCALE_SEG_FONT: f32 = 11.0;
const SCALE_LABEL_MIN_W: f32 = 60.0;

/// 顶部横向工具栏（由 PR 布局扣出的整条顶部带）。
///
/// 右侧常驻两项视图开关：调式折叠（FOLD）与键数（88/128/256）。
/// `fold_mask` = 当前调号对应的 12 位音级掩码；`None` = 无调式信息（折叠禁用）。
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
pub fn show(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    active_tool: &mut Tool,
    view: &mut PianoRollView,
    fold_mask: Option<u16>,
    scale_label: &str,
    clip_mode: &mut bool,
    clip_enabled: bool,
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
        // 右侧常驻开关（键数 / 方向 / CLIP·SONG / Scale / 调式名）按实际字形宽度预留，
        // 避免窄窗口下工具按钮与右侧分组重叠。
        let right_reserve = right_group_width(ui, scale_label, clip_enabled, spacing_x, pad_x);
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
                    Some(active),
                    KEY_SEG_MIN_W,
                    h,
                    KEY_SEG_FONT,
                    "pr_key_seg",
                ) {
                    view.set_key_count(counts[i]);
                }

                // [→|↓]：卷帘方向双联按钮（Material Icons，双段并列）。
                let orient_active = usize::from(view.is_vertical());
                if let Some(i) = segmented_icons(
                    ui,
                    &[ICON_KEYBOARD_ARROW_RIGHT, ICON_KEYBOARD_ARROW_DOWN],
                    Some(orient_active),
                    ORIENT_SEG_MIN_W,
                    h,
                    ORIENT_ICON_FONT,
                    "pr_orientation",
                ) {
                    view.set_orientation(if i == 0 {
                        Orientation::Horizontal
                    } else {
                        Orientation::Vertical
                    });
                }

                // [CLIP|SONG]：分段开关，小字号，带背景。总开关关闭时只保留 SONG。
                if clip_enabled {
                    let scope_active = usize::from(!*clip_mode);
                    if let Some(i) = segmented(
                        ui,
                        &["CLIP", "SONG"],
                        Some(scope_active),
                        SCOPE_SEG_MIN_W,
                        h,
                        SCOPE_SEG_FONT,
                        "pr_scope_seg",
                    ) {
                        *clip_mode = i == 0;
                    }
                } else {
                    *clip_mode = false;
                }

                // Scale 开关（单段胶囊，矮按钮）。
                let fold_on = view.layout.fold_mask != 0;
                if segmented(
                    ui,
                    &["Scale"],
                    fold_on.then_some(0),
                    SCALE_SEG_MIN_W,
                    h,
                    SCALE_SEG_FONT,
                    "pr_scale",
                )
                .is_some()
                {
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
                if segmented(
                    ui,
                    &[scale_label],
                    Some(0),
                    SCALE_LABEL_MIN_W,
                    h,
                    SCALE_SEG_FONT,
                    "pr_scale_select",
                )
                .is_some()
                {
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
    let mut toggle_requested = false;
    let mut add_requested = false;
    ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
        crate::widgets::grid::weighted_columns(ui, gap, &[1.0, 1.0], |cells| {
            cells.cell(|ui| {
                let cell = ui.max_rect();
                let toggle = icon_button_at(
                    ui,
                    cell.center(),
                    cell.size(),
                    "pr_auto_toggle",
                    ICON_BAR_CHART,
                    *show_panels,
                );
                crate::widgets::hint::hover(ui.ctx(), &toggle, t!("hint.pr.auto_toggle"));
                if toggle.clicked() {
                    toggle_requested = true;
                }
            });
            cells.cell(|ui| {
                let cell = ui.max_rect();
                let add = icon_button_at(
                    ui,
                    cell.center(),
                    cell.size(),
                    "pr_auto_add",
                    ICON_ADD_CHART,
                    false,
                );
                crate::widgets::hint::hover(ui.ctx(), &add, t!("hint.pr.auto_add"));
                if add.clicked() {
                    add_requested = true;
                }
            });
        });
    });
    if toggle_requested {
        *show_panels = !*show_panels;
        if *show_panels && panels.is_empty() {
            panels.push(AutomationPanelView::default());
        }
    }
    if add_requested {
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

/// 右侧整组（键数 / 方向 / CLIP·SONG / Scale / 调式名）占用的宽度。
///
/// 与绘制共用同一套实测宽度，工具区据此计算还能放几个按钮，避免两组重叠。
fn right_group_width(
    ui: &egui::Ui,
    scale_label: &str,
    clip_enabled: bool,
    spacing_x: f32,
    pad_x: f32,
) -> f32 {
    let key_w = segmented_labels_width(ui, &["88", "128", "256"], KEY_SEG_FONT, KEY_SEG_MIN_W);
    let orient_w = segmented_icons_width(
        ui,
        &[ICON_KEYBOARD_ARROW_RIGHT, ICON_KEYBOARD_ARROW_DOWN],
        ORIENT_ICON_FONT,
        ORIENT_SEG_MIN_W,
    );
    let scope_w = if clip_enabled {
        segmented_labels_width(ui, &["CLIP", "SONG"], SCOPE_SEG_FONT, SCOPE_SEG_MIN_W)
    } else {
        0.0
    };
    let scale_w = segmented_labels_width(ui, &["Scale"], SCALE_SEG_FONT, SCALE_SEG_MIN_W);
    let label_w = segment_label_width(ui, scale_label, SCALE_SEG_FONT, SCALE_LABEL_MIN_W);
    // 组内间隔数 = 项数 - 1，额外留一格作为工具区与右侧分组之间的保底间距。
    let items = 3.0 + if clip_enabled { 1.0 } else { 0.0 };
    key_w + orient_w + scope_w + scale_w + label_w + spacing_x * (items + 1.0) + pad_x
}

/// 实测一段文字所需宽度（含按钮内边距，向上取整并留 1px 余量）。
fn segment_label_width(ui: &egui::Ui, label: &str, font_size: f32, min_w: f32) -> f32 {
    let text_w = ui.fonts_mut(|f| {
        f.layout_no_wrap(
            label.to_owned(),
            egui::FontId::proportional(font_size),
            egui::Color32::PLACEHOLDER,
        )
        .size()
        .x
    });
    (text_w + ui.spacing().button_padding.x * 2.0)
        .max(min_w)
        .ceil()
        + 1.0
}

/// 实测一个 Material 图标段所需宽度（口径同 [`segment_label_width`]）。
fn segment_icon_width(
    ui: &egui::Ui,
    icon: egui_material_icons::MaterialIcon,
    font_size: f32,
    min_w: f32,
) -> f32 {
    let text_w = ui.fonts_mut(|f| {
        f.layout_no_wrap(
            icon.codepoint.to_string(),
            egui::FontId::new(font_size, icon.font_family()),
            egui::Color32::PLACEHOLDER,
        )
        .size()
        .x
    });
    (text_w + ui.spacing().button_padding.x * 2.0)
        .max(min_w)
        .ceil()
        + 1.0
}

fn segmented_labels_width(ui: &egui::Ui, labels: &[&str], font_size: f32, min_w: f32) -> f32 {
    labels
        .iter()
        .map(|label| segment_label_width(ui, label, font_size, min_w))
        .sum()
}

fn segmented_icons_width(
    ui: &egui::Ui,
    icons: &[egui_material_icons::MaterialIcon],
    font_size: f32,
    min_w: f32,
) -> f32 {
    icons
        .iter()
        .map(|&icon| segment_icon_width(ui, icon, font_size, min_w))
        .sum()
}

/// 分段开关：整组带背景，选中段填 selected_bg；段间竖线为直角，不随圆角弯曲。
///
/// 每段宽度按实际字形测量后再分配：字体度量偏大时把整组撑到右缘外，
/// 曾被裁掉最后一个字（如「256」）。
///
/// `labels` 从左到右排列；返回被点击的段下标。
#[allow(clippy::too_many_arguments)]
fn segmented(
    ui: &mut egui::Ui,
    labels: &[&str],
    active: Option<usize>,
    seg_w: f32,
    h: f32,
    font_size: f32,
    id_salt: &str,
) -> Option<usize> {
    let segments: Vec<(egui::WidgetText, f32)> = labels
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let color = if Some(i) == active {
                crate::theme::accent_active()
            } else {
                crate::theme::text_primary()
            };
            (
                egui::RichText::new(*label)
                    .size(font_size)
                    .color(color)
                    .into(),
                segment_label_width(ui, label, font_size, seg_w),
            )
        })
        .collect();
    segmented_widgets(ui, &segments, active, h, id_salt, &[])
}

/// 双联图标分段开关（Material Icons，用于横向/纵向切换）。
///
/// 宽度分配与 [`segmented`] 一致：先实测图标宽度，再按实测总宽分配，
/// 保证整组不会越过工具栏右缘。
#[allow(clippy::too_many_arguments)]
fn segmented_icons(
    ui: &mut egui::Ui,
    icons: &[egui_material_icons::MaterialIcon],
    active: Option<usize>,
    seg_w: f32,
    h: f32,
    font_size: f32,
    id_salt: &str,
) -> Option<usize> {
    let segments: Vec<(egui::WidgetText, f32)> = icons
        .iter()
        .enumerate()
        .map(|(i, &icon)| {
            let color = if Some(i) == active {
                crate::theme::accent_active()
            } else {
                crate::theme::text_primary()
            };
            (
                icon.rich_text().size(font_size).color(color).into(),
                segment_icon_width(ui, icon, font_size, seg_w),
            )
        })
        .collect();
    let hints = [
        t!("menu.orientation.horizontal").to_string(),
        t!("menu.orientation.vertical").to_string(),
    ];
    segmented_widgets(ui, &segments, active, h, id_salt, &hints)
}

/// 分段开关的公共绘制：`segments` = (每段内容, 实测段宽)，`hints` 为逐段悬停提示。
fn segmented_widgets(
    ui: &mut egui::Ui,
    segments: &[(egui::WidgetText, f32)],
    active: Option<usize>,
    h: f32,
    id_salt: &str,
    hints: &[String],
) -> Option<usize> {
    let mut clicked = None;
    ui.push_id(id_salt, |ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let width: f32 = segments.iter().map(|(_, w)| *w).sum();
        ui.allocate_ui_with_layout(
            egui::vec2(width, h),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                for (i, (text, w)) in segments.iter().enumerate() {
                    let radius = egui::CornerRadius {
                        nw: if i == 0 { 3 } else { 0 },
                        sw: if i == 0 { 3 } else { 0 },
                        ne: if i + 1 == segments.len() { 3 } else { 0 },
                        se: if i + 1 == segments.len() { 3 } else { 0 },
                    };
                    let resp = ui.add(
                        egui::Button::new(text.clone())
                            .selected(Some(i) == active)
                            .min_size(egui::vec2(*w, h))
                            .corner_radius(radius),
                    );
                    if resp.hovered() {
                        let resp = resp.clone().on_hover_cursor(egui::CursorIcon::PointingHand);
                        if let Some(hint) = hints.get(i) {
                            resp.on_hover_text(hint);
                        }
                    }
                    if resp.clicked() {
                        clicked = Some(i);
                    }
                }
            },
        );
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

#[cfg(test)]
mod tests {
    use super::*;

    fn text_rects(ctx: &egui::Context, width: f32) -> Vec<(String, egui::Rect)> {
        let rect = egui::Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(width, crate::theme::PR_TOOL_BAR_H),
        );
        let mut view = PianoRollView::default();
        let mut tool = Tool::Select;
        let mut clip_mode = false;
        let output = ctx.run_ui(Default::default(), |ui| {
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                show(
                    ui,
                    rect,
                    &mut tool,
                    &mut view,
                    Some(0xFFF),
                    "C Major",
                    &mut clip_mode,
                    true,
                );
            });
        });
        let mut out = Vec::new();
        for cs in &output.shapes {
            if let egui::Shape::Text(t) = &cs.shape {
                out.push((
                    t.galley.text().to_owned(),
                    egui::Rect::from_min_size(t.pos, t.galley.size()),
                ));
            }
        }
        output.drop_without_applying_deltas();
        out
    }

    /// 回归：右侧文字分段组不得互相压住，且必须完整落在工具栏内。
    #[test]
    fn right_text_groups_stay_separated() {
        let ctx = egui::Context::default();
        crate::fonts::install(&ctx, 400, "zh-CN");
        ctx.run_ui(Default::default(), |_| {})
            .drop_without_applying_deltas();
        for width in [1600.0, 1400.0, 1200.0, 1000.0, 900.0] {
            let rects = text_rects(&ctx, width);

            let find = |needle: &str| {
                rects
                    .iter()
                    .find(|(t, _)| t == needle)
                    .map(|(_, r)| *r)
                    .unwrap_or_else(|| panic!("缺少文字 {needle}: {rects:?}"))
            };
            let a = find("88");
            let b = find("128");
            let c = find("256");
            let scale = find("C Major");
            assert!(
                a.max.x <= b.min.x + 0.5,
                "88/128 重叠 (width={width}): {a:?} {b:?}"
            );
            assert!(
                b.max.x <= c.min.x + 0.5,
                "128/256 重叠 (width={width}): {b:?} {c:?}"
            );
            assert!(
                scale.max.x <= a.min.x + 0.5,
                "Scale 与键数分段重叠 (width={width}): {scale:?} {a:?}"
            );
            for r in [a, b, c, scale] {
                assert!(
                    r.min.x >= 0.0 && r.max.x <= width,
                    "文字超出工具栏 (width={width}): {r:?}"
                );
            }
            for (text, r) in &rects {
                assert!(
                    r.min.x >= 0.0 && r.max.x <= width,
                    "「{text}」超出工具栏 (width={width}): {r:?}"
                );
            }
        }
    }
}
