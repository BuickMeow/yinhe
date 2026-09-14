//! 「添加自动化」窗口（独立 OS viewport）。
//!
//! 自动化属于**乐器设备**（XSynth 或插件），不属于音轨：
//! - XSynth 设备：列出内置参数（CC/PB/RPN）+ 自定义 CC；
//! - 插件设备：列出插件参数（可达数千个，虚拟滚动 + 搜索）。
//!
//! 点击条目添加对应 AM lane，再次点击移除（对勾 = 已添加）。
//!
//! 窗口不修改模型：条目标签与目标在打开时（App 侧）收集，点击返回 [`Toggle`]
//! 动作，由 App 落模型并回写对勾状态。

use eframe::egui;
use rust_i18n::t;
use yinhe_types::AutomationTarget;

/// 窗口里的一个可添加项。
pub(crate) struct AutomationEntry {
    pub target: AutomationTarget,
    /// 显示名（含 CC 名 / 模块）。
    pub label: String,
    /// 是否已有对应 AM lane（对勾）。
    pub existing: bool,
}

/// 窗口状态（挂在 App 上，跨帧保留；打开时重置）。
#[derive(Default)]
pub(crate) struct AutomationPickerState {
    pub open: bool,
    /// 打开后首帧把窗口提到最前。
    pub just_opened: bool,
    /// 目标轨道（lane 建在该轨上）。
    pub track_idx: usize,
    /// 窗口标题里的通道/设备标签（MIDI-A05 / Inst-01）。
    channel_label: String,
    /// 设备名（XSynth / Serum 2）。
    device_name: String,
    entries: Vec<AutomationEntry>,
    /// 搜索过滤后的条目索引。
    filtered: Vec<usize>,
    search: String,
    filter_dirty: bool,
    /// 是否显示自定义 CC 行（仅 XSynth 设备）。
    show_custom_cc: bool,
    /// 自定义 CC 控制器号输入。
    custom_cc: u8,
}

/// 用户动作（窗口不修改模型，交给 App 处理）。
pub(crate) enum AutomationPickerAction {
    None,
    /// 添加/移除该目标的 AM lane。
    Toggle {
        track_idx: usize,
        target: AutomationTarget,
        add: bool,
    },
    Close,
}

impl AutomationPickerState {
    /// 打开窗口并填入条目。
    #[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
    pub(crate) fn open(
        &mut self,
        track_idx: usize,
        channel_label: String,
        device_name: String,
        entries: Vec<AutomationEntry>,
        show_custom_cc: bool,
    ) {
        self.open = true;
        self.just_opened = true;
        self.track_idx = track_idx;
        self.channel_label = channel_label;
        self.device_name = device_name;
        self.entries = entries;
        self.search.clear();
        self.filter_dirty = true;
        self.show_custom_cc = show_custom_cc;
        self.custom_cc = self.custom_cc.clamp(0, 127);
        self.rebuild_filter();
    }

    /// App 落模型后回写对勾状态。
    pub(crate) fn set_existing(&mut self, target: &AutomationTarget, existing: bool) {
        if let Some(e) = self.entries.iter_mut().find(|e| &e.target == target) {
            e.existing = existing;
        }
    }

    fn rebuild_filter(&mut self) {
        let needle = self.search.trim().to_lowercase();
        self.filtered.clear();
        if needle.is_empty() {
            self.filtered.extend(0..self.entries.len());
        } else {
            self.filtered.extend(
                self.entries
                    .iter()
                    .enumerate()
                    .filter_map(|(i, e)| e.label.to_lowercase().contains(&needle).then_some(i)),
            );
        }
        self.filter_dirty = false;
    }
}

/// 显示「添加自动化」窗口。
pub(crate) fn show_viewport(
    ctx: &egui::Context,
    state: &mut AutomationPickerState,
) -> AutomationPickerAction {
    let viewport_id = egui::ViewportId::from_hash_of("automation_picker");
    let title = t!("dialog.automation.title", ch = state.channel_label.as_str());
    let mut action = AutomationPickerAction::None;
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
                            // 副标题：设备名 + 参数数量 + 搜索。
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(format!(
                                        "{} · {}",
                                        state.device_name,
                                        t!("dialog.automation.count", n = state.entries.len())
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
                                egui::RichText::new(t!("dialog.automation.hint"))
                                    .size(crate::theme::SMALL_FONT)
                                    .color(crate::theme::text_muted()),
                            );
                            ui.separator();

                            if state.entries.is_empty() {
                                ui.label(
                                    egui::RichText::new(t!("dialog.automation.empty"))
                                        .size(crate::theme::SMALL_FONT)
                                        .color(crate::theme::text_muted()),
                                );
                            } else {
                                // 设置菜单样式的行：左标题（无描述）+ 右开关（= 已添加自动化）。
                                let row_h = 40.0;
                                egui::ScrollArea::vertical()
                                    .id_salt("automation_picker_list")
                                    .auto_shrink([false, false])
                                    .max_height(ui.available_height() - 40.0)
                                    .show_rows(ui, row_h, state.filtered.len(), |ui, range| {
                                        for &pi in &state.filtered[range] {
                                            let label = state.entries[pi].label.clone();
                                            let added = state.entries[pi].existing;
                                            let mut on = added;
                                            let mut toggled = false;
                                            ui.horizontal(|ui| {
                                                ui.label(egui::RichText::new(label).strong().size(
                                                    crate::scaling::scaled_font(
                                                        ui.ctx(),
                                                        crate::theme::SUB_TITLE_FONT,
                                                    ),
                                                ));
                                                ui.with_layout(
                                                    egui::Layout::right_to_left(
                                                        egui::Align::Center,
                                                    ),
                                                    |ui| {
                                                        if crate::widgets::switch::switch(
                                                            ui, &mut on,
                                                        )
                                                        .changed()
                                                        {
                                                            toggled = true;
                                                        }
                                                    },
                                                );
                                            });
                                            ui.add_space(6.0);
                                            ui.separator();
                                            if toggled {
                                                let target = state.entries[pi].target.clone();
                                                state.entries[pi].existing = on;
                                                action = AutomationPickerAction::Toggle {
                                                    track_idx: state.track_idx,
                                                    target,
                                                    add: on,
                                                };
                                            }
                                        }
                                    });
                            }

                            // 自定义 CC 行（仅 XSynth 设备）：输入控制器号 → 添加。
                            if state.show_custom_cc {
                                ui.add_space(8.0);
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new(t!("arrange.custom_cc")).strong().size(
                                            crate::scaling::scaled_font(
                                                ui.ctx(),
                                                crate::theme::SUB_TITLE_FONT,
                                            ),
                                        ),
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if ui
                                                .add(
                                                    egui::DragValue::new(&mut state.custom_cc)
                                                        .range(0..=127),
                                                )
                                                .changed()
                                            {
                                                // 仅更新输入值。
                                            }
                                            ui.label(t!("dialog.automation.cc_number"));
                                        },
                                    );
                                });
                                ui.add_space(4.0);
                                if ui.button(t!("arrange.create")).clicked() {
                                    let target = AutomationTarget::CC {
                                        controller: state.custom_cc,
                                    };
                                    let label = crate::arrange::lane_label(&target);
                                    if let Some(e) =
                                        state.entries.iter_mut().find(|e| e.target == target)
                                    {
                                        if !e.existing {
                                            e.existing = true;
                                            action = AutomationPickerAction::Toggle {
                                                track_idx: state.track_idx,
                                                target: target.clone(),
                                                add: true,
                                            };
                                        }
                                    } else {
                                        state.entries.push(AutomationEntry {
                                            target: target.clone(),
                                            label,
                                            existing: true,
                                        });
                                        // 新条目可能不在当前过滤结果里：重建。
                                        state.filter_dirty = true;
                                        action = AutomationPickerAction::Toggle {
                                            track_idx: state.track_idx,
                                            target,
                                            add: true,
                                        };
                                    }
                                }
                            }
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
        AutomationPickerAction::Close
    } else {
        action
    }
}
