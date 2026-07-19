use crate::app::App;
use yinhe_editor_core::progress;

impl App {
    /// Notify the audio engine that the active document's model has changed (full
    /// rebuild: cc_events + audible_notes + chase). Use for automation edits,
    /// undo / redo, or any edit that may have touched automation lanes.
    ///
    /// 若 channel 激活状态翻转（首/末发声音符添加/删除，或 automation/PC 增删），
    /// 自动 teardown 引擎——`ChannelLayout` 创建后不可变，必须重建才能让新通道
    /// 被 dispatch。下一帧 `rebuild_audio_if_needed` 会用新 model 重新 spawn。
    pub(crate) fn notify_audio_model_changed(&mut self) {
        let Some(idx) = self.active_doc else { return };
        if self.audio_state.handle.is_none() { return; }
        if self.channel_layout_flipped_for_doc(idx) {
            self.teardown_audio();
        } else if let Some(audio) = &self.audio_state.handle {
            audio.reload_notes(self.documents[idx].data.model.clone());
        }
    }

    /// Notify the audio engine that only notes have changed (no automation, no
    /// chase). Cheaper than `notify_audio_model_changed` — skips the expensive
    /// `flatten_automation_to_cc_events` rebuild and the linear chase scan.
    /// Use for pure note edits (move/drag/add/delete/paste/duplicate/transpose).
    ///
    /// 若 channel 激活状态翻转（首/末发声音符添加/删除），自动 teardown 引擎
    /// 并下一帧重建——同 `notify_audio_model_changed`。
    pub(crate) fn notify_notes_changed(&mut self) {
        let Some(idx) = self.active_doc else { return };
        if self.audio_state.handle.is_none() { return; }
        if self.channel_layout_flipped_for_doc(idx) {
            self.teardown_audio();
        } else if let Some(audio) = &self.audio_state.handle {
            audio.update_notes(self.documents[idx].data.model.clone());
        }
    }

    /// 检测 doc idx 的当前 model 是否与引擎持有的 `ChannelLayout` 在激活状态上有差异。
    ///
    /// 返回 true 表示有 channel 的激活状态翻转了（0→1 或 1→0），必须 teardown。
    /// 返回 false 表示激活状态没变（如已激活 channel 加/删非末音符），可走便宜路径。
    ///
    /// 若引擎绑定的 doc 与 idx 不一致（tab 切换后的 1 帧延迟），也返回 true
    /// ——必须 teardown 重建以绑定到新 doc。
    fn channel_layout_flipped_for_doc(&self, idx: usize) -> bool {
        let Some(layout) = &self.audio_state.last_channel_layout else {
            return true; // 引擎未 spawn 过，让 rebuild 处理
        };
        if self.audio_state.active_doc != Some(idx) {
            return true; // 绑定的 doc 不一致，必须重建
        }
        let model = &self.documents[idx].data.model;
        layout.differs_from_counts(
            &model.channel_note_count,
            &model.channel_ctrl_count,
        )
    }

    /// Resolve the merged SF configuration for the given document.
    ///
    /// Returns a list of `(port, paths)` for every port the MIDI uses.
    pub(crate) fn resolve_sf_config(&self, doc: &yinhe_editor_core::document::Document) -> Vec<(u8, Vec<String>)> {
        let layout = yinhe_audio::channels_for_model(&doc.data.model);
        let num_ports = (layout.num_channels().div_ceil(16) as u8).max(1);
        let global = &self.audio_settings.global_sf_config;
        let project = &doc.edit.project_sf;

        let mut result: Vec<(u8, Vec<String>)> = Vec::new();

        for port in 0..num_ports {
            if global.global_enabled {
                // ── Global mode: all ports share ports[0] ──
                let paths: Vec<String> = global.ports[0]
                    .iter()
                    .filter(|e| e.enabled)
                    .map(|e| e.path.clone())
                    .collect();
                if !paths.is_empty() {
                    result.push((port, paths));
                    continue;
                }
            } else {
                // ── Project mode: per-port from overrides ──
                if let Some((_, entries)) = project.overrides.iter().find(|(p, _)| *p == port) {
                    let paths: Vec<String> = entries
                        .iter()
                        .filter(|e| e.enabled)
                        .map(|e| e.path.clone())
                        .collect();
                    if !paths.is_empty() {
                        result.push((port, paths));
                        continue;
                    }
                }
            }

            // Built-in fallback
            if let Some(builtin) = yinhe_editor_core::config::builtin_soundfont_path() {
                let path_str = builtin.to_string_lossy().to_string();
                result.push((port, vec![path_str]));
            }
        }

        result
    }

    /// Rebuild the audio engine if the active document changed or audio was dropped.
    pub(crate) fn rebuild_audio_if_needed(&mut self) {
        let idx = match self.active_doc {
            Some(idx) => idx,
            None => return,
        };

        let needs_rebuild = self.audio_state.active_doc != Some(idx) || self.audio_state.handle.is_none();
        if !needs_rebuild {
            return;
        }

        progress::set_visible(&self.load_progress, true);
        progress::set_stage(&self.load_progress, 2, progress::StageStatus::Active);

        // Drop old audio (stops cpal stream, frees engine)
        self.audio_state.handle = None;

        let doc = &self.documents[idx];
        let sr = self.audio_settings.sample_rate;
        let layout = yinhe_audio::channels_for_model(&doc.data.model);
        // spawn_cpal_audio 消费 layout，提前克隆一份作为快照，
        // 供后续 notify_notes_changed / notify_audio_model_changed 做 flip 检测。
        let layout_snapshot = layout.clone();
        let buffer_size = if self.audio_settings.buffer_size == 0 {
            cpal::BufferSize::Default
        } else {
            cpal::BufferSize::Fixed(self.audio_settings.buffer_size)
        };

        match yinhe_audio::spawn_cpal_audio(
            sr,
            layout,
            buffer_size,
            self.audio_settings.output_device_name.as_deref(),
            #[cfg(feature = "gpu")]
            self.audio_settings.use_gpu_synth,
        ) {
            Ok(audio) => {
                progress::set_stage(&self.load_progress, 2, progress::StageStatus::Done);

                self.send_initial_audio_state(&audio, doc);

                self.audio_state.handle = Some(audio);
                self.audio_state.active_doc = Some(idx);
                self.audio_state.last_channel_layout = Some(layout_snapshot);

                progress::set_visible(&self.load_progress, false);
            }
            Err(e) => {
                tracing::error!("Failed to create audio: {}", e);
                progress::set_visible(&self.load_progress, false);
            }
        }
    }

    /// Send the initial state to a freshly spawned audio handle:
    /// automation density, model, layer count, soundfonts, mute/solo.
    ///
    /// 拆分自 `rebuild_audio_if_needed`，让 spawn 路径与初始状态注入解耦。
    fn send_initial_audio_state(&self, audio: &yinhe_audio::CpalAudioHandle, doc: &yinhe_editor_core::document::Document) {
        // Apply automation density before LoadModel so the first prepare uses it
        audio.handle.send(yinhe_audio::AudioCommand::SetAutomationDensity {
            density: self.audio_settings.automation_event_density,
        });

        // Load MIDI
        audio.handle.send(yinhe_audio::AudioCommand::LoadModel {
            model: doc.data.model.clone(),
        });

        // Apply XSynth layer count
        let layers = if self.audio_settings.xsynth_layers == 0 {
            None
        } else {
            Some(self.audio_settings.xsynth_layers as usize)
        };
        audio
            .handle
            .send(yinhe_audio::AudioCommand::SetLayerCount { count: layers });

        // Load SoundFonts — resolved from global + project config
        let port_configs = self.resolve_sf_config(doc);
        let total_sf: usize = port_configs.iter().map(|(_, p)| p.len()).sum();
        progress::set_stage_progress(
            &self.load_progress,
            3,
            0.0,
            format!("0/{}", total_sf),
        );
        let mut loaded = 0usize;
        for (port, paths) in &port_configs {
            for _p in paths {
                loaded += 1;
                progress::set_stage_progress(
                    &self.load_progress,
                    3,
                    loaded as f32 / total_sf.max(1) as f32,
                    format!("{}/{}", loaded, total_sf),
                );
            }
            audio.handle.send(yinhe_audio::AudioCommand::LoadSoundFont {
                port: *port,
                paths: paths.clone(),
            });
        }
        progress::set_stage(&self.load_progress, 3, progress::StageStatus::Done);

        // Send initial mute/solo state
        let has_solo = doc.edit.track_overrides.iter().any(|t| t.soloed);
        let skip: Vec<bool> = doc
            .edit.track_overrides
            .iter()
            .map(|ov| if has_solo { !ov.soloed } else { ov.muted })
            .collect();
        audio
            .handle
            .send(yinhe_audio::AudioCommand::SkipTracks { skip });
    }

    /// 切换音频输出设备（由"音频设备切换"对话框触发）。
    ///
    /// 流程：保存当前 sample_position → 更新设置 → drop 旧 handle →
    /// `rebuild_audio_if_needed` 用新设备名 spawn → 发 Seek 恢复位置。
    ///
    /// spawn 成功：清 `device_switch_pending`，对话框下帧消失。
    /// spawn 失败：保留 `device_switch_pending`，把错误塞进 `device_switch_error`，
    /// 对话框保持打开让用户重选。
    pub(crate) fn switch_audio_device(&mut self, device_name: String) {
        let saved_sample = self
            .audio_state
            .handle
            .as_ref()
            .map(|h| h.handle.sample_position())
            .unwrap_or(0);

        self.audio_settings.output_device_name = Some(device_name);
        self.audio_settings.save();

        // drop 旧 handle（Drop trait 会通知 renderer 线程退出），强制 rebuild
        self.audio_state.handle = None;

        // 用新设备名重建（rebuild_audio_if_needed 会读 output_device_name）
        self.rebuild_audio_if_needed();

        if let Some(audio) = &self.audio_state.handle {
            // spawn 成功 —— 恢复播放位置，关对话框
            audio
                .handle
                .send(yinhe_audio::AudioCommand::Seek { sample: saved_sample });
            self.audio_state.device_switch_pending = false;
            self.audio_state.device_switch_error = None;
        } else {
            // spawn 失败 —— 保留对话框，显示错误
            self.audio_state.device_switch_error =
                Some("无法在该设备上创建音频流，请选另一个设备".to_string());
        }
    }

    /// Handle playback toggle/pause/stop and cursor sync.
    pub(crate) fn handle_playback(
        &mut self,
        toggle_play: bool,
        pause_return: bool,
        stop_play: bool,
    ) {
        let (idx, audio) = match (self.active_doc, &self.audio_state.handle) {
            (Some(idx), Some(audio)) => (idx, audio),
            _ => return,
        };

        let doc = &mut self.documents[idx];
        let handle = &audio.handle;

        if toggle_play {
            if handle.is_playing() {
                handle.send(yinhe_audio::AudioCommand::Pause);
                self.audio_state.pending_playback = false;
                let sample = handle.sample_position();
                let time = sample as f64 / audio.sample_rate as f64;
                doc.edit.cursor_tick = Some(doc.data.model.tempo_map.tick_at_time(time));
                doc.edit.playback.stop();
            } else {
                let tick = doc.edit.cursor_tick.unwrap_or(0.0);
                let cursor_sample = (doc.data.model.tempo_map.tick_to_seconds(tick as u64)
                    * audio.sample_rate as f64) as u64;
                let engine_sample = handle.sample_position();
                // If cursor is at the engine's position, just resume (no seek)
                if cursor_sample.abs_diff(engine_sample) < (audio.sample_rate as u64 / 10) {
                    handle.send(yinhe_audio::AudioCommand::Resume);
                } else {
                    handle.send(yinhe_audio::AudioCommand::Play {
                        from_sample: cursor_sample,
                    });
                }
                self.audio_state.pending_playback = true;
                doc.edit.playback.toggle_play(tick, &doc.data.model);
            }
        }
        if pause_return {
            handle.send(yinhe_audio::AudioCommand::Pause);
            self.audio_state.pending_playback = false;
            let sample = handle.sample_position();
            let time = sample as f64 / audio.sample_rate as f64;
            doc.edit.cursor_tick = Some(doc.data.model.tempo_map.tick_at_time(time));
            doc.edit.playback.stop();
        }
        if stop_play {
            handle.send(yinhe_audio::AudioCommand::Stop);
            self.audio_state.pending_playback = false;
            doc.edit.cursor_tick = Some(0.0);
            doc.edit.playback.stop();
        }

        // 光标推进交给 interpolate_playback_cursor 独占处理～
        // 这里千万不要每帧重置 playback_anchor，不然插值会被压成一帧、变得一卡一卡的喵！
    }

    /// Between audio callback updates, interpolate the cursor position
    /// using the last known anchor + elapsed wall-clock time.
    /// Call this every frame during playback for smooth cursor motion.
    pub(crate) fn interpolate_playback_cursor(&mut self) {
        let (idx, audio) = match (self.active_doc, &self.audio_state.handle) {
            (Some(idx), Some(audio)) => (idx, audio),
            _ => return,
        };
        let handle = &audio.handle;
        if !handle.is_playing() {
            self.audio_state.playback_anchor = None;
            // Clear pending flag once the audio thread has caught up
            if self.audio_state.pending_playback {
                // Audio thread hasn't processed the Play command yet.
                // Keep the flag set so request_repaint() keeps firing.
                return;
            }
            return;
        }
        // Audio is confirmed playing — clear the pending flag.
        self.audio_state.pending_playback = false;

        let sr = audio.sample_rate as f64;
        let doc = match self.documents.get_mut(idx) {
            Some(doc) => doc,
            None => return,
        };

        let now = std::time::Instant::now();
        let engine_sample = handle.sample_position();

        // Anchor = the last time the engine's atomic sample position actually
        // changed.  We only refresh it when engine_sample differs from the
        // anchored sample, so `elapsed` accumulates the true wall-clock time
        // since the audio callback last advanced the position.  Resetting the
        // anchor every frame would collapse `elapsed` to a single frame and
        // kill the interpolation (cursor would step once per callback).
        let interpolated_sample = match self.audio_state.playback_anchor {
            Some((anchor_sample, anchor_time)) if engine_sample == anchor_sample => {
                // Atomic unchanged — extrapolate from the anchor.
                let elapsed = now.saturating_duration_since(anchor_time);
                anchor_sample as f64 + elapsed.as_secs_f64() * sr
            }
            _ => {
                // Atomic advanced (or first frame) — re-anchor to it.
                self.audio_state.playback_anchor = Some((engine_sample, now));
                engine_sample as f64
            }
        };

        let time = interpolated_sample / sr;
        let tick = doc.data.model.tempo_map.tick_at_time(time);
        let end_tick = doc.data.model.tick_length as f64;
        if tick >= end_tick {
            handle.send(yinhe_audio::AudioCommand::Stop);
            doc.edit.cursor_tick = Some(0.0);
            doc.edit.playback.stop();
            self.audio_state.playback_anchor = None;
        } else {
            doc.edit.cursor_tick = Some(tick.max(0.0));
        }
    }

    /// Tear down audio (e.g. on new project or settings change).
    pub(crate) fn teardown_audio(&mut self) {
        self.audio_state.handle = None;
        self.audio_state.active_doc = None;
        self.audio_state.last_channel_layout = None;
    }
}
