//! 主输出表：底部 / 浮动共用的模块化电平、响度、频谱与立体声视图。

use std::collections::{HashMap, HashSet};

use eframe::egui;
use rust_i18n::t;
use yinhe_editor_core::audio_settings::{MeterModuleKind, MeterModuleSlot, default_meter_modules};

use crate::app::App;

mod loudness;
mod oscilloscope;
mod spectrogram;
mod spectrum;
mod stereometer;
mod vu;
mod waveform;

const MIN_PANEL_H: f32 = 160.0;
const DEFAULT_PANEL_H: f32 = 220.0;
const MODULE_HEADER_H: f32 = 22.0;
const MODULE_MIN_W: f32 = 128.0;
const MODULE_GAP: f32 = 2.0;
/// 末尾「+」和「更多」两列占用的宽度。
const TOOLBAR_W: f32 = 28.0 + 4.0 + 24.0;

pub(super) trait MeterModule {
    fn update(&mut self, samples: &[f32], sample_rate: u32, dt: f32);
    fn draw(&mut self, ui: &mut egui::Ui, rect: egui::Rect);
    fn reset(&mut self);
}

pub(crate) struct MeterPanelState {
    layout: Vec<MeterModuleSlot>,
    modules: HashMap<MeterModuleKind, Box<dyn MeterModule>>,
    samples: Vec<f32>,
    sample_rate: u32,
    last_generation: u64,
    was_playing: bool,
    /// 正在拖动排序的模块。
    dragging: Option<MeterModuleKind>,
    /// 本帧各模块矩形（下一帧用它计算拖拽插入位置）。
    last_rects: Vec<(MeterModuleKind, egui::Rect)>,
}

impl Default for MeterPanelState {
    fn default() -> Self {
        Self {
            layout: default_meter_modules(),
            modules: HashMap::new(),
            samples: Vec::new(),
            sample_rate: 48_000,
            last_generation: 0,
            was_playing: false,
            dragging: None,
            last_rects: Vec::new(),
        }
    }
}

impl MeterPanelState {
    pub(crate) fn layout(&self) -> &[MeterModuleSlot] {
        &self.layout
    }

    /// 从设置加载布局（启动时调用一次）。不合法/空的布局保持原样：
    /// 空布局表示用户主动移除了全部模块。
    pub(crate) fn load_layout(&mut self, layout: &[MeterModuleSlot]) {
        self.layout = layout.to_vec();
        self.modules.clear();
    }

    fn ensure_modules(&mut self, sample_rate: u32) {
        let kinds: HashSet<MeterModuleKind> = self.layout.iter().map(|s| s.kind).collect();
        self.modules.retain(|kind, _| kinds.contains(kind));
        for slot in &self.layout {
            self.modules
                .entry(slot.kind)
                .or_insert_with(|| new_module(slot.kind, sample_rate));
        }
    }

    fn recreate_all(&mut self, sample_rate: u32) {
        self.sample_rate = sample_rate;
        self.modules.clear();
        self.ensure_modules(sample_rate);
    }

    fn reset_all(&mut self) {
        for module in self.modules.values_mut() {
            module.reset();
        }
    }

    /// 每帧从音频引擎 drain 一次 Master 样本，再分发给所有模块。
    fn update_from_audio(&mut self, handle: Option<&yinhe_audio::CpalAudioHandle>, dt: f32) {
        let sample_rate = handle.map(|h| h.sample_rate).unwrap_or(self.sample_rate);
        if sample_rate != self.sample_rate {
            self.recreate_all(sample_rate);
            self.last_generation = 0;
            self.was_playing = false;
        }
        let playing = handle.map(|h| h.handle.is_playing()).unwrap_or(false);
        let generation = handle.map(|h| h.handle.reset_generation()).unwrap_or(0);
        if generation != self.last_generation {
            self.reset_all();
            self.last_generation = generation;
        }
        // 停止→播放 = 新的一次播放；暂停→继续不重置。
        if playing && !self.was_playing {
            self.reset_all();
        }
        self.was_playing = playing;

        self.samples.clear();
        if let Some(handle) = handle {
            handle.handle.master_samples_drain(&mut self.samples);
        }
        self.ensure_modules(self.sample_rate);
        for module in self.modules.values_mut() {
            module.update(&self.samples, self.sample_rate, dt);
        }
    }
}

/// 底部停靠入口：Panel::bottom + 可拖高度。必须在 compute_layout 之前调用。
pub(crate) fn show(app: &mut App, ui: &mut egui::Ui) {
    if !app.show_meter_panel {
        return;
    }
    let max_h = (ui.available_height() - 160.0).max(MIN_PANEL_H);
    let h = app.meter_height.clamp(MIN_PANEL_H, max_h);
    let mut layout_changed = false;
    egui::Panel::bottom("bottom_meter")
        .exact_size(h)
        .resizable(false)
        .show_separator_line(false)
        .frame(egui::Frame {
            fill: crate::theme::app_bg(),
            inner_margin: egui::Margin::ZERO,
            ..Default::default()
        })
        .show(ui, |ui| {
            let panel_rect = ui.max_rect();
            let handle_rect = egui::Rect::from_min_size(
                panel_rect.min,
                egui::vec2(panel_rect.width(), crate::theme::SPLIT_HANDLE_W),
            );
            let handle =
                crate::widgets::split_handle::horizontal(ui, "__meter_split__", handle_rect);
            if handle.dragged() {
                app.meter_height =
                    (app.meter_height - handle.drag_delta().y).clamp(MIN_PANEL_H, max_h);
            }
            if handle.double_clicked() {
                app.meter_height = DEFAULT_PANEL_H;
            }
            if handle.drag_stopped() || handle.double_clicked() {
                app.layout_needs_save = true;
            }
            crate::widgets::hint::hover(ui.ctx(), &handle, t!("hint.meter.split"));

            let content = egui::Rect::from_min_max(
                egui::pos2(panel_rect.min.x + 4.0, handle_rect.max.y + 2.0),
                egui::pos2(panel_rect.max.x - 4.0, panel_rect.max.y - 4.0),
            );
            if ui
                .input(|i| i.pointer.hover_pos())
                .is_some_and(|p| content.contains(p))
            {
                crate::widgets::hint::set_region(ui.ctx(), t!("hint.panel.meter"));
            }
            layout_changed = show_content(app, ui, content);
        });
    if layout_changed {
        app.layout_needs_save = true;
    }
}

/// 在给定矩形内渲染模块条；底部停靠与浮动窗口共用。
pub(crate) fn show_content(app: &mut App, ui: &mut egui::Ui, rect: egui::Rect) -> bool {
    let dt = ui.ctx().input(|i| i.stable_dt).min(0.1);
    let handle = app.audio_state.handle.as_ref();
    app.meter_panel.update_from_audio(handle, dt);

    let mut changed = false;
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        changed |= app.meter_panel.show_strip(ui, rect);
    });
    if changed {
        app.layout_needs_save = true;
    }
    changed
}

impl MeterPanelState {
    fn show_strip(&mut self, ui: &mut egui::Ui, rect: egui::Rect) -> bool {
        let mut changed = false;
        let height = rect.height();
        if height <= MODULE_HEADER_H {
            return false;
        }
        if self.layout.is_empty() {
            let add_rect = egui::Rect::from_center_size(rect.center(), egui::vec2(120.0, 28.0));
            let resp = ui.interact(
                add_rect,
                ui.scope_id().with("meter_add_empty"),
                egui::Sense::click(),
            );
            ui.painter().text(
                add_rect.center(),
                egui::Align2::CENTER_CENTER,
                t!("meter.add").to_string(),
                egui::FontId::proportional(crate::theme::SMALL_FONT),
                crate::theme::text_muted(),
            );
            crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.meter.add"));
            if resp.clicked() {
                self.open_add_menu(ui, &resp);
            }
            return false;
        }

        // 右侧固定工具栏：不放进横向滚动区，保证「+」和「更多」永远可点。
        let toolbar_rect = egui::Rect::from_min_max(
            egui::pos2((rect.max.x - TOOLBAR_W).max(rect.min.x), rect.min.y),
            rect.max,
        );
        let modules_rect = egui::Rect::from_min_max(
            rect.min,
            egui::pos2(toolbar_rect.min.x - MODULE_GAP, rect.max.y),
        );
        let available = modules_rect.width().max(1.0);
        let weights: f32 = self.layout.iter().map(|s| s.weight.max(0.01)).sum();
        let natural: Vec<f32> = self
            .layout
            .iter()
            .map(|s| available * s.weight.max(0.01) / weights.max(0.01))
            .collect();
        let widths: Vec<f32> = natural.iter().map(|w| w.max(MODULE_MIN_W)).collect();
        let total_w: f32 = widths.iter().sum::<f32>() + MODULE_GAP * (self.layout.len() - 1) as f32;
        let content_w = total_w.max(available);

        ui.scope_builder(egui::UiBuilder::new().max_rect(modules_rect), |ui| {
            egui::ScrollArea::horizontal()
                .id_salt("meter_modules_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let (content_rect, _) =
                        ui.allocate_exact_size(egui::vec2(content_w, height), egui::Sense::hover());
                    let mut x = content_rect.min.x;
                    let mut rects = Vec::with_capacity(self.layout.len());
                    for (idx, slot) in self.layout.iter().enumerate() {
                        let w = widths[idx];
                        let module_rect = egui::Rect::from_min_size(
                            egui::pos2(x, content_rect.min.y),
                            egui::vec2(w, height),
                        );
                        rects.push((slot.kind, module_rect));
                        x += w + MODULE_GAP;
                    }
                    self.last_rects = rects.clone();

                    // 先处理拖动排序，再画模块：拖动时实时换位，松手落盘。
                    if let Some(dragging) = self.dragging {
                        if let Some(pointer) = ui.input(|i| i.pointer.interact_pos()) {
                            let target = rects
                                .iter()
                                .position(|(_, r)| pointer.x < r.center().x)
                                .unwrap_or(rects.len().saturating_sub(1));
                            if let Some(from) = self.layout.iter().position(|s| s.kind == dragging)
                                && from != target
                                && target < self.layout.len()
                            {
                                let slot = self.layout.remove(from);
                                self.layout.insert(target, slot);
                                changed = true;
                            }
                        }
                        if ui.input(|i| i.pointer.any_released()) {
                            self.dragging = None;
                            changed = true;
                        }
                    }

                    for idx in 0..self.layout.len() {
                        let slot = self.layout[idx];
                        let Some((_, module_rect)) = rects.get(idx).copied() else {
                            continue;
                        };
                        let (next_changed, remove, drag) =
                            self.draw_module(ui, slot.kind, module_rect, idx);
                        changed |= next_changed;
                        if let Some(kind) = drag {
                            self.dragging = Some(kind);
                        }
                        if remove {
                            self.layout.retain(|s| s.kind != slot.kind);
                            self.modules.remove(&slot.kind);
                            changed = true;
                            break;
                        }
                    }

                    // 拖动插入指示线。
                    if let Some(dragging) = self.dragging
                        && let Some(pointer) = ui.input(|i| i.pointer.interact_pos())
                        && let Some((_, r)) = rects
                            .iter()
                            .find(|(kind, r)| *kind == dragging || pointer.x < r.center().x)
                    {
                        let line_x = r.min.x - MODULE_GAP * 0.5;
                        ui.painter().vline(
                            line_x,
                            r.y_range(),
                            egui::Stroke::new(2.0, crate::theme::accent_active()),
                        );
                    }
                });
        });

        // 「+」添加模块。
        let add_rect = egui::Rect::from_center_size(
            egui::pos2(toolbar_rect.center().x, toolbar_rect.center().y - 14.0),
            egui::vec2(24.0, 24.0),
        );
        let add_resp = ui.interact(
            add_rect,
            ui.scope_id().with("meter_add_end"),
            egui::Sense::click(),
        );
        if add_resp.hovered() {
            ui.painter().rect_filled(
                add_rect,
                3.0,
                crate::theme::hover_color(crate::theme::line_fg()),
            );
        }
        icon_at(
            ui,
            add_rect,
            egui_material_icons::icons::ICON_ADD,
            if add_resp.hovered() {
                crate::theme::text_primary()
            } else {
                crate::theme::text_muted()
            },
        );
        crate::widgets::hint::hover(ui.ctx(), &add_resp, t!("hint.meter.add"));
        if add_resp.clicked() {
            self.open_add_menu(ui, &add_resp);
        }

        // 「更多」：恢复默认布局 / 重置全部数据。
        let menu_rect = egui::Rect::from_center_size(
            egui::pos2(toolbar_rect.center().x, toolbar_rect.center().y + 14.0),
            egui::vec2(24.0, 24.0),
        );
        if let Some(changed_layout) = self.draw_reset_menu(ui, menu_rect) {
            changed |= changed_layout;
        }
        changed
    }

    fn draw_module(
        &mut self,
        ui: &mut egui::Ui,
        kind: MeterModuleKind,
        rect: egui::Rect,
        _idx: usize,
    ) -> (bool, bool, Option<MeterModuleKind>) {
        let mut remove = false;
        let mut drag = None;
        let header = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), MODULE_HEADER_H));
        if ui
            .input(|i| i.pointer.hover_pos())
            .is_some_and(|p| rect.contains(p))
        {
            crate::widgets::hint::set_region(ui.ctx(), t!("hint.panel.meter"));
        }

        let drag_rect = egui::Rect::from_min_size(header.min, egui::vec2(22.0, MODULE_HEADER_H));
        let drag_resp = ui.interact(
            drag_rect,
            ui.scope_id().with((kind, "meter_drag")),
            egui::Sense::click_and_drag(),
        );
        if drag_resp.dragged() {
            drag = Some(kind);
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        icon_at(
            ui,
            drag_rect,
            egui_material_icons::icons::ICON_DRAG_INDICATOR,
            if drag_resp.hovered() {
                crate::theme::text_primary()
            } else {
                crate::theme::text_muted()
            },
        );
        crate::widgets::hint::hover(ui.ctx(), &drag_resp, t!("hint.meter.drag"));

        let title = module_title(kind);
        ui.painter().text(
            egui::pos2(header.min.x + 24.0, header.center().y),
            egui::Align2::LEFT_CENTER,
            title,
            egui::FontId::proportional(crate::theme::SMALL_FONT),
            crate::theme::text_secondary(),
        );

        let close_rect = egui::Rect::from_min_size(
            egui::pos2(header.max.x - 20.0, header.min.y),
            egui::vec2(20.0, MODULE_HEADER_H),
        );
        let close_resp = ui.interact(
            close_rect,
            ui.scope_id().with((kind, "meter_close")),
            egui::Sense::click(),
        );
        if close_resp.hovered() {
            ui.painter().rect_filled(
                close_rect.shrink(3.0),
                3.0,
                crate::theme::hover_color(crate::theme::line_fg()),
            );
            crate::widgets::hint::hover(ui.ctx(), &close_resp, t!("hint.meter.remove"));
        }
        icon_at(
            ui,
            close_rect,
            egui_material_icons::icons::ICON_CLOSE,
            if close_resp.hovered() {
                crate::theme::text_primary()
            } else {
                crate::theme::text_muted()
            },
        );
        if close_resp.clicked() {
            remove = true;
        }

        let body = egui::Rect::from_min_max(egui::pos2(rect.min.x, header.max.y), rect.max);
        if let Some(module) = self.modules.get_mut(&kind) {
            module.draw(ui, body);
        }
        (false, remove, drag)
    }

    fn open_add_menu(&mut self, _ui: &mut egui::Ui, resp: &egui::Response) {
        egui::Popup::from_toggle_button_response(resp)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.set_min_width(150.0);
                for kind in MeterModuleKind::ALL {
                    if self.layout.iter().any(|s| s.kind == kind) {
                        continue;
                    }
                    if ui.button(module_title(kind)).clicked() {
                        self.layout.push(MeterModuleSlot { kind, weight: 1.0 });
                        self.modules
                            .insert(kind, new_module(kind, self.sample_rate));
                        egui::Popup::close_all(ui.ctx());
                    }
                }
            });
    }

    fn draw_reset_menu(&mut self, ui: &mut egui::Ui, rect: egui::Rect) -> Option<bool> {
        let resp = ui.interact(rect, ui.scope_id().with("meter_more"), egui::Sense::click());
        icon_at(
            ui,
            rect,
            egui_material_icons::icons::ICON_MORE_VERT,
            if resp.hovered() {
                crate::theme::text_primary()
            } else {
                crate::theme::text_muted()
            },
        );
        crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.meter.more"));
        let mut changed = false;
        egui::Popup::from_toggle_button_response(&resp)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
            .show(|ui| {
                if ui.button(t!("meter.reset_layout").to_string()).clicked() {
                    self.layout = default_meter_modules();
                    self.modules.clear();
                    self.ensure_modules(self.sample_rate);
                    changed = true;
                }
                if ui.button(t!("meter.reset_all").to_string()).clicked() {
                    self.reset_all();
                }
            });
        changed.then_some(true)
    }
}

fn module_title(kind: MeterModuleKind) -> String {
    match kind {
        MeterModuleKind::Loudness => t!("meter.module.loudness").to_string(),
        MeterModuleKind::Oscilloscope => t!("meter.module.oscilloscope").to_string(),
        MeterModuleKind::Spectrum => t!("meter.module.spectrum").to_string(),
        MeterModuleKind::Spectrogram => t!("meter.module.spectrogram").to_string(),
        MeterModuleKind::Stereometer => t!("meter.module.stereometer").to_string(),
        MeterModuleKind::Vu => t!("meter.module.vu").to_string(),
        MeterModuleKind::Waveform => t!("meter.module.waveform").to_string(),
    }
}

fn new_module(kind: MeterModuleKind, sample_rate: u32) -> Box<dyn MeterModule> {
    match kind {
        MeterModuleKind::Loudness => Box::new(loudness::LoudnessModule::new(sample_rate)),
        MeterModuleKind::Oscilloscope => {
            Box::new(oscilloscope::OscilloscopeModule::new(sample_rate))
        }
        MeterModuleKind::Spectrum => Box::new(spectrum::SpectrumModule::new(sample_rate)),
        MeterModuleKind::Spectrogram => Box::new(spectrogram::SpectrogramModule::new(sample_rate)),
        MeterModuleKind::Stereometer => Box::new(stereometer::StereometerModule::new(sample_rate)),
        MeterModuleKind::Vu => Box::new(vu::VuModule::new(sample_rate)),
        MeterModuleKind::Waveform => Box::new(waveform::WaveformModule::new(sample_rate)),
    }
}

fn icon_at(
    ui: &egui::Ui,
    rect: egui::Rect,
    icon: egui_material_icons::MaterialIcon,
    color: egui::Color32,
) {
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icon.codepoint,
        egui::FontId::new(crate::theme::ICON_FONT_SM, icon.font_family()),
        color,
    );
}

pub(super) fn plot_bg(ui: &egui::Ui, rect: egui::Rect) {
    ui.painter()
        .rect_filled(rect, 0.0, crate::theme::track_bg());
}

pub(super) fn grid(painter: &egui::Painter, rect: egui::Rect, x_lines: usize, y_lines: usize) {
    let stroke = egui::Stroke::new(1.0, crate::theme::text_muted().gamma_multiply(0.5));
    for i in 1..x_lines {
        let x = rect.min.x + rect.width() * i as f32 / x_lines as f32;
        painter.vline(x, rect.y_range(), stroke);
    }
    for i in 1..y_lines {
        let y = rect.min.y + rect.height() * i as f32 / y_lines as f32;
        painter.hline(rect.x_range(), y, stroke);
    }
}

pub(super) fn db_text(db: f32) -> String {
    if !db.is_finite() || db <= -70.0 {
        "—".to_string()
    } else {
        format!("{db:.1}")
    }
}

pub(super) fn lufs_text(v: f32) -> String {
    if !v.is_finite() || v <= -70.0 {
        "—".to_string()
    } else {
        format!("{v:.1}")
    }
}

pub(super) fn small_label(
    ui: &egui::Ui,
    pos: egui::Pos2,
    align: egui::Align2,
    text: impl ToString,
    color: egui::Color32,
) {
    ui.painter().text(
        pos,
        align,
        text.to_string(),
        egui::FontId::proportional(crate::theme::SMALL_LABEL_FONT),
        color,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_strip(state: &mut MeterPanelState) {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 220.0),
            )),
            ..Default::default()
        };
        ctx.run_ui(input, |ui| {
            let rect = ui.max_rect();
            state.update_from_audio(None, 1.0 / 60.0);
            state.show_strip(ui, rect);
        })
        .drop_without_applying_deltas();
    }

    #[test]
    fn default_layout_draws_without_panic() {
        let mut state = MeterPanelState::default();
        assert_eq!(state.layout().len(), default_meter_modules().len());
        run_strip(&mut state);
    }

    #[test]
    fn empty_layout_draws_add_state() {
        let mut state = MeterPanelState::default();
        state.load_layout(&[]);
        run_strip(&mut state);
        assert!(state.layout().is_empty());
    }

    #[test]
    fn load_layout_preserves_order() {
        let mut state = MeterPanelState::default();
        state.load_layout(&[
            MeterModuleSlot {
                kind: MeterModuleKind::Spectrum,
                weight: 2.0,
            },
            MeterModuleSlot {
                kind: MeterModuleKind::Vu,
                weight: 1.0,
            },
        ]);
        assert_eq!(state.layout()[0].kind, MeterModuleKind::Spectrum);
        assert_eq!(state.layout()[1].kind, MeterModuleKind::Vu);
        run_strip(&mut state);
    }
}
