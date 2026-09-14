//! 插件参数自动化选择窗口（独立 OS viewport）。
//!
//! 轨道右键「添加插件参数自动化…」触发：列出该乐器通道插件的全部参数
//!（Serum 等可达数千个，虚拟滚动 + 搜索过滤），点击参数创建/定位对应
//! 的 AM lane。已存在的参数用对勾标记。
//!
//! 参数列表在打开时（App 侧）枚举一次填入状态；窗口本身不访问插件实例。

use eframe::egui;
use rust_i18n::t;

use crate::mix::plugin_instance::PluginParam;

/// 窗口状态（挂在 App 上，跨帧保留；打开时重置）。
#[derive(Default)]
pub(crate) struct PluginParamPickerState {
    pub open: bool,
    /// 打开后首帧把窗口提到最前。
    pub just_opened: bool,
    /// 目标轨道（AM lane 建在该轨上）。
    pub track_idx: usize,
    pub instrument_channel: u16,
    /// 通道标签（标题显示，如 Inst-01）。
    channel_label: String,
    /// 插件显示名（副标题显示）。
    plugin_name: String,
    params: Vec<PluginParam>,
    /// 搜索过滤后的参数索引。
    filtered: Vec<usize>,
    search: String,
    filter_dirty: bool,
    /// 已有 AM lane 的 param_id（对勾标记）。
    pub existing: std::collections::HashSet<u32>,
}

/// 用户动作（窗口不修改模型，交给 App 处理）。
pub(crate) enum PluginParamPickerAction {
    None,
    /// 为该参数创建/定位 AM lane。
    Add {
        track_idx: usize,
        param_id: u32,
        name: String,
    },
    Close,
}

impl PluginParamPickerState {
    /// 打开窗口并填入参数列表。
    #[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
    pub(crate) fn open(
        &mut self,
        track_idx: usize,
        instrument_channel: u16,
        channel_label: String,
        plugin_name: String,
        params: Vec<PluginParam>,
        existing: std::collections::HashSet<u32>,
    ) {
        self.open = true;
        self.just_opened = true;
        self.track_idx = track_idx;
        self.instrument_channel = instrument_channel;
        self.channel_label = channel_label;
        self.plugin_name = plugin_name;
        self.params = params;
        self.search.clear();
        self.filter_dirty = true;
        self.existing = existing;
        self.rebuild_filter();
    }

    fn rebuild_filter(&mut self) {
        let needle = self.search.trim().to_lowercase();
        self.filtered.clear();
        if needle.is_empty() {
            self.filtered.extend(0..self.params.len());
        } else {
            self.filtered
                .extend(self.params.iter().enumerate().filter_map(|(i, p)| {
                    (p.name.to_lowercase().contains(&needle)
                        || p.module.to_lowercase().contains(&needle))
                    .then_some(i)
                }));
        }
        self.filter_dirty = false;
    }
}

/// 显示插件参数选择窗口。
pub(crate) fn show_viewport(
    ctx: &egui::Context,
    state: &mut PluginParamPickerState,
) -> PluginParamPickerAction {
    let viewport_id = egui::ViewportId::from_hash_of("plugin_param_picker");
    let title = t!(
        "dialog.plugin_param.title",
        ch = state.channel_label.as_str()
    );
    let mut action = PluginParamPickerAction::None;
    let mut closed = false;

    ctx.show_viewport_immediate(
        viewport_id,
        crate::chrome::dialog::viewport_builder(title.as_ref(), [420.0, 540.0], false),
        |vctx, _class| {
            if vctx.input(|i| i.viewport().close_requested()) {
                closed = true;
            }
            let mut close = closed;
            egui::CentralPanel::default()
                .frame(egui::Frame {
                    fill: crate::theme::app_bg(),
                    ..Default::default()
                })
                .show(vctx, |ui| {
                    crate::chrome::dialog::title_bar(ui, title.as_ref(), &mut close, false);
                    egui::Frame::new()
                        .inner_margin(egui::Margin {
                            left: 12,
                            right: 12,
                            top: 0,
                            bottom: 12,
                        })
                        .show(ui, |ui| {
                            if state.filter_dirty {
                                state.rebuild_filter();
                            }
                            // 副标题：插件名 + 参数数量 + 搜索。
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(format!(
                                        "{} · {}",
                                        state.plugin_name,
                                        t!("dialog.plugin_param.count", n = state.params.len())
                                    ))
                                    .size(crate::theme::SMALL_FONT)
                                    .color(crate::theme::text_muted()),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui
                                            .add(
                                                egui::TextEdit::singleline(&mut state.search)
                                                    .desired_width(140.0)
                                                    .hint_text(t!("mix.search")),
                                            )
                                            .changed()
                                        {
                                            state.filter_dirty = true;
                                        }
                                    },
                                );
                            });
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(t!("dialog.plugin_param.hint"))
                                    .size(crate::theme::SMALL_FONT)
                                    .color(crate::theme::text_muted()),
                            );
                            ui.separator();

                            if state.filtered.is_empty() {
                                ui.label(
                                    egui::RichText::new(t!("dialog.plugin_param.empty"))
                                        .size(crate::theme::SMALL_FONT)
                                        .color(crate::theme::text_muted()),
                                );
                                return;
                            }

                            let row_h = 22.0;
                            egui::ScrollArea::vertical()
                                .id_salt("plugin_param_picker_list")
                                .auto_shrink([false, false])
                                .max_height(ui.available_height())
                                .show_rows(ui, row_h, state.filtered.len(), |ui, range| {
                                    for &pi in &state.filtered[range] {
                                        let p = &state.params[pi];
                                        let label = if p.module.is_empty() {
                                            p.name.clone()
                                        } else {
                                            format!("{}/{}", p.module, p.name)
                                        };
                                        let added = state.existing.contains(&p.id);
                                        let text_color = if p.read_only {
                                            crate::theme::text_muted()
                                        } else {
                                            crate::theme::text_primary()
                                        };
                                        let resp = ui.add_sized(
                                            [ui.available_width(), row_h],
                                            egui::Button::new(
                                                egui::RichText::new(label)
                                                    .size(crate::theme::SMALL_FONT)
                                                    .color(text_color),
                                            )
                                            .fill(egui::Color32::TRANSPARENT)
                                            .stroke(egui::Stroke::NONE),
                                        );
                                        if added {
                                            ui.painter().text(
                                                egui::pos2(
                                                    resp.rect.max.x - 10.0,
                                                    resp.rect.center().y,
                                                ),
                                                egui::Align2::RIGHT_CENTER,
                                                egui_material_icons::icons::ICON_CHECK_CIRCLE
                                                    .codepoint,
                                                egui::FontId::new(
                                                    13.0,
                                                    egui_material_icons::icons::ICON_CHECK_CIRCLE
                                                        .font_family(),
                                                ),
                                                crate::theme::accent_active(),
                                            );
                                        }
                                        if resp.clicked() && !p.read_only {
                                            state.existing.insert(p.id);
                                            action = PluginParamPickerAction::Add {
                                                track_idx: state.track_idx,
                                                param_id: p.id,
                                                name: p.name.clone(),
                                            };
                                        }
                                    }
                                });
                        });
                });
            if close {
                vctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                closed = true;
            }
        },
    );

    if closed {
        state.open = false;
        PluginParamPickerAction::Close
    } else {
        action
    }
}
