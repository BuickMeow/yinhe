use std::sync::Arc;

use xsynth_core::channel::{ChannelAudioEvent, ChannelConfigEvent, ChannelEvent};
use xsynth_core::channel_group::SynthEvent;
use xsynth_core::soundfont::SoundfontBase;

use yinhe_core::YinModel;

use crate::audio_model::{ActiveNote, AudioModel, AudibleNote, PreparedModel, flatten_automation_to_cc_events};
use crate::channel::ChannelState;
use crate::engine::AudioEngine;
use crate::prepare_model::build_audible_notes;

impl AudioEngine {
    pub(crate) fn load_model(&mut self, model: &Arc<YinModel>) {
        let audio_model = AudioModel::from_model(model);
        self.setup_percussion(&audio_model);

        self.cc_events = flatten_automation_to_cc_events(model, self.sample_rate, self.automation_density);
        self.chase_generation = self.chase_generation.wrapping_add(1);
        self.cc_cursor = 0;
        self.active_notes.clear();

        self.duration_samples = (model.tempo_map.tick_to_seconds(model.tick_length) * self.sample_rate as f64) as u64;

        self.skip_track = model
            .track_audible_count
            .iter()
            .map(|&c| c == 0)
            .collect();

        self.note_cursor = [0; 128];
        self.yin_model = Some(Arc::clone(model));
        self.audible_notes = build_audible_notes(model, self.sample_rate);
        self.model = Some(audio_model);
    }

    /// Apply a `PreparedModel` computed on a worker thread.
    pub(crate) fn apply_prepared_model(&mut self, prepared: PreparedModel) {
        self.setup_percussion(&prepared.model);

        self.cc_events = prepared.cc_events;
        // cc_events 变了，旧 generation 的 chase 结果必须丢弃
        self.chase_generation = self.chase_generation.wrapping_add(1);
        self.duration_samples = prepared.duration_samples;
        // Skip is ignored here — we keep whatever the user set via SkipTracks.
        self.yin_model = Some(prepared.yin_model);
        self.audible_notes = prepared.audible_notes;
        self.model = Some(prepared.model);

        // Seek to current playback position to avoid triggering all notes
        // before the current position (which would cause voice stealing).
        // 方案 B：seek_to 不再同步 chase —— renderer 在 apply_prepared_model 返回后
        // 发 PrepareChase 给 worker 异步计算 channel state。
        let current_sample = self.sample_position;
        self.seek_to(current_sample);

        // If Play arrived while loading, seek now
        if let Some(from_sample) = self.pending_play_from_sample.take() {
            self.seek_to(from_sample);
            self.playing = true;
        }
    }

    /// 方案 A：只应用音符更新（`UpdateNotes` 路径）。
    /// 不重建 cc_events，不 seek，不 chase —— 保持当前播放位置和 channel state。
    /// 只替换 `audible_notes` 和 `model`，影响后续音符 dispatch。
    pub(crate) fn apply_notes_only(
        &mut self,
        model: AudioModel,
        yin_model: Arc<YinModel>,
        audible_notes: Box<[Vec<AudibleNote>; 128]>,
        duration_samples: u64,
    ) {
        self.setup_percussion(&model);
        self.duration_samples = duration_samples;
        self.yin_model = Some(yin_model);
        self.audible_notes = audible_notes;
        self.model = Some(model);

        // 重新计算 note_cursor：保持当前 sample_position，重新找每个 key 桶的游标。
        // 不需要 AllNotesOff / ResetControl / chase —— 当前活跃音符和 channel state 不变。
        let sample = self.sample_position;
        for key in 0..128usize {
            self.note_cursor[key] = self.audible_notes[key].partition_point(|n| n.start_sample < sample);
        }
    }

    /// 方案 B：应用 worker 线程异步算好的 256 通道状态快照。
    /// 在 `seek_to` 之后由 renderer 收到 `ChaseResult` 时调用，恢复各通道的
    /// volume / pan / program / pitch bend / RPN 等控制器值。
    pub(crate) fn apply_chase_result(&mut self, states: Box<[ChannelState; 256]>) {
        for ch in 0..256u32 {
            let dense = self.channel_layout.dense_for(ch as usize);
            if dense == u32::MAX {
                continue;
            }
            states[ch as usize].send_to(dense, &mut self.channel_group);
        }
    }

    fn setup_percussion(&mut self, model: &AudioModel) {
        // Drum channels in GM are channel 9 of each port (port*16 + 9).
        for src_ch in (9..256).step_by(16) {
            let dense = self.channel_layout.dense_for(src_ch);
            if dense == u32::MAX {
                continue;
            }
            self.channel_group.send_event(SynthEvent::Channel(
                dense,
                ChannelEvent::Config(ChannelConfigEvent::SetPercussionMode(true)),
            ));
        }
        // Honour CC0 (Bank Select MSB) values >= 120 as percussion-mode toggle,
        // matching the legacy MidiFile path.
        for (track_idx, cc0_values) in model.track_cc0.iter().enumerate() {
            if cc0_values.is_empty() {
                continue;
            }
            let src_ch = model.track_channel(track_idx) as usize;
            if src_ch >= 256 {
                continue;
            }
            let dense = self.channel_layout.dense_for(src_ch);
            if dense == u32::MAX {
                continue;
            }
            for &value in cc0_values {
                self.channel_group.send_event(SynthEvent::Channel(
                    dense,
                    ChannelEvent::Config(ChannelConfigEvent::SetPercussionMode(value >= 120)),
                ));
            }
        }
    }

    pub(crate) fn load_soundfont_for_port(&mut self, port: u8, paths: &[String]) {
        let dense_channels = self.dense_channels_for_port(port);
        if dense_channels.is_empty() {
            return;
        }
        let _ = self.sf_manager.load_for_port_with_dense(
            port,
            paths,
            &mut self.channel_group,
            &dense_channels,
        );
    }

    pub(crate) fn dense_channels_for_port(&self, port: u8) -> Vec<u32> {
        self.channel_layout.dense_channels_for_port(port)
    }

    pub(crate) fn apply_loaded_soundfont_for_port(
        &mut self,
        port: u8,
        soundfonts: Vec<Arc<dyn SoundfontBase>>,
        dense_channels: &[u32],
    ) {
        if dense_channels.is_empty() {
            return;
        }
        self.sf_manager.apply_loaded_for_port_with_dense(
            port,
            soundfonts,
            &mut self.channel_group,
            dense_channels,
        );
    }

    pub(crate) fn seek_to(&mut self, sample: u64) {
        self.channel_group
            .send_event(SynthEvent::AllChannels(ChannelEvent::Audio(
                ChannelAudioEvent::AllNotesOff,
            )));
        self.channel_group
            .send_event(SynthEvent::AllChannels(ChannelEvent::Audio(
                ChannelAudioEvent::ResetControl,
            )));

        self.sample_position = sample;
        self.note_cursor = [0; 128];
        self.cc_cursor = 0;
        self.active_notes.clear();

        self.cc_cursor = self.cc_events.partition_point(|cc| cc.sample < sample);

        // Reset note cursors to the correct position based on pre-built audible_notes.
        // 桶内 start_sample 严格升序，partition_point 谓词单调，结果正确（修 P0-2）。
        for key in 0..128usize {
            let notes = self.audible_notes[key].as_slice();
            let cursor = notes.partition_point(|n| n.start_sample < sample);
            self.note_cursor[key] = cursor;

            // 扫描 seek 点之前开始、seek 点之后才结束的所有音符，全部重启（修 P2-10）。
            // 桶按 start_sample 升序，但 end_sample 不保证有序，必须线性扫 [..cursor]。
            // 黑乐谱叠层场景下 cursor 前通常有几十个跨点音符，O(cursor) 完全可接受。
            for n in &notes[..cursor] {
                if n.end_sample <= sample {
                    continue;
                }
                let track = n.track as usize;
                let ch = self
                    .model
                    .as_ref()
                    .map(|m| m.track_channel(track) as usize)
                    .unwrap_or(0);
                if self.skip_track.get(track).copied().unwrap_or(false) {
                    continue;
                }
                let dense = self.channel_layout.dense_for(ch);
                if dense == u32::MAX {
                    continue;
                }
                self.channel_group.send_event(SynthEvent::Channel(
                    dense,
                    ChannelEvent::Audio(ChannelAudioEvent::NoteOn {
                        key: key as u8,
                        vel: n.velocity,
                    }),
                ));
                self.active_notes.push(ActiveNote {
                    key: key as u8,
                    channel: ch as u8,
                    end_sample: n.end_sample,
                });
            }
        }

        // 方案 B：chase（恢复 CC/PitchBend/RPN 等控制器值）移到 worker 线程异步计算。
        // renderer 在 seek_to 返回后发 PrepareChase，worker 算完回传 ChaseResult，
        // 由 apply_chase_result 应用。期间 channel state 是 ResetControl 后的初始值，
        // 渲染短暂静音 —— 比 renderer 线程同步阻塞几十万次 ChannelState::apply 更好。
    }
}