//! 插件参数面板（egui 浮窗）：枚举、搜索、拖动参数。
//!
//! 参数写入链路：拖动 → [`ParamQueue::push`]（UI 线程）→ 渲染线程下一块以
//! `ParamValue` 事件交给插件（CLAP 宿主没有直接 set_value 的 API）。
//! 参数读取：每帧只对**可见行**调 `get_param_value`（Kontakt 等插件参数可达
//! 数千个，不整体重读），拖动中优先用本地值避免抖动。
//! 插件请求 rescan（如 Kontakt 载入新音色）时自动重枚举参数列表。

use std::collections::HashMap;

use eframe::egui;
use rust_i18n::t;
use yinhe_mixer::ParamQueue;

use super::plugin_instance::{PluginInstance, PluginParam};

use crate::app::App;

/// 参数面板目标槽位。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParamTarget {
    /// insert 链目标（源通道 / 总线 / master）。
    Insert {
        target: yinhe_audio::InsertTarget,
        slot: usize,
    },
    /// 乐器通道（0 起）。
    Instrument { channel: u16 },
}

/// 参数面板状态（打开时枚举一次；插件 rescan 时重枚举）。
pub(crate) struct ParamPanel {
    target: ParamTarget,
    /// 插件显示名（窗口标题）。
    title: String,
    /// 插件 id：槽位被换成别的插件时检测并重枚举。
    plugin_id: String,
    /// 写入队列（拖动时 push）。
    queue: std::sync::Arc<ParamQueue>,
    params: Vec<PluginParam>,
    /// 搜索过滤后的参数索引。
    filtered: Vec<usize>,
    search: String,
    /// 拖动中的本地值（优先于插件读值，松手后丢弃）。
    editing: HashMap<u32, f64>,
    /// search 变化后待重建 filtered。
    filter_dirty: bool,
    /// 本帧写过参数：用于标记工程 mixer 脏（保存时会把插件状态写进工程）。
    wrote_params: bool,
}

impl ParamPanel {
    /// 打开面板：枚举参数（无参数/枚举失败也可打开，显示空态）。
    pub(crate) fn open(target: ParamTarget, title: String, instance: &mut PluginInstance) -> Self {
        let params = instance.param_list();
        let queue = instance.param_queue();
        let mut panel = Self {
            target,
            title,
            plugin_id: instance.id().to_string(),
            queue,
            params,
            filtered: Vec::new(),
            search: String::new(),
            editing: HashMap::new(),
            filter_dirty: true,
            wrote_params: false,
        };
        panel.rebuild_filter();
        panel
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

    fn refresh(&mut self, instance: &mut PluginInstance) {
        self.params = instance.param_list();
        self.editing.clear();
        self.rebuild_filter();
    }
}

/// 每帧渲染参数面板（MIX 模式；面板未打开时无操作）。
pub(crate) fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut panel) = app.mix.param_panel.take() else {
        return;
    };
    let Some(idx) = app.workspace.active_doc else {
        app.mix.param_panel = None;
        return;
    };

    let instance: Option<&mut PluginInstance> = match panel.target {
        ParamTarget::Insert { target, slot } => app
            .mixer_racks
            .get_mut(idx)
            .and_then(|rack| rack.instance_mut(target, slot)),
        ParamTarget::Instrument { channel } => app
            .instrument_racks
            .get_mut(idx)
            .and_then(|rack| rack.instance_mut(channel)),
    };

    let mut open = true;
    egui::Window::new(format!("{} — {}", t!("mix.params_title"), panel.title))
        .id(egui::Id::new("mix_param_panel"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size([460.0, 520.0])
        .show(ctx, |ui| {
            let Some(instance) = instance else {
                ui.label(
                    egui::RichText::new(t!("mix.params_unloaded"))
                        .color(crate::theme::text_muted()),
                );
                return;
            };
            if instance.id() != panel.plugin_id {
                // 槽位被换成别的插件：重枚举参数。
                panel.plugin_id = instance.id().to_string();
                panel.refresh(instance);
            } else if instance.take_params_rescan() {
                panel.refresh(instance);
            }
            if panel.filter_dirty {
                panel.rebuild_filter();
            }

            ui.horizontal(|ui| {
                ui.label(t!("mix.search"));
                if ui.text_edit_singleline(&mut panel.search).changed() {
                    panel.filter_dirty = true;
                }
                let count = format!("{}/{}", panel.filtered.len(), panel.params.len());
                ui.label(
                    egui::RichText::new(count)
                        .size(crate::theme::SMALL_FONT)
                        .color(crate::theme::text_muted()),
                );
            });
            ui.separator();

            if panel.params.is_empty() {
                ui.label(
                    egui::RichText::new(t!("mix.no_params")).color(crate::theme::text_muted()),
                );
                return;
            }

            let row_h = 22.0;
            let list_h = ui.available_height();
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .max_height(list_h)
                .show_rows(ui, row_h, panel.filtered.len(), |ui, range| {
                    for &pi in &panel.filtered[range] {
                        param_row(
                            ui,
                            row_h,
                            &panel.params[pi],
                            instance,
                            &mut panel.editing,
                            &panel.queue,
                            &mut panel.wrote_params,
                        );
                    }
                });
        });
    if panel.wrote_params {
        // 参数变化写进插件 state，保存工程时随 InsertRef.state 持久化 → 标脏。
        app.workspace.documents[idx].mixer_mut();
        panel.wrote_params = false;
    }
    if open {
        app.mix.param_panel = Some(panel);
    }
}

/// 单行参数：名称（模块/名称）+ 滑块 + 插件格式化值。
fn param_row(
    ui: &mut egui::Ui,
    row_h: f32,
    p: &PluginParam,
    instance: &mut PluginInstance,
    editing: &mut HashMap<u32, f64>,
    queue: &ParamQueue,
    wrote_params: &mut bool,
) {
    // 拖动中优先本地值；否则读插件当前值；读不到退回默认值。
    let live = editing
        .get(&p.id)
        .copied()
        .or_else(|| instance.get_param_value(p.id))
        .unwrap_or(p.default);

    ui.horizontal(|ui| {
        let label = if p.module.is_empty() {
            p.name.clone()
        } else {
            format!("{}/{}", p.module, p.name)
        };
        ui.add_sized(
            [170.0, row_h],
            egui::Label::new(
                egui::RichText::new(label)
                    .size(crate::theme::SMALL_FONT)
                    .color(crate::theme::text_primary()),
            )
            .truncate(),
        );
        let mut value = live;
        let slider = ui.add_enabled(
            !p.read_only,
            egui::Slider::new(&mut value, p.min..=p.max).show_value(false),
        );
        if slider.changed() {
            editing.insert(p.id, value);
            queue.push(p.id, value);
            *wrote_params = true;
        }
        // 松手：丢弃本地值，下一帧从插件读回。
        if slider.drag_stopped() || (slider.lost_focus() && !slider.has_focus()) {
            editing.remove(&p.id);
        }
        let text = instance
            .value_to_text(p.id, live)
            .unwrap_or_else(|| format!("{live:.3}"));
        ui.add_sized(
            [90.0, row_h],
            egui::Label::new(
                egui::RichText::new(text)
                    .size(crate::theme::SMALL_FONT)
                    .monospace()
                    .color(crate::theme::text_secondary()),
            )
            .truncate(),
        );
    });
}
