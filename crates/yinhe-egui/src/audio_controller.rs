use std::sync::Arc;

use crate::app::App;

impl App {
    /// Resolve the merged SF configuration for the given document.
    ///
    /// Returns a list of `(port, paths)` for every port the MIDI uses.
    fn resolve_sf_config(&self, doc: &crate::document::Document) -> Vec<(u8, Vec<String>)> {
        let num_ch = yinhe_audio::channels_for_midi(&doc.midi).0;
        let num_ports = ((num_ch / 16) as u8).max(1);
        let global = &self.audio_settings.global_sf_config;
        let project = &doc.project_sf;

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
            if let Some(builtin) = crate::right_panel::config::builtin_soundfont_path() {
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

        let needs_rebuild = self.audio_active_doc != Some(idx) || self.audio.is_none();
        if !needs_rebuild {
            return;
        }

        // Drop old audio (stops cpal stream, frees engine)
        self.audio = None;

        let doc = &self.documents[idx];
        let sr = self.audio_settings.sample_rate;
        let (num_ch, active_mask) = yinhe_audio::channels_for_midi(&doc.midi);

        match yinhe_audio::spawn_cpal_audio(sr, num_ch, active_mask) {
            Ok(audio) => {
                // Load MIDI
                audio.handle.send(yinhe_audio::AudioCommand::LoadMidi {
                    midi: Arc::clone(&doc.midi),
                });
                // Load SoundFonts — resolved from global + project config
                let port_configs = self.resolve_sf_config(doc);
                for (port, paths) in &port_configs {
                    audio.handle.send(yinhe_audio::AudioCommand::LoadSoundFont {
                        port: *port,
                        paths: paths.clone(),
                    });
                }
                // Send initial mute/solo state
                let has_solo = doc.track_overrides.iter().any(|t| t.soloed);
                let skip: Vec<bool> = doc
                    .track_overrides
                    .iter()
                    .map(|ov| if has_solo { !ov.soloed } else { ov.muted })
                    .collect();
                audio
                    .handle
                    .send(yinhe_audio::AudioCommand::SkipTracks { skip });

                self.audio = Some(audio);
                self.audio_active_doc = Some(idx);
            }
            Err(e) => {
                tracing::error!("Failed to create audio: {}", e);
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
        let (idx, audio) = match (self.active_doc, &self.audio) {
            (Some(idx), Some(audio)) => (idx, audio),
            _ => return,
        };

        let doc = &mut self.documents[idx];
        let handle = &audio.handle;

        if toggle_play {
            if handle.is_playing() {
                handle.send(yinhe_audio::AudioCommand::Pause);
                let sample = handle.sample_position();
                let time = sample as f64 / audio.sample_rate as f64;
                doc.cursor_tick = Some(doc.midi.tick_at_time(time));
                doc.playback.stop();
            } else {
                let tick = doc.cursor_tick.unwrap_or(0.0);
                let cursor_sample =
                    (doc.midi.tick_to_seconds(tick as u64) * audio.sample_rate as f64) as u64;
                let engine_sample = handle.sample_position();
                // If cursor is at the engine's position, just resume (no seek)
                if cursor_sample.abs_diff(engine_sample) < (audio.sample_rate as u64 / 10) {
                    handle.send(yinhe_audio::AudioCommand::Resume);
                } else {
                    handle.send(yinhe_audio::AudioCommand::Play {
                        from_sample: cursor_sample,
                    });
                }
                doc.playback.toggle_play(tick, &doc.midi);
            }
        }
        if pause_return {
            handle.send(yinhe_audio::AudioCommand::Pause);
            let sample = handle.sample_position();
            let time = sample as f64 / audio.sample_rate as f64;
            doc.cursor_tick = Some(doc.midi.tick_at_time(time));
            doc.playback.stop();
        }
        if stop_play {
            handle.send(yinhe_audio::AudioCommand::Stop);
            doc.cursor_tick = Some(0.0);
            doc.playback.stop();
        }

        // Sync cursor from audio position during playback
        if handle.is_playing() {
            let sample = handle.sample_position();
            let time = sample as f64 / audio.sample_rate as f64;
            let tick = doc.midi.tick_at_time(time);
            let end_tick = doc.midi.tick_length as f64;
            if tick >= end_tick {
                handle.send(yinhe_audio::AudioCommand::Stop);
                doc.cursor_tick = Some(0.0);
                doc.playback.stop();
            } else {
                doc.cursor_tick = Some(tick.max(0.0));
            }
        }
    }

    /// Tear down audio (e.g. on new project or settings change).
    pub(crate) fn teardown_audio(&mut self) {
        self.audio = None;
        self.audio_active_doc = None;
    }
}
