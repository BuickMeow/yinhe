use crate::app::App;
use rust_i18n::t;

impl App {
    /// 通道 AR：显示通道行、自动化子行，并落地通道级交互。
    pub(in crate::app) fn show_channel_ar_view(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        arr_h: f32,
        is_playing: bool,
        follow_mode: &mut crate::view_interaction::FollowMode,
    ) {
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let mut request_pianoroll = false;
        let mut ca_drag_delta: Option<crate::channel_arrange::CaArRangeDelta> = None;
        let mut ca_picker: Option<usize> = None;
        let mut ca_notify = false;
        let mut ca_set_strips: Vec<(u8, yinhe_mixer::StripParams)> = Vec::new();
        let mut ca_set_audio_strips: Vec<(u16, yinhe_mixer::StripParams)> = Vec::new();
        let mut ca_menu_actions: Vec<crate::arrange::context_menu::TimelineMenuAction> = Vec::new();
        // 通道电平显示值（独立衰减；见 App::ca_channel_levels）。
        let ca_dt = ui.ctx().input(|i| i.stable_dt).min(0.1);
        let (ca_midi_levels, ca_audio_levels) = self.ca_channel_levels(idx, ca_dt);
        {
            let mut guard =
                crate::app::main_loop::ReplaceGuard::new(&mut self.workspace.documents[idx]);
            let cfg = crate::channel_arrange::CaArrangeCfg {
                is_playing,
                follow_mode,
                min_border_width: self.audio_settings.min_border_width,
                revision: guard.as_ref().data.revision,
            };
            crate::channel_arrange::show(
                ui,
                guard.as_mut(),
                crate::channel_arrange::CaArrangeLayout {
                    remaining: rect,
                    arr_h,
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
                &mut ca_menu_actions,
            );
            // guard dropped here
        }
        self.apply_timeline_menu_actions(ui.ctx(), ca_menu_actions);
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
            self.apply_node_strip(idx, yinhe_types::MixerChannelId::Midi(ch), params);
        }
        for (ach, params) in ca_set_audio_strips {
            self.note_mixer_edit(t!("undo.mix_param").as_ref());
            self.apply_node_strip(idx, yinhe_types::MixerChannelId::Audio(ach), params);
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
}
