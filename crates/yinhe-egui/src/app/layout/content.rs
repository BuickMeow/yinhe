#![allow(unused_imports)]
use crate::app::App;
use crate::app::floating;
use crate::arrange;
use crate::piano_view;
use crate::right_panel::automation_undo::push_automation_actions;
use crate::right_panel::info_panel::selection::selected_am_events;
use eframe::egui;
use rust_i18n::t;
use yinhe_editor_core::audio_settings::LayoutSettings;
use yinhe_editor_core::batch_ops::summarize_selected;
use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_types::time_format::{format_tick_bar_beat_with_time_sig, measure_ticks};
use yinhe_types::{AnchorSelRect, NoteSource, VelocityEdit};

use super::{LayoutInfo, sel_hint::SelHintInfo};

mod interaction;
mod views;

use interaction::*;
use views::*;

impl App {
    /// Show the main content area: arrangement view, pianoroll, and note drag handling.
    pub(in crate::app) fn show_main_content(&mut self, ui: &mut egui::Ui, layout: &LayoutInfo) {
        use crate::chrome::mode_bar::ViewMode;
        let is_mix = self.view_mode == ViewMode::Mix;
        let has_mixer = is_mix && self.show_mixer_panel;
        let has_piano = self
            .view_mode
            .show_pianoroll(self.show_pianoroll_in_arrange)
            && !has_mixer;

        // MIX 无工程：只铺背景（原行为）。
        // 主区域绝不能是全透明（clear_color 为全透明，透明模式下会露出桌面）。
        if is_mix && self.workspace.active_doc.is_none() {
            ui.painter()
                .rect_filled(layout.remaining, 0.0, crate::theme::track_bg());
            return;
        }
        // EDIT 之外一定有一个主视图（轨道 AR / 通道 AR / 全屏 PR）。
        let has_view = self.view_mode.show_transport() || has_piano || is_mix;
        if !has_view {
            ui.painter()
                .rect_filled(layout.remaining, 0.0, crate::theme::track_bg());
            return;
        }
        let Some(idx) = self.workspace.active_doc else {
            // 有视图控件（AR/PR 模式）但未打开工程：显示欢迎内容而非透明。
            self.show_welcome_content(ui, layout);
            return;
        };
        // 滚动条密度预览：预算内增量重扫脏 key（跟随 track_visible 显隐）。
        self.workspace.documents[idx].sync_density(yinhe_editor_core::density::SYNC_BUDGET);
        // 三视图选框互斥：先于任何渲染执行，保证同一时刻只有一个视图拥有选框。
        self.enforce_sel_rect_exclusivity(idx);
        // 聚焦视图（指针悬停决定；Ctrl+A 等快捷键按它路由）。
        self.update_view_focus(ui, layout);

        let is_playing = self
            .audio_state
            .handle
            .as_ref()
            .map(|a| a.handle.is_playing())
            .unwrap_or(false);
        let mut follow_mode = self.follow_mode;

        // 讲解行选框统计（存在选框时 Some；PR/AR/AM 互斥，单一来源）
        let sel_hint = Self::compute_sel_hint(&self.workspace.documents[idx]);

        // Arrangement view（工程走带；浮动窗口打开时主窗口让位）
        if self.view_mode.show_transport() {
            if self.float_arrange {
                Self::paint_float_yielded(
                    ui,
                    egui::Rect::from_min_max(
                        layout.remaining.min,
                        egui::pos2(
                            layout.remaining.max.x,
                            layout.remaining.min.y + layout.arr_h,
                        ),
                    ),
                );
            } else {
                self.show_arrange_view(ui, layout.remaining, layout.arr_h, &mut follow_mode, false);
            }
        }

        // ── 通道 AR（MIX 主区域）──
        if is_mix {
            let mut request_pianoroll = false;
            let mut ca_drag_delta: Option<crate::channel_arrange::CaArRangeDelta> = None;
            let mut ca_picker: Option<usize> = None;
            let mut ca_notify = false;
            let mut ca_set_strips: Vec<(u8, yinhe_mixer::StripParams)> = Vec::new();
            let mut ca_set_audio_strips: Vec<(u16, yinhe_mixer::StripParams)> = Vec::new();
            // 通道电平显示值（独立衰减；见 App::ca_channel_levels）。
            let ca_dt = ui.ctx().input(|i| i.stable_dt).min(0.1);
            let (ca_midi_levels, ca_audio_levels) = self.ca_channel_levels(idx, ca_dt);
            {
                let mut guard =
                    crate::app::main_loop::ReplaceGuard::new(&mut self.workspace.documents[idx]);
                let cfg = crate::channel_arrange::CaArrangeCfg {
                    is_playing,
                    follow_mode: &mut follow_mode,
                    min_border_width: self.audio_settings.min_border_width,
                    revision: guard.as_ref().data.revision,
                };
                crate::channel_arrange::show(
                    ui,
                    guard.as_mut(),
                    crate::channel_arrange::CaArrangeLayout {
                        remaining: layout.remaining,
                        arr_h: layout.arr_h,
                        panel_width: &mut self.transport_panel_width,
                        drag_ended: &mut self.layout_needs_save,
                    },
                    &mut self.arr_renderer,
                    &mut self.arr_render_ctx,
                    cfg,
                    self.audio_state.handle.as_ref(),
                    &self.audio_library,
                    &mut request_pianoroll,
                    &mut self.track_selection_anchor,
                    &mut ca_drag_delta,
                    &mut self.info_content,
                    &mut self.right_tab,
                    &mut ca_notify,
                    &mut ca_picker,
                    &self.mix.track_display,
                    &ca_midi_levels,
                    &ca_audio_levels,
                    &mut ca_set_strips,
                    &mut ca_set_audio_strips,
                );
                // guard dropped here
            }
            if ca_notify {
                self.notify_audio_model_changed();
            }
            if let Some(track_idx) = ca_picker {
                self.open_automation_picker(ui.ctx(), track_idx);
            }
            if let Some((dt, dtr, alt)) = ca_drag_delta {
                self.handle_arr_drag(dt, dtr, alt);
            }
            // 通道行 M/S（混音台通道）：与混音台同一 undo 合并路径。
            for (ch, params) in ca_set_strips {
                self.note_mixer_edit(t!("undo.mix_param").as_ref());
                self.apply_strip(idx, ch, params);
            }
            for (ach, params) in ca_set_audio_strips {
                self.note_mixer_edit(t!("undo.mix_param").as_ref());
                self.apply_audio_strip(idx, ach, params);
            }
            // 通道电平动画：播放中或衰减未归零时保持约 30fps 重绘。
            let ca_any_level = ca_midi_levels
                .iter()
                .any(|&((l, r), _)| l > 0.001 || r > 0.001)
                || ca_audio_levels
                    .iter()
                    .any(|&((l, r), _)| l > 0.001 || r > 0.001);
            if is_playing || ca_any_level {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(33));
            }
            if request_pianoroll {
                crate::chrome::mode_bar::set_pianoroll_visible(
                    &mut self.show_pianoroll_in_arrange,
                    &mut self.show_bottom_dock,
                    &mut self.show_mixer_panel,
                    &mut self.show_meter_panel,
                    true,
                );
            }
        }
        // 方案 A：音轨结构变化、AR 橡皮擦、AR 拖动、自动化通知、
        // 参数自动化选择窗口等 AR 事件落地，全部在 `show_arrange_view` 内处理
        // （guard 释放后执行，见该方法）。

        // ── 上方主视图与底部面板（钢琴卷帘/混音台）之间的分割条 ──
        // ARRANGE：AR/PR（arr_split）；MIX：通道 AR/底部面板（mix_height）。
        if layout.arr_h > 0.0 && (has_piano || has_mixer) {
            let h_split_rect = egui::Rect::from_min_max(
                egui::pos2(
                    layout.remaining.min.x,
                    layout.remaining.min.y + layout.arr_h,
                ),
                egui::pos2(
                    layout.remaining.max.x,
                    layout.remaining.min.y + layout.arr_h + crate::theme::SPLIT_GAP,
                ),
            );
            let h_split_resp =
                crate::widgets::split_handle::horizontal(ui, "__h_split__", h_split_rect);
            if h_split_resp.dragged() {
                let total_y = layout.remaining.size().y;
                let delta = h_split_resp.drag_delta().y;
                if has_mixer {
                    // 底部混音台高度（px）：向上拖（delta < 0）变高；拖拽后转为手动高度。
                    self.mix_height_auto = false;
                    let max_h = (total_y - crate::theme::MIN_ARR_HEIGHT - crate::theme::SPLIT_GAP)
                        .max(crate::theme::MIN_ARR_HEIGHT);
                    self.mix_height = (total_y - (layout.arr_h + delta) - crate::theme::SPLIT_GAP)
                        .clamp(crate::theme::MIN_ARR_HEIGHT, max_h);
                } else {
                    let ratio = ((layout.arr_h + delta) / total_y)
                        .clamp(crate::theme::SPLIT_CLAMP_MIN, crate::theme::SPLIT_CLAMP_MAX);
                    self.arr_split = ratio;
                }
            }
            if h_split_resp.double_clicked() {
                // 双击分割线 → 还原默认分割
                if has_mixer {
                    // 混音台恢复「随总线数自动高度」。
                    self.mix_height_auto = true;
                } else {
                    self.arr_split = LayoutSettings::default().arr_split;
                }
            }
            if h_split_resp.drag_stopped() || h_split_resp.double_clicked() {
                self.layout_needs_save = true;
            }
        }

        // Pianoroll area（MIX 下与混音台/设备链三选一显示）。
        if has_piano {
            if self.view_mode == ViewMode::Edit && self.float_pianoroll {
                // EDIT 主视图 = 全屏 PR：浮动窗口打开时主窗口让位（GPU 视图单实例）。
                Self::paint_float_yielded(
                    ui,
                    egui::Rect::from_min_max(
                        egui::pos2(layout.remaining.min.x, layout.bottom_y),
                        layout.remaining.max,
                    ),
                );
            } else {
                self.show_pianoroll_split(
                    ui,
                    layout,
                    idx,
                    is_playing,
                    &mut follow_mode,
                    sel_hint.as_ref(),
                );
            }
        }

        // MIX 混音台面板（底部；与钢琴卷帘/设备链三选一，默认打开）。
        if has_mixer {
            let rect = egui::Rect::from_min_max(
                egui::pos2(layout.remaining.min.x, layout.bottom_y),
                layout.remaining.max,
            );
            crate::mix::show(self, ui, rect);
        }

        self.follow_mode = follow_mode;
    }

    /// 主视图已浮动为独立窗口时的占位提示（视图让位）。
    fn paint_float_yielded(ui: &egui::Ui, rect: egui::Rect) {
        ui.painter()
            .rect_filled(rect, 0.0, crate::theme::track_bg());
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            t!("float.yielded"),
            egui::FontId::proportional(crate::theme::BODY_FONT),
            crate::theme::text_muted(),
        );
    }

    /// Show right panel, and request repaint if playing.
    pub(in crate::app) fn show_panels_and_overlays(
        &mut self,
        ui: &mut egui::Ui,
        layout: &LayoutInfo,
    ) {
        // Right panel（rect 来自 Panel::right 占位：整条，dock 不截断）
        if self.right_tab.is_some() {
            let right_rect = layout.right_panel_rect;
            let doc = self
                .workspace
                .active_doc
                .and_then(|idx| self.workspace.documents.get_mut(idx));
            let (changed, jump_request, history_jump, width_drag_ended) = crate::right_panel::show(
                ui,
                right_rect,
                &mut self.right_panel_width,
                &mut self.right_tab,
                &mut self.right_panel_layout,
                self.view_mode == crate::chrome::mode_bar::ViewMode::Mix,
                doc,
                self.audio_state.handle.as_ref(),
                &mut self.event_browser_state,
                &mut self.info_content,
                self.automation_drag_ghost,
            );
            if changed {
                // 右栏改动（改 port/channel、图层拖动排序）→ 统一走 notify：
                // 通道集合真的变了才 teardown 重建，重排等只 reload_notes。
                self.notify_audio_model_changed();
            }
            if width_drag_ended {
                self.layout_needs_save = true;
            }
            if let Some(req) = jump_request {
                self.handle_jump_request(req, layout);
            }
            if let Some(target) = history_jump {
                self.jump_history_to(target);
            }
        }

        // Request repaint during playback (or while waiting for audio thread to start)
        let is_audio_playing = self
            .audio_state
            .handle
            .as_ref()
            .map(|a| a.handle.is_playing())
            .unwrap_or(false);
        // macOS: 播放时阻止 App Nap（防止系统降低定时器精度导致播放卡顿），
        // 仅在播放状态翻转时调用平台 API。
        let playing = is_audio_playing || self.audio_state.pending_playback;
        if playing != self.app_nap_active {
            self.app_nap_active = playing;
            crate::platform::set_app_nap_enabled(playing);
        }
        // macOS: 窗口被完全遮挡时暂停动画重绘省 CPU/电（音频在独立线程继续播）。
        // 恢复可见时 Occluded(false) 事件会触发一次重绘，playhead 按 anchor+Instant 重算，不会跳帧丢位置。
        let occluded = ui.ctx().input(|i| i.viewport().occluded == Some(true));
        if playing && !occluded {
            ui.ctx().request_repaint();
        }
    }

    /// 帧末：把当前布局值同步进设置（内存），拖拽结束/切换帧写盘一次。
    /// 拖拽过程中值每帧变化但不置标志，避免拖拽时每帧刷盘。
    pub(in crate::app) fn sync_layout_settings(&mut self) {
        let l = &mut self.audio_settings.layout;
        l.right_panel_width = self.right_panel_width;
        l.arr_split = self.arr_split;
        l.transport_panel_width = self.transport_panel_width;
        l.show_pianoroll_in_arrange = self.show_pianoroll_in_arrange;
        l.show_bottom_dock = self.show_bottom_dock;
        l.bottom_dock_height = self.bottom_dock_height;
        l.show_mixer_panel = self.show_mixer_panel;
        l.mix_height = self.mix_height;
        l.show_meter_panel = self.show_meter_panel;
        l.meter_height = self.meter_height;
        l.meter_modules = self.meter_panel.layout().to_vec();
        l.mix_height_auto = self.mix_height_auto;
        l.float_pianoroll = yinhe_editor_core::audio_settings::FloatPanelState {
            floating: self.float_pianoroll,
            rect: self.float_pianoroll_rect.map(floating::rect_to_arr),
        };
        l.float_bottom_dock = yinhe_editor_core::audio_settings::FloatPanelState {
            floating: self.float_bottom_dock,
            rect: self.float_bottom_dock_rect.map(floating::rect_to_arr),
        };
        l.float_mixer = yinhe_editor_core::audio_settings::FloatPanelState {
            floating: self.float_mixer,
            rect: self.float_mixer_rect.map(floating::rect_to_arr),
        };
        l.float_arrange = yinhe_editor_core::audio_settings::FloatPanelState {
            floating: self.float_arrange,
            rect: self.float_arrange_rect.map(floating::rect_to_arr),
        };
        l.float_meter = yinhe_editor_core::audio_settings::FloatPanelState {
            floating: self.float_meter,
            rect: self.float_meter_rect.map(floating::rect_to_arr),
        };
        l.right_panel_layout = self.right_panel_layout.clone();
        if self.layout_needs_save {
            self.audio_settings.save();
            self.layout_needs_save = false;
        }
    }
}
