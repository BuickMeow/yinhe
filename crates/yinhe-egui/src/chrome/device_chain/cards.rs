//! 设备链卡片：乐器卡片、效果器卡片、标题栏与竖排标题。

use eframe::egui;
use rust_i18n::t;
use yinhe_types::automation::{CHANNEL_DSP_PARAMS, XSYNTH_PARAMS};

use super::knobs::param_knobs;
use super::*;

/// 音源面板的自动化目标（通道处理参数在前、XSynth 音源参数在后）。
///
/// 通道处理（Volume/Expression/Pan/Cutoff/Resonance）已内置到音源的通道
/// 处理段，与 XSynth 的 CC 绑定参数一样走低层 `CC lane`（回放广播给通道
/// 处理段/合成器）；PB/RPN 无低层 lane 形态，保留设备参数
/// （`ChannelInstrument`）。返回 (target, 显示名, 默认值, 是否居中归零)，
/// 显示名取参数表自己的名字（如 "Sustain"），不用 CC 目标的通用显示名
/// （"CC 064 Sustain"）。居中归零来自参数表的 `center` 标记（Pan 等）。
pub(super) fn instrument_targets(channel: u8) -> Vec<(AutomationTarget, String, f32, bool)> {
    CHANNEL_DSP_PARAMS
        .iter()
        .chain(XSYNTH_PARAMS)
        .map(|p| {
            (
                crate::piano_view::automation_panel::builtin_target(p, channel),
                p.name.to_string(),
                // 默认值取参数表（引擎无事件时的实际值，如 Volume/Expression
                // 满增益 1.0）；不能用 CC 目标的通用默认值（CC7 会得 0）。
                p.default,
                p.center,
            )
        })
        .collect()
}

/// 无边框 Material 图标按钮（尺寸/颜色/提示可调）。
pub(super) fn icon_button(
    ui: &mut egui::Ui,
    icon: egui_material_icons::MaterialIcon,
    size: f32,
    color: egui::Color32,
    hover: impl Into<egui::WidgetText>,
) -> egui::Response {
    let resp = ui.add(
        egui::Button::new(
            egui::RichText::new(icon.codepoint)
                .font(egui::FontId::new(size, icon.font_family()))
                .color(color),
        )
        .frame(false),
    );
    if resp.hovered() {
        crate::widgets::hint::set(ui.ctx(), hover.into().text().to_owned());
    }
    resp
}

/// 收起态卡片顶部电源键：与展开态标题行同一个开关。
/// 返回（是否点击，标题文字可用的剩余矩形）。
pub(super) fn collapsed_power(
    ui: &mut egui::Ui,
    title_rect: egui::Rect,
    powered: bool,
    hint: String,
) -> (bool, egui::Rect) {
    let color = if powered {
        crate::theme::accent_active()
    } else {
        crate::theme::text_muted()
    };
    let btn_rect = egui::Rect::from_min_size(
        title_rect.min,
        egui::vec2(title_rect.width(), COLLAPSED_CARD_W),
    );
    let mut clicked = false;
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(btn_rect)
            .layout(egui::Layout::top_down(egui::Align::Center)),
        |ui| {
            // 直接画在整块槽的中心：不经过 Button 的 padding，保证和展开态
            // 标题行的电源键位置一致。
            let (rect, resp) = ui.allocate_exact_size(ui.available_size(), egui::Sense::click());
            let icon = egui_material_icons::icons::ICON_POWER_SETTINGS_NEW;
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                icon.codepoint,
                egui::FontId::new(14.0, icon.font_family()),
                color,
            );
            crate::widgets::hint::hover(ui.ctx(), &resp, hint);
            clicked = resp.clicked();
        },
    );
    let text_rect =
        egui::Rect::from_min_max(egui::pos2(title_rect.min.x, btn_rect.max.y), title_rect.max);
    (clicked, text_rect)
}

/// 效果器大卡片：插件效果器显示参数面板 / 原生界面入口。
/// 返回 true = 卡片被点击（选中高亮）。
#[allow(clippy::too_many_arguments)] // 设备链绘制上下文透传
pub(super) fn effect_card(
    ui: &mut egui::Ui,
    slot: usize,
    ins: &DockInsert,
    selected: Option<DockDevice>,
    open_params: &mut Option<DockDevice>,
    toggle_bypass: &mut Option<(usize, bool)>,
    open_gui: &mut Option<DockDevice>,
    remove_insert: &mut Option<usize>,
    collapsed: &mut std::collections::HashSet<usize>,
) -> bool {
    let is_selected = selected == Some(DockDevice::Insert(slot));
    let is_collapsed = collapsed.contains(&slot);
    let mut clicked = false;

    let rect = ui.max_rect();
    // 必须占满整块：否则父布局只按标题内容宽度推进，下一张卡片/「+」会错位。
    ui.allocate_rect(rect, egui::Sense::hover());
    let painter = ui.painter_at(rect);
    paint_card_frame(&painter, rect);

    let title_rect = egui::Rect::from_min_size(
        rect.min,
        egui::vec2(
            rect.width(),
            if is_collapsed {
                rect.height()
            } else {
                CARD_TITLE_H
            },
        ),
    );
    paint_card_title(&painter, title_rect, is_selected, is_collapsed);
    let title_fg = crate::theme::contrast_text(crate::theme::selected_bg());

    let title_resp = ui.interact(
        title_rect,
        ui.scope_id().with(("dock_card_title", slot)),
        egui::Sense::click(),
    );
    if title_resp.double_clicked() {
        if !collapsed.remove(&slot) {
            collapsed.insert(slot);
        }
    } else if title_resp.clicked() {
        clicked = true;
    }

    if is_collapsed {
        let (power_clicked, text_rect) = collapsed_power(
            ui,
            title_rect,
            !ins.bypassed,
            t!("hint.dock.insert_power").to_string(),
        );
        if power_clicked {
            *toggle_bypass = Some((slot, !ins.bypassed));
        }
        paint_vertical_title(ui, text_rect, &ins.name, title_fg);
        return clicked;
    }

    // 标题行内容（贴边，左右 6px 内缩）。
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(title_rect.shrink2(egui::vec2(6.0, 2.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            let power_color = if ins.bypassed {
                crate::theme::text_muted()
            } else {
                crate::theme::accent_active()
            };
            if icon_button(
                ui,
                egui_material_icons::icons::ICON_POWER_SETTINGS_NEW,
                14.0,
                power_color,
                t!("hint.dock.insert_power"),
            )
            .clicked()
            {
                *toggle_bypass = Some((slot, !ins.bypassed));
            }

            let name_resp = ui.add(
                egui::Label::new(
                    egui::RichText::new(&ins.name)
                        .size(crate::theme::SMALL_FONT + 2.0)
                        .color(crate::theme::text_primary()),
                )
                .sense(egui::Sense::click()),
            );
            crate::widgets::hint::hover(ui.ctx(), &name_resp, t!("hint.dock.insert_select"));
            if name_resp.clicked() {
                clicked = true;
            }

            if icon_button(
                ui,
                egui_material_icons::icons::ICON_TUNE,
                13.0,
                crate::theme::text_secondary(),
                t!("dock.open_params"),
            )
            .clicked()
            {
                *open_params = Some(DockDevice::Insert(slot));
            }

            if icon_button(
                ui,
                egui_material_icons::icons::ICON_HOME_STORAGE,
                13.0,
                crate::theme::text_secondary(),
                t!("mix.toggle_gui"),
            )
            .clicked()
            {
                *open_gui = Some(DockDevice::Insert(slot));
            }

            if icon_button(
                ui,
                egui_material_icons::icons::ICON_DELETE,
                13.0,
                crate::theme::text_secondary(),
                t!("mix.remove_insert"),
            )
            .clicked()
            {
                *remove_insert = Some(slot);
            }
        },
    );

    let content = card_content_rect(rect, title_rect);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(content)
            .layout(egui::Layout::top_down(egui::Align::LEFT)),
    );
    child.set_clip_rect(rect);
    // ── 内容：参数面板入口 + 提示 ──
    if crate::widgets::flat::flat_button(&mut child, t!("dock.open_params")).clicked() {
        *open_params = Some(DockDevice::Insert(slot));
    }
    child.add_space(4.0);
    child.label(
        egui::RichText::new(t!("dock.plugin_hint"))
            .size(crate::theme::SMALL_FONT)
            .color(crate::theme::text_muted()),
    );

    clicked
}

/// 卡片底：圆角背景（选中态改由标题栏深浅表示，不再画描边圈）。
fn paint_card_frame(painter: &egui::Painter, rect: egui::Rect) {
    painter.rect_filled(rect, 4.0, crate::theme::track_bg());
}

/// 标题栏：贴卡片上缘与左右边，只有上方两角圆角，下部直角。
/// 普通卡片用很轻的强调底色，选中卡片加深一档。
fn paint_card_title(painter: &egui::Painter, rect: egui::Rect, selected: bool, collapsed: bool) {
    let fill = if selected {
        crate::theme::selected_bg()
    } else {
        crate::theme::control_selected_bg()
    };
    // 展开态只有上方两角圆角（下缘接内容区）；收起态整张卡片就是标题条，
    // 四角都圆。
    let radius = if collapsed {
        egui::CornerRadius::same(4)
    } else {
        egui::CornerRadius {
            nw: 4,
            ne: 4,
            sw: 0,
            se: 0,
        }
    };
    painter.rect_filled(rect, radius, fill);
}

/// 卡片内容区（标题下方、左右/底部内缩 10/8px）。
fn card_content_rect(card: egui::Rect, title: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_max(
        egui::pos2(card.min.x + 10.0, title.max.y + 6.0),
        egui::pos2(card.max.x - 10.0, card.max.y - 8.0),
    )
}

/// 收起态标题：沿卡片长边竖排（旋转 -90°），字号与展开态标题一致。
fn paint_vertical_title(ui: &egui::Ui, rect: egui::Rect, text: &str, color: egui::Color32) {
    let font = egui::FontId::proportional(crate::theme::SMALL_FONT + 2.0);
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, color);
    ui.painter().add(vertical_title_shape(rect, galley, color));
}

/// 竖排标题的 TextShape：旋转后可视中心严格落在 `rect.center()`。
///
/// `TextShape::new` 的 pos 是未旋转文字的左上角；`with_angle_and_anchor`
/// 保持锚点在旋转前的屏幕位置，所以先取「未旋转居中时的左上角」。
pub(super) fn vertical_title_shape(
    rect: egui::Rect,
    galley: std::sync::Arc<egui::Galley>,
    color: egui::Color32,
) -> egui::epaint::TextShape {
    let top_left = rect.center() - galley.rect.center().to_vec2();
    egui::epaint::TextShape::new(top_left, galley, color)
        .with_angle_and_anchor(-std::f32::consts::FRAC_PI_2, egui::Align2::CENTER_CENTER)
}

/// 「+」添加效果器竖条（高度撑满主体区）。
pub(super) fn add_column(ui: &mut egui::Ui, height: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(44.0, height), egui::Sense::click());
    let bg = if resp.hovered() {
        crate::theme::hover_color(crate::theme::track_bg())
    } else {
        crate::theme::btn_bg().gamma_multiply(0.6)
    };
    let painter = ui.painter();
    painter.rect_filled(rect, 4.0, bg);
    painter.rect_stroke(
        rect,
        4.0,
        egui::Stroke::new(1.0, crate::theme::grid_sub_beat()),
        egui::StrokeKind::Inside,
    );
    let add = egui_material_icons::icons::ICON_ADD;
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        add.codepoint,
        egui::FontId::new(16.0, add.font_family()),
        crate::theme::text_muted(),
    );
    crate::widgets::hint::hover(ui.ctx(), &resp, t!("mix.add_insert_hint"));
    resp
}

/// 乐器大卡片（永远在最左）：XSynth 旋钮列表 / 插件参数入口；
/// 音频语境显示通道信息。参数拖动写 lane，与效果器卡片同一套机制。
#[allow(clippy::too_many_arguments)] // dock 卡片上下文透传，见 AGENTS 约定
pub(super) fn instrument_card(
    ui: &mut egui::Ui,
    context: DockContext,
    instrument_plugin: Option<&str>,
    lane_current: &[DockParam],
    search: &mut String,
    inst_powered: bool,
    knob_actions: &mut Vec<KnobAction>,
    open_params: &mut Option<DockDevice>,
    toggle_instrument: &mut Option<bool>,
    open_gui: &mut Option<DockDevice>,
    open_instrument_picker: &mut Option<u8>,
    open_synth_params: &mut Option<u8>,
    midi_channel: Option<u8>,
    collapsed: &mut std::collections::HashSet<usize>,
    is_selected: bool,
) -> bool {
    let has_instrument = midi_channel.is_some();
    let use_xsynth = has_instrument && instrument_plugin.is_none();
    let is_collapsed = collapsed.contains(&INSTRUMENT_KEY);
    let mut clicked = false;
    let title_text = if has_instrument {
        instrument_plugin
            .map(str::to_string)
            .unwrap_or_else(|| "XSynth".to_string())
    } else {
        context_label(context)
    };

    let rect = ui.max_rect();
    // 必须占满整块：否则父布局只按标题内容宽度推进，下一张卡片/「+」会错位。
    ui.allocate_rect(rect, egui::Sense::hover());
    let painter = ui.painter_at(rect);
    paint_card_frame(&painter, rect);

    let title_rect = egui::Rect::from_min_size(
        rect.min,
        egui::vec2(
            rect.width(),
            if is_collapsed {
                rect.height()
            } else {
                CARD_TITLE_H
            },
        ),
    );
    paint_card_title(&painter, title_rect, is_selected, is_collapsed);
    let title_fg = crate::theme::contrast_text(crate::theme::selected_bg());

    let title_resp = ui.interact(
        title_rect,
        ui.scope_id().with("dock_instrument_title"),
        egui::Sense::click(),
    );
    if title_resp.double_clicked() {
        if !collapsed.remove(&INSTRUMENT_KEY) {
            collapsed.insert(INSTRUMENT_KEY);
        }
    } else if title_resp.clicked() {
        clicked = true;
    }

    if is_collapsed {
        let text_rect = if has_instrument {
            let (power_clicked, text_rect) = collapsed_power(
                ui,
                title_rect,
                inst_powered,
                t!("hint.dock.instrument_power").to_string(),
            );
            if power_clicked {
                *toggle_instrument = Some(inst_powered);
            }
            text_rect
        } else {
            title_rect
        };
        paint_vertical_title(ui, text_rect, &title_text, title_fg);
        return clicked;
    }

    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(title_rect.shrink2(egui::vec2(6.0, 2.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            if !has_instrument {
                ui.label(
                    egui::RichText::new(context_label(context))
                        .size(crate::theme::SMALL_FONT + 2.0)
                        .color(crate::theme::text_primary()),
                );
                return;
            }

            let power_color = if inst_powered {
                crate::theme::accent_active()
            } else {
                crate::theme::text_muted()
            };
            if icon_button(
                ui,
                egui_material_icons::icons::ICON_POWER_SETTINGS_NEW,
                14.0,
                power_color,
                t!("hint.dock.instrument_power"),
            )
            .clicked()
            {
                // 目标 muted 值 = 当前是否开着（true→全 mute，false→全恢复）。
                *toggle_instrument = Some(inst_powered);
            }

            let name_resp = ui.label(
                egui::RichText::new(&title_text)
                    .size(crate::theme::SMALL_FONT + 2.0)
                    .color(crate::theme::text_primary()),
            );
            crate::widgets::hint::hover(ui.ctx(), &name_resp, t!("hint.dock.instrument_name"));

            // 搜索（仅内置 XSynth 的旋钮参数列表）：默认只显示图标按钮，
            // 点击才展开输入框；失焦且为空自动收起。
            if use_xsynth {
                let open_id = ui.scope_id().with("dock_search_open");
                let mut open = ui
                    .data_mut(|d| d.get_temp::<bool>(open_id))
                    .unwrap_or(false);
                if open {
                    let search_resp = crate::widgets::text_input::control_text_input(
                        ui,
                        search,
                        88.0,
                        "mix_search",
                        Some(t!("mix.search").as_ref()),
                    );
                    crate::widgets::hint::hover(ui.ctx(), &search_resp, t!("hint.dock.search"));
                    if search_resp.lost_focus() && search.is_empty() {
                        open = false;
                    }
                } else if icon_button(
                    ui,
                    egui_material_icons::icons::ICON_SEARCH,
                    14.0,
                    crate::theme::text_secondary(),
                    t!("hint.dock.search"),
                )
                .clicked()
                {
                    open = true;
                }
                ui.data_mut(|d| d.insert_temp(open_id, open));
            }

            // 界面按钮：插件设备打开原生 GUI；XSynth 打开音色库配置窗口。
            let (icon, hover) = if use_xsynth {
                (
                    egui_material_icons::icons::ICON_LIBRARY_MUSIC,
                    t!("hint.dock.instrument_gui").to_string(),
                )
            } else {
                (
                    egui_material_icons::icons::ICON_HOME_STORAGE,
                    t!("hint.dock.instrument_gui").to_string(),
                )
            };
            if icon_button(ui, icon, 14.0, crate::theme::text_secondary(), hover).clicked() {
                *open_gui = Some(DockDevice::Instrument);
            }

            // 参数面板入口：内置 XSynth 打开工程级音源参数窗口，
            // 插件乐器打开插件参数列表。
            if icon_button(
                ui,
                egui_material_icons::icons::ICON_TUNE,
                14.0,
                crate::theme::text_secondary(),
                t!("dock.open_params"),
            )
            .clicked()
            {
                if use_xsynth {
                    *open_synth_params = midi_channel;
                } else {
                    *open_params = Some(DockDevice::Instrument);
                }
            }

            // 更换乐器：内置 XSynth 与 VST/CLAP 插件在同一个选择器里切换。
            if icon_button(
                ui,
                egui_material_icons::icons::ICON_SWAP_HORIZ,
                14.0,
                crate::theme::text_secondary(),
                t!("hint.dock.instrument_picker"),
            )
            .clicked()
            {
                *open_instrument_picker = midi_channel;
            }
        },
    );

    let content = card_content_rect(rect, title_rect);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(content)
            .layout(egui::Layout::top_down(egui::Align::LEFT)),
    );
    child.set_clip_rect(rect);

    if use_xsynth {
        let needle = search.trim().to_lowercase();
        let filtered: Vec<DockParam> = lane_current
            .iter()
            .filter(|p| needle.is_empty() || p.name.to_lowercase().contains(&needle))
            .cloned()
            .collect();
        egui::ScrollArea::vertical()
            .id_salt("xsynth_knobs")
            .auto_shrink([false, false])
            .show(&mut child, |ui| {
                param_knobs(ui, &filtered, knob_actions);
            });
    } else {
        // 插件乐器：参数面板入口 + 提示。
        if crate::widgets::flat::flat_button(&mut child, t!("dock.open_params")).clicked() {
            *open_params = Some(DockDevice::Instrument);
        }
        child.add_space(4.0);
        child.label(
            egui::RichText::new(t!("dock.plugin_hint"))
                .size(crate::theme::SMALL_FONT)
                .color(crate::theme::text_muted()),
        );
    }

    clicked
}
