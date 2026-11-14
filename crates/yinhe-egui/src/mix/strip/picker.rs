//! 插件选择器与发送面板。

use eframe::egui;
use rust_i18n::t;
use yinhe_audio::InsertTarget;

use crate::mix::plugin_instance::PluginEntry;

use super::*;
use crate::widgets::meter::gain_to_db;

/// 插件选择器窗口 id（打开动作与窗口声明共用一个来源）。
///
/// 乐器/效果器两个入口互斥，共用同一个窗口。
pub(crate) fn picker_viewport_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("mix_plugin_picker")
}

/// 发送面板窗口 id（打开动作与窗口声明共用一个来源）。
pub(crate) fn sends_viewport_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("mix_send_popup")
}

/// 发送面板（某源通道对各总线的发送量 / 推子前推子后）。
pub(crate) fn send_popup(
    app: &mut App,
    ctx: &egui::Context,
    channel: u8,
    actions: &mut Vec<MixAction>,
) {
    let Some(idx) = app.workspace.active_doc else {
        return;
    };
    let bus_count = app.workspace.documents[idx].mixer.buses.len();
    let current: Vec<(f32, bool)> = (0..bus_count)
        .map(|b| {
            app.workspace.documents[idx]
                .mixer
                .sends
                .get(channel as usize)
                .and_then(|list| list.iter().find(|s| s.bus as usize == b))
                .map(|s| (s.amount, s.pre_fader))
                .unwrap_or((0.0, false))
        })
        .collect();

    // 独立 OS 窗口（viewport）：发送面板。
    let id = sends_viewport_id();
    crate::chrome::dialog::raise_on_open(ctx, id);
    let title = format!("{} {}", t!("mix.sends"), crate::mix::channel_label(channel));
    let mut close = false;
    ctx.show_viewport_immediate(
        id,
        crate::chrome::dialog::viewport_builder(title.as_ref(), [300.0, 260.0], false),
        |vctx, _class| {
            if vctx.input(|i| i.viewport().close_requested()) {
                close = true;
            }
            let mut closed = close;
            egui::CentralPanel::default()
                .frame(egui::Frame {
                    fill: crate::theme::app_bg(),
                    ..Default::default()
                })
                .show(vctx, |ui| {
                    crate::chrome::dialog::title_bar(ui, title.as_ref(), &mut closed, false);
                    egui::Frame::new()
                        .inner_margin(egui::Margin {
                            left: 12,
                            right: 12,
                            top: 0,
                            bottom: 12,
                        })
                        .show(ui, |ui| {
                            if bus_count == 0 {
                                ui.label(
                                    egui::RichText::new(t!("mix.no_buses"))
                                        .color(crate::theme::text_muted()),
                                );
                                return;
                            }
                            for (b, (amount, pre)) in current.iter().enumerate() {
                                crate::widgets::rows::inline_row(ui, |ui| {
                                    ui.label(format!("BUS {}", b + 1));
                                    let mut val = *amount;
                                    let slider = crate::widgets::slider::control_slider(
                                        ui,
                                        &mut val,
                                        0.0..=2.0,
                                        ui.available_width().min(160.0),
                                        None,
                                        false,
                                    );
                                    crate::widgets::hint::hover(
                                        ui.ctx(),
                                        &slider,
                                        t!("hint.mix.send_amount"),
                                    );
                                    if slider.changed() {
                                        actions.push(MixAction::SetSend {
                                            channel,
                                            bus: b as u8,
                                            amount: val,
                                            pre_fader: *pre,
                                        });
                                    }
                                    let db = if val <= 0.0 {
                                        "-∞".to_string()
                                    } else {
                                        format!("{:+.1} dB", gain_to_db(val))
                                    };
                                    ui.label(
                                        egui::RichText::new(db)
                                            .size(crate::theme::SMALL_FONT)
                                            .color(crate::theme::text_secondary()),
                                    );
                                    let mut pre_flag = *pre;
                                    let pre_resp = ui.checkbox(&mut pre_flag, t!("mix.pre_fader"));
                                    crate::widgets::hint::hover(
                                        ui.ctx(),
                                        &pre_resp,
                                        t!("hint.mix.pre_fader"),
                                    );
                                    if pre_resp.changed() {
                                        actions.push(MixAction::SetSend {
                                            channel,
                                            bus: b as u8,
                                            amount: val,
                                            pre_fader: pre_flag,
                                        });
                                    }
                                });
                            }
                        });
                });
            if closed {
                close = true;
            }
        },
    );
    if close {
        crate::chrome::dialog::mark_viewport_closed(ctx, id);
        app.mix.sends_for = None;
    }
}

/// 选择器左侧分类页签。
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum PickerTab {
    /// 全部（所有插件 + 内置 XSynth）。
    #[default]
    All,
    /// 乐器（内置 XSynth + 插件乐器）。
    Instrument,
    /// 效果器（插件效果器）。
    Effect,
    /// 扩展（全部已安装的 VST / CLAP 插件，不含内置）。
    Extension,
}

/// 选择器中内置 XSynth 行的选中 id（非插件条目）。
const XS_SELECTED_ID: &str = "__xsynth__";

impl PickerTab {
    const ALL: [Self; 4] = [Self::All, Self::Instrument, Self::Effect, Self::Extension];

    fn icon(self) -> egui_material_icons::MaterialIcon {
        use egui_material_icons::icons::*;
        match self {
            Self::All => ICON_APPS,
            Self::Instrument => ICON_PIANO,
            Self::Effect => ICON_GRADIENT,
            Self::Extension => ICON_EXTENSION,
        }
    }

    fn label(self) -> String {
        match self {
            Self::All => t!("mix.picker_tab_all").to_string(),
            Self::Instrument => t!("mix.picker_tab_instrument").to_string(),
            Self::Effect => t!("mix.picker_tab_effect").to_string(),
            Self::Extension => t!("mix.picker_tab_extension").to_string(),
        }
    }

    /// 该条目是否属于本分类。
    fn matches(self, plugin: &PluginEntry) -> bool {
        match self {
            Self::All => true,
            Self::Instrument => plugin.is_instrument,
            Self::Effect => plugin.is_effect,
            Self::Extension => true,
        }
    }

    /// 是否显示内置 XSynth 项（全部 / 乐器分类）。
    fn shows_xsynth(self) -> bool {
        matches!(self, Self::All | Self::Instrument)
    }
}

/// 选择器用途：效果器（insert 链）或乐器（MIDI 通道挂载）。
#[derive(Clone, Copy)]
enum PickerKind {
    /// 效果器：选中插件加到 `target` 的 insert 链。
    Effect { target: InsertTarget },
    /// 乐器：选中插件挂到 MIDI 通道；内置 XSynth 项 = 清除插件挂载。
    Instrument { channel: u8 },
}

impl PickerKind {
    fn viewport_id(self) -> egui::ViewportId {
        // 统一窗口：乐器/效果器共用（两入口互斥，同一时刻只开一个）。
        picker_viewport_id()
    }

    /// 语境是否允许加载该插件（效果器语境不接受乐器插件）。
    fn can_pick(self, plugin: &PluginEntry) -> bool {
        match self {
            Self::Effect { .. } => plugin.is_effect,
            Self::Instrument { .. } => true,
        }
    }

    /// 选中一个插件条目：按语境与插件类型分派——
    /// 乐器语境选乐器 = 挂到通道；乐器语境选效果器 = 加到该通道 insert 链；
    /// 效果器语境只接受效果器。
    fn pick_plugin(self, plugin: &PluginEntry, actions: &mut Vec<MixAction>) {
        match self {
            Self::Effect { target } if plugin.is_effect => actions.push(MixAction::AddInsert {
                target,
                plugin: plugin.clone(),
            }),
            Self::Instrument { channel } if plugin.is_instrument => {
                actions.push(MixAction::AssignInstrument {
                    channel,
                    plugin: plugin.clone(),
                })
            }
            Self::Instrument { channel } => actions.push(MixAction::AddInsert {
                target: InsertTarget::Channel(channel),
                plugin: plugin.clone(),
            }),
            _ => {}
        }
    }

    /// 点击内置 XSynth（仅乐器）：清空该通道的插件乐器链（回到默认 XSynth）。
    fn pick_builtin(self, actions: &mut Vec<MixAction>) {
        if let Self::Instrument { channel } = self {
            actions.push(MixAction::ClearInstruments { channel });
        }
    }

    /// 窗口关闭后清理打开标记（两个入口共用同一窗口）。
    fn clear_open(self, app: &mut App) {
        app.mix.picker_for = None;
        app.mix.instrument_picker_for = None;
    }
}

/// 加载当前选中项（双击 / 「好」共用）：插件按语境分派，XSynth 清空乐器链。
fn pick_selected(app: &App, kind: PickerKind, actions: &mut Vec<MixAction>) {
    let Some(sel) = app.mix.picker_selected.clone() else {
        return;
    };
    if sel == XS_SELECTED_ID {
        kind.pick_builtin(actions);
        return;
    }
    if let Some(p) = app
        .mix
        .scanned
        .as_ref()
        .and_then(|list| list.iter().find(|p| p.id == sel))
        && p.error.is_none()
        && kind.can_pick(p)
    {
        kind.pick_plugin(p, actions);
    }
}

/// 左侧分类栏（PR 工具栏同款 44px 竖条）：乐器 / 效果器 / 扩展 / 收藏。
fn picker_tab_bar(app: &mut App, ui: &mut egui::Ui) {
    let bar_w = crate::theme::TOOL_BAR_W;
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(bar_w, ui.available_height()),
        egui::Sense::hover(),
    );
    // 贴窗口左缘、无底色（与窗口背景一体）。
    let btn = crate::theme::TRANSPORT_BTN_SIZE;
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
            ui.add_space(6.0);
            for tab in PickerTab::ALL {
                let selected = app.mix.picker_tab == tab;
                let color = if selected {
                    crate::theme::accent_active()
                } else {
                    crate::theme::text_primary()
                };
                let sel_bg = selected.then(crate::theme::selected_bg);
                let resp = ui
                    .push_id(("picker_tab", tab as usize), |ui| {
                        crate::widgets::flat::flat_button_filled_ghost(
                            ui,
                            tab.icon()
                                .rich_text()
                                .size(crate::theme::TRANSPORT_BTN_FONT)
                                .color(color),
                            egui::vec2(btn, btn),
                            sel_bg,
                            true,
                        )
                    })
                    .inner;
                if resp.clicked() {
                    app.mix.picker_tab = tab;
                    app.mix.picker_selected = None;
                }
                // 独立窗口内用鼠标旁 tooltip（模式栏讲解不覆盖子窗口）。
                resp.on_hover_text(tab.label());
            }
        });
    });
}

/// 厂家分组标题行（列表按厂家分组时的组头）。
fn vendor_header(ui: &mut egui::Ui, vendor: &str) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.0), egui::Sense::hover());
    ui.painter().text(
        egui::pos2(rect.min.x + 6.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        vendor,
        egui::FontId::proportional(crate::theme::SUB_TITLE_FONT),
        crate::theme::text_primary(),
    );
    ui.painter().hline(
        rect.x_range(),
        rect.max.y - 0.5,
        egui::Stroke::new(1.0, crate::theme::line_fg()),
    );
}

/// 中间列表：自绘搜索框 + 星标筛选 + 按厂家分组 + 单击选中 / 双击加载。
fn picker_list(
    app: &mut App,
    ui: &mut egui::Ui,
    kind: PickerKind,
    actions: &mut Vec<MixAction>,
    close: &mut bool,
) {
    // 列宽由外部 three_columns 分配，这里直接使用当前 cell。
    // 搜索行：自绘输入框 + 星标筛选开关（点亮 = 只看收藏）。
    let star_w = 26.0;
    let mut filter = app.mix.picker_filter.clone();
    let fav_only = app.mix.picker_fav_only;
    let mut next_fav = fav_only;
    let search_h = crate::widgets::control::h(ui.ctx());
    crate::widgets::grid::fixed_columns(
        ui,
        6.0,
        search_h,
        &[
            egui_extras::Size::remainder(),
            egui_extras::Size::exact(star_w),
        ],
        |cells| {
            cells.cell(|ui| {
                let search_resp = crate::widgets::text_input::control_text_input(
                    ui,
                    &mut filter,
                    ui.available_width().max(1.0),
                    "mix_picker_search",
                    Some(&t!("mix.search")),
                );
                search_resp.on_hover_text(t!("hint.mix.picker_search"));
            });
            cells.cell(|ui| {
                let (icon, color) = if fav_only {
                    (
                        egui_material_icons::icons::ICON_STAR,
                        crate::theme::warning_gold(),
                    )
                } else {
                    (
                        egui_material_icons::icons::ICON_STAR.outlined(),
                        crate::theme::text_muted(),
                    )
                };
                let star_resp = crate::widgets::flat::flat_button_filled_ghost(
                    ui,
                    icon.rich_text().size(crate::theme::ICON_FONT).color(color),
                    egui::vec2(star_w, crate::widgets::control::h(ui.ctx())),
                    None,
                    true,
                )
                .on_hover_text(t!("hint.mix.picker_fav_filter"));
                if star_resp.clicked() {
                    next_fav = !fav_only;
                }
            });
        },
    );
    app.mix.picker_filter = filter;
    app.mix.picker_fav_only = next_fav;
    ui.separator();
    let filter = app.mix.picker_filter.to_lowercase();
    let favorites = app.audio_settings.favorite_plugins.clone();
    let fav_only = app.mix.picker_fav_only;
    let tab = app.mix.picker_tab;
    let mut clicked: Option<String> = None;
    let mut activated: Option<String> = None;
    egui::ScrollArea::vertical()
        .id_salt("mix_plugin_picker_list")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let mut any = false;
            // 内置 XSynth：全部/乐器分类（选择 = 清除插件挂载）；
            // 星标筛选（只看收藏）时隐藏（XSynth 不是插件、不可收藏）。
            if tab.shows_xsynth() && !fav_only && (filter.is_empty() || "xsynth".contains(&filter))
            {
                any = true;
                let selected = app.mix.picker_selected.as_deref() == Some(XS_SELECTED_ID);
                let resp = plugin_row(
                    ui,
                    egui_material_icons::icons::ICON_PIANO,
                    "XSynth",
                    yinhe_mixer::PluginFormat::Builtin,
                    true,
                    selected,
                )
                .on_hover_text(t!("hint.mix.picker_xsynth"));
                if resp.clicked() {
                    clicked = Some(XS_SELECTED_ID.to_string());
                }
                if resp.double_clicked() {
                    activated = Some(XS_SELECTED_ID.to_string());
                }
            }
            if let Some(plugins) = app.mix.scanned.as_ref() {
                // 过滤（分类 + 星标筛选 + 搜索）后**按厂家分组**，
                // 组内按名称排序；组头仅在厂家非空时绘制。
                let mut list: Vec<&PluginEntry> = plugins
                    .iter()
                    .filter(|p| tab.matches(p))
                    .filter(|p| !fav_only || favorites.iter().any(|f| f == &p.id))
                    .filter(|p| filter.is_empty() || p.name.to_lowercase().contains(&filter))
                    .collect();
                list.sort_by(|a, b| a.vendor.cmp(&b.vendor).then_with(|| a.name.cmp(&b.name)));
                let mut last_vendor: Option<&str> = None;
                for p in list {
                    if !p.vendor.is_empty() && last_vendor != Some(p.vendor.as_str()) {
                        vendor_header(ui, &p.vendor);
                        last_vendor = Some(p.vendor.as_str());
                    }
                    any = true;
                    let enabled = p.error.is_none() && kind.can_pick(p);
                    let selected = app.mix.picker_selected.as_deref() == Some(p.id.as_str());
                    let resp = plugin_row(
                        ui,
                        plugin_kind_icon(p),
                        &p.display_name(),
                        p.format,
                        enabled,
                        selected,
                    );
                    if let Some(err) = &p.error {
                        // 加载失败的插件：灰色不可选，hover 显示原因。
                        resp.on_hover_text(err);
                        continue;
                    }
                    if !kind.can_pick(p) {
                        resp.on_hover_text(t!("mix.picker_not_applicable"));
                        continue;
                    }
                    let resp = resp.on_hover_text(t!("hint.mix.picker_plugin"));
                    if resp.clicked() {
                        clicked = Some(p.id.clone());
                    }
                    if resp.double_clicked() {
                        activated = Some(p.id.clone());
                    }
                }
            }
            if !any {
                ui.label(
                    egui::RichText::new(t!("mix.picker_empty").to_string())
                        .color(crate::theme::text_muted()),
                );
            }
        });
    if let Some(sel) = clicked {
        app.mix.picker_selected = Some(sel);
    }
    if let Some(sel) = activated {
        app.mix.picker_selected = Some(sel);
        pick_selected(app, kind, actions);
        *close = true;
    }
}

/// 右侧信息面板：选中插件的名称/厂商/格式/路径 + 收藏 star 开关。
fn picker_info(app: &mut App, ui: &mut egui::Ui) {
    let info_w = ui.available_width().max(1.0);
    ui.allocate_ui_with_layout(
        egui::vec2(info_w, ui.available_height()),
        egui::Layout::top_down(egui::Align::LEFT),
        |ui| {
            ui.add_space(4.0);
            let Some(sel) = app.mix.picker_selected.clone() else {
                ui.label(
                    egui::RichText::new(t!("mix.picker_info_empty").to_string())
                        .size(crate::theme::SMALL_FONT)
                        .color(crate::theme::text_muted()),
                );
                return;
            };
            if sel == XS_SELECTED_ID {
                ui.label(
                    egui::RichText::new("XSynth")
                        .size(crate::theme::SUB_TITLE_FONT)
                        .color(crate::theme::text_primary()),
                );
                ui.label(
                    egui::RichText::new(t!("mix.picker_tab_instrument").to_string())
                        .size(crate::theme::SMALL_FONT)
                        .color(crate::theme::text_label()),
                );
                return;
            }
            let Some(p) = app
                .mix
                .scanned
                .as_ref()
                .and_then(|l| l.iter().find(|p| p.id == sel))
                .cloned()
            else {
                return;
            };
            let is_fav = app
                .audio_settings
                .favorite_plugins
                .iter()
                .any(|f| f == &p.id);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(&p.name)
                        .size(crate::theme::SUB_TITLE_FONT)
                        .color(crate::theme::text_primary()),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (icon, color) = if is_fav {
                        (
                            egui_material_icons::icons::ICON_STAR,
                            crate::theme::warning_gold(),
                        )
                    } else {
                        (
                            egui_material_icons::icons::ICON_STAR.outlined(),
                            crate::theme::text_muted(),
                        )
                    };
                    let resp = crate::widgets::flat::flat_button_filled_ghost(
                        ui,
                        icon.rich_text().size(crate::theme::ICON_FONT).color(color),
                        egui::vec2(24.0, 24.0),
                        None,
                        true,
                    )
                    .on_hover_text(t!("hint.mix.picker_favorite"));
                    if resp.clicked() {
                        if is_fav {
                            app.audio_settings.favorite_plugins.retain(|f| f != &p.id);
                        } else {
                            app.audio_settings.favorite_plugins.push(p.id.clone());
                        }
                        app.audio_settings.save();
                    }
                });
            });
            ui.add_space(4.0);
            let kind_text = if p.is_instrument {
                t!("mix.picker_tab_instrument")
            } else if p.is_effect {
                t!("mix.picker_tab_effect")
            } else {
                t!("mix.picker_tab_extension")
            };
            ui.label(
                egui::RichText::new(format!(
                    "{} · {}",
                    kind_text,
                    match p.format {
                        yinhe_mixer::PluginFormat::Clap => "CLAP",
                        yinhe_mixer::PluginFormat::Vst3 => "VST3",
                        yinhe_mixer::PluginFormat::Builtin => "Yinhe",
                    }
                ))
                .size(crate::theme::SMALL_FONT)
                .color(crate::theme::text_label()),
            );
            ui.label(
                egui::RichText::new(&p.vendor)
                    .size(crate::theme::SMALL_FONT)
                    .color(crate::theme::text_label()),
            );
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(p.path.display().to_string())
                    .size(crate::theme::SMALL_LABEL_FONT)
                    .color(crate::theme::text_muted()),
            );
        },
    );
}

/// 统一插件选择器窗口：左侧分类（全部/乐器/效果器/扩展，贴窗口左缘）+ 中间列表 +
/// 右侧插件信息与收藏 + 底部按钮。单击选中、双击或「好」加载选中项。
fn picker_window(
    app: &mut App,
    ctx: &egui::Context,
    kind: PickerKind,
    actions: &mut Vec<MixAction>,
) {
    let id = kind.viewport_id();
    crate::chrome::dialog::raise_on_open(ctx, id);
    let title = t!("mix.picker_title").to_string();
    let mut close = false;
    ctx.show_viewport_immediate(
        id,
        crate::chrome::dialog::viewport_builder(title.as_ref(), [680.0, 460.0], true),
        |vctx, _class| {
            if vctx.input(|i| i.viewport().close_requested()) {
                close = true;
            }
            let mut closed = close;
            egui::CentralPanel::default()
                .frame(egui::Frame {
                    fill: crate::theme::app_bg(),
                    ..Default::default()
                })
                .show(vctx, |ui| {
                    crate::chrome::dialog::title_bar(ui, title.as_ref(), &mut closed, false);
                    egui::Frame::new()
                        .inner_margin(egui::Margin {
                            // 左栏贴窗口左缘（无内边距、无底色），列表与信息保留右/下边距。
                            left: 0,
                            right: 12,
                            top: 0,
                            bottom: 12,
                        })
                        .show(ui, |ui| {
                            // 左栏分类 + 中间列表上下贯通到窗口底缘；只有右栏
                            // （插件信息）带底部按钮行，按钮只占右侧栏宽度。
                            let btn_zone = crate::chrome::dialog_buttons::btn_zone_h(ui.ctx());
                            crate::widgets::grid::three_columns(
                                ui,
                                12.0,
                                crate::theme::TOOL_BAR_W,
                                190.0,
                                |cells| {
                                    cells.cell(|ui| picker_tab_bar(app, ui));
                                    cells.cell(|ui| {
                                        picker_list(app, ui, kind, actions, &mut close)
                                    });
                                    cells.cell(|ui| {
                                        crate::widgets::grid::content_footer(
                                            ui,
                                            0.0,
                                            btn_zone,
                                            |cells| {
                                                cells.cell(|ui| picker_info(app, ui));
                                                cells.cell(|ui| {
                                                    // 底部按钮贴底（bottom_up）：下边距与右边距一致。
                                                    ui.with_layout(
                                                        egui::Layout::bottom_up(
                                                            egui::Align::Center,
                                                        ),
                                                        |ui| {
                                                            let ok_text = t!("mix.picker_ok");
                                                            let cancel_text =
                                                                t!("mix.picker_cancel");
                                                            let ok_enabled = app
                                                                .mix
                                                                .picker_selected
                                                                .is_some();
                                                            if let Some(idx) =
                                                                crate::chrome::dialog_buttons::dialog_button_row(
                                                                    ui,
                                                                    &[
                                                                        crate::chrome::dialog_buttons::DialogButton::secondary(
                                                                            cancel_text.as_ref(),
                                                                        ),
                                                                        crate::chrome::dialog_buttons::DialogButton::primary(
                                                                            ok_text.as_ref(),
                                                                        )
                                                                        .enabled(ok_enabled),
                                                                    ],
                                                                )
                                                            {
                                                                match idx {
                                                                    0 => close = true,
                                                                    1 => {
                                                                        pick_selected(
                                                                            app, kind, actions,
                                                                        );
                                                                        close = true;
                                                                    }
                                                                    _ => {}
                                                                }
                                                            }
                                                        },
                                                    );
                                                });
                                            },
                                        );
                                    });
                                },
                            );
                        });
                });
            if closed {
                close = true;
            }
        },
    );
    if close {
        crate::chrome::dialog::mark_viewport_closed(ctx, id);
        kind.clear_open(app);
    }
}

/// 统一插件选择器（独立 OS 窗口）：效果器语境（insert 链）。
/// 左侧分类可切到乐器/扩展/收藏；乐器插件在此语境不可选。
pub(crate) fn plugin_picker(
    app: &mut App,
    ctx: &egui::Context,
    target: InsertTarget,
    actions: &mut Vec<MixAction>,
) {
    picker_window(app, ctx, PickerKind::Effect { target }, actions);
}

/// 统一插件选择器（独立 OS 窗口）：乐器语境（MIDI 通道挂载）。
/// 初始分类为乐器；选择效果器则加到该通道 insert 链；选择 XSynth 清空插件挂载。
pub(crate) fn instrument_picker(
    app: &mut App,
    ctx: &egui::Context,
    channel: u8,
    actions: &mut Vec<MixAction>,
) {
    picker_window(app, ctx, PickerKind::Instrument { channel }, actions);
}

/// 插件分类图标：乐器 / 效果器 / 扩展（VST/CLAP 但不声明类型时）。
///
/// 列表行用它做视觉分类标识；XSynth 是内置采样器，按乐器显示。
fn plugin_kind_icon(plugin: &PluginEntry) -> egui_material_icons::MaterialIcon {
    use egui_material_icons::icons::*;
    if plugin.is_instrument {
        ICON_PIANO
    } else if plugin.is_effect {
        ICON_GRADIENT
    } else {
        ICON_EXTENSION
    }
}

/// 插件选择器的一行：左分类图标 + 名称（超宽裁剪）+ 右格式标签（CLAP / VST3）。
///
/// `enabled = false`（加载失败/语境不支持）时灰色且不响应点击；
/// `selected` 时整行选中底色；hover 提示由调用方追加。
fn plugin_row(
    ui: &mut egui::Ui,
    icon: egui_material_icons::MaterialIcon,
    name: &str,
    format: yinhe_mixer::PluginFormat,
    enabled: bool,
    selected: bool,
) -> egui::Response {
    let row_h = 24.0;
    let (rect, resp) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), row_h),
        egui::Sense::click(),
    );
    let text_color = if enabled {
        crate::theme::text_primary()
    } else {
        crate::theme::text_muted()
    };
    if selected {
        ui.painter()
            .rect_filled(rect, 0.0, crate::theme::selected_bg());
    } else if enabled && resp.hovered() {
        ui.painter().rect_filled(
            rect,
            0.0,
            crate::theme::hover_color(crate::theme::track_bg()),
        );
    }
    let font = egui::FontId::proportional(crate::theme::SMALL_FONT);
    // 名称（左侧，超宽裁剪；右侧留 56px 给格式标签）。图标与文字同一 LayoutJob，
    // 保证基线与行高一致；图标用 Material 家族，避免被正文私有字形抢占。
    let name_rect = egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x - 56.0, rect.max.y));
    let job =
        crate::widgets::icon_text::icon_text(icon, name, crate::theme::SMALL_FONT, text_color);
    let galley = ui.painter().layout_job(job);
    let pos = egui::pos2(rect.min.x + 6.0, rect.center().y - galley.size().y * 0.5);
    ui.painter()
        .with_clip_rect(name_rect)
        .galley(pos, galley, text_color);
    // 格式标签（右对齐）。
    let fmt = match format {
        yinhe_mixer::PluginFormat::Clap => "CLAP",
        yinhe_mixer::PluginFormat::Vst3 => "VST3",
        // 内置内容（如 XSynth）归 yinhe 自身，而非 DSP 插件格式。
        yinhe_mixer::PluginFormat::Builtin => "Yinhe",
    };
    ui.painter().text(
        egui::pos2(rect.max.x - 6.0, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        fmt,
        font,
        crate::theme::text_muted(),
    );
    if enabled {
        resp
    } else {
        resp.on_hover_cursor(egui::CursorIcon::NotAllowed)
    }
}
