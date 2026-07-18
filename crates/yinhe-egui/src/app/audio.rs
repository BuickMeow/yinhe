use crate::app::App;
use yinhe_editor_core::progress;

impl App {
    /// Notify the audio engine that the active document's model has changed.
    pub(crate) fn notify_audio_model_changed(&self) {
        if let (Some(idx), Some(audio)) = (self.active_doc, &self.audio_state.handle) {
            audio.reload_notes(self.documents[idx].data.model.clone());
        }
    }

    /// Resolve the merged SF configuration for the given document.
    ///
    /// Returns a list of `(port, paths)` for every port the MIDI uses.
    pub(crate) fn resolve_sf_config(&self, doc: &yinhe_editor_core::document::Document) -> Vec<(u8, Vec<String>)> {
        let num_ch = yinhe_audio::channels_for_model(&doc.data.model).0;
        let num_ports = (num_ch.div_ceil(16) as u8).max(1);
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
        let (num_ch, active_mask) = yinhe_audio::channels_for_model(&doc.data.model);
        let buffer_size = if self.audio_settings.buffer_size == 0 {
            cpal::BufferSize::Default
        } else {
            cpal::BufferSize::Fixed(self.audio_settings.buffer_size)
        };

        match yinhe_audio::spawn_cpal_audio(
            sr,
            num_ch,
            active_mask,
            buffer_size,
            self.audio_settings.output_device_name.as_deref(),
            #[cfg(feature = "gpu")]
            self.audio_settings.use_gpu_synth,
        ) {
            Ok(audio) => {
                progress::set_stage(&self.load_progress, 2, progress::StageStatus::Done);

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

                self.audio_state.handle = Some(audio);
                self.audio_state.active_doc = Some(idx);

                progress::set_visible(&self.load_progress, false);
            }
            Err(e) => {
                tracing::error!("Failed to create audio: {}", e);
                progress::set_visible(&self.load_progress, false);
            }
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
    }
}
