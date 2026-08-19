use std::collections::HashMap;
use std::sync::Arc;

use xsynth_core::channel::{ChannelAudioEvent, ChannelConfigEvent, ChannelEvent};
use xsynth_core::channel_group::SynthEvent;
use xsynth_core::soundfont::SoundfontBase;

use yinhe_core::YinModel;

use crate::audio_model::{ActiveNote, AudioModel, PreparedModel, flatten_automation_to_cc_events};
use crate::channel::{ChannelState, ChaseSkip};
use crate::engine::AudioEngine;
use crate::prepare_model::build_audible_notes;

impl AudioEngine {
    pub(crate) fn load_model(&mut self, model: &Arc<YinModel>) {
        let audio_model = AudioModel::from_model(model);
        self.setup_percussion(&audio_model);

        self.cc_events =
            flatten_automation_to_cc_events(model, self.automation_density, &HashMap::new());
        self.chase_generation = self.chase_generation.wrapping_add(1);
        self.cc_cursor = 0;
        self.chase_cc_base = 0;
        self.active_notes.clear();

        self.duration_samples =
            (model.tempo_map.tick_to_seconds(model.tick_length) * self.sample_rate as f64) as u64;

        self.skip_track = model.track_audible_count.iter().map(|&c| c == 0).collect();

        self.note_cursor = [0; 128];
        self.current_tick = 0;
        self.yin_model = Some(Arc::clone(model));
        self.audible_notes = build_audible_notes(model);
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
    /// 只替换 `audible_delta` 中 dirty 桶并重置对应 note_cursor；
    /// 干净桶保留旧数据与旧 cursor（增量语义，1 亿音符工程编辑不再全量重扫）。
    pub(crate) fn apply_notes_only(
        &mut self,
        model: AudioModel,
        yin_model: Arc<YinModel>,
        audible_delta: crate::audio_model::AudibleDelta,
        duration_samples: u64,
    ) {
        self.setup_percussion(&model);
        self.duration_samples = duration_samples;
        self.yin_model = Some(yin_model);
        self.model = Some(model);

        // 只替换 dirty 桶：重置该桶的 note_cursor（保持当前播放位置，
        // 重新找游标）。不需要 AllNotesOff / ResetControl / chase ——
        // 当前活跃音符和 channel state 不变。
        let tick = self.current_tick;
        for (key, bucket) in audible_delta.into_iter().enumerate() {
            if let Some(bucket) = bucket {
                self.audible_notes[key] = bucket;
                self.note_cursor[key] =
                    self.audible_notes[key].partition_point(|n| n.start_tick < tick);
            }
        }
    }

    /// 方案 B：应用 worker 线程异步算好的 256 通道状态快照。
    /// 在 `seek_to` 之后由 renderer 收到 `ChaseResult` 时调用，恢复各通道的
    /// volume / pan / program / pitch bend / RPN 等控制器值。
    ///
    /// chase 是异步的：结果到达时渲染器可能已经 dispatch 了 seek 点之后的
    /// 实时事件（包括 seek 点同 sample 的事件）。若整体覆盖，这些新值会被
    /// 打回 seek 前的旧值（从中间小节开始播放时 PBS/PitchBend 被覆盖的根因）。
    /// 因此对 `[chase_cc_base, cc_cursor)` 区间内已 dispatch 的控制器跳过，
    /// 只补齐尚未被实时事件覆盖的状态。
    /// 构建 chase 跳过掩码：`[chase_cc_base, cc_cursor)` 区间内已 dispatch 的控制器。
    /// 独立为 pub(crate) 方法，便于测试直接观察跳过行为。
    pub(crate) fn chase_skip(&self) -> ChaseSkip {
        let mut skip = ChaseSkip::default();
        for cc in &self.cc_events[self.chase_cc_base..self.cc_cursor] {
            skip.mark(&cc.event, cc.channel as usize);
        }
        skip
    }

    pub(crate) fn apply_chase_result(&mut self, states: &[ChannelState; 256]) {
        let skip = self.chase_skip();
        for ch in 0..256u32 {
            let dense = self.channel_layout.dense_for(ch as usize);
            if dense == u32::MAX {
                continue;
            }
            states[ch as usize].send_to(dense, &mut self.channel_set, &skip);
        }
    }

    fn setup_percussion(&mut self, model: &AudioModel) {
        // Drum channels in GM are channel 9 of each port (port*16 + 9).
        for src_ch in (9..256).step_by(16) {
            let dense = self.channel_layout.dense_for(src_ch);
            if dense == u32::MAX {
                continue;
            }
            self.channel_set.send_event(SynthEvent::Channel(
                dense,
                ChannelEvent::Config(ChannelConfigEvent::SetPercussionMode(true)),
            ));
        }
        // Honour Bank Select MSB declarations (>= 120 = drum kit, GS/XG
        // convention): standalone CC0 automation lanes and CC0 folded into
        // PcEvent.bank_msb, merged per track in tick order. Last declaration
        // per channel wins, matching the legacy MidiFile path.
        for (track_idx, banks) in model.track_banks.iter().enumerate() {
            if banks.is_empty() {
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
            for &(_, value) in banks {
                self.channel_set.send_event(SynthEvent::Channel(
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
            &mut self.channel_set,
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
            &mut self.channel_set,
            dense_channels,
        );
    }

    pub(crate) fn seek_to(&mut self, sample: u64) {
        self.channel_set
            .send_event(SynthEvent::AllChannels(ChannelEvent::Audio(
                ChannelAudioEvent::AllNotesOff,
            )));
        self.channel_set
            .send_event(SynthEvent::AllChannels(ChannelEvent::Audio(
                ChannelAudioEvent::ResetControl,
            )));
        // insert 效果器（delay 尾音/envelope 等）随 seek 清空内部状态
        self.mixer.reset_inserts();

        self.sample_position = sample;
        self.current_tick = self.sample_to_tick(sample);
        self.note_cursor = [0; 128];
        self.cc_cursor = 0;
        self.active_notes.clear();

        self.cc_cursor = self
            .cc_events
            .partition_point(|cc| cc.tick < self.current_tick);
        // 记录 seek 点，供 apply_chase_result 计算"已 dispatch 事件区间"。
        self.chase_cc_base = self.cc_cursor;

        // Reset note cursors to the correct position based on pre-built audible_notes.
        // 桶内 start_tick 严格升序，partition_point 谓词单调，结果正确（修 P0-2）。
        let tick = self.current_tick;
        for key in 0..128usize {
            let notes = self.audible_notes[key].as_slice();
            let cursor = notes.partition_point(|n| n.start_tick < tick);
            self.note_cursor[key] = cursor;

            // 扫描 seek 点之前开始、seek 点之后才结束的所有音符，全部重启（修 P2-10）。
            // 桶按 start_tick 升序，但 end_tick 不保证有序，必须线性扫 [..cursor]。
            // 黑乐谱叠层场景下 cursor 前通常有几十个跨点音符，O(cursor) 完全可接受。
            for n in &notes[..cursor] {
                if n.end_tick <= tick {
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
                self.channel_set.send_event(SynthEvent::Channel(
                    dense,
                    ChannelEvent::Audio(ChannelAudioEvent::NoteOn {
                        key: key as u8,
                        vel: n.velocity,
                    }),
                ));
                self.active_notes.push(std::cmp::Reverse(ActiveNote {
                    key: key as u8,
                    channel: ch as u8,
                    end_tick: n.end_tick,
                }));
            }
        }

        // 方案 B：chase（恢复 CC/PitchBend/RPN 等控制器值）移到 worker 线程异步计算。
        // renderer 在 seek_to 返回后发 PrepareChase，worker 算完回传 ChaseResult，
        // 由 apply_chase_result 应用。期间 channel state 是 ResetControl 后的初始值，
        // 渲染短暂静音 —— 比 renderer 线程同步阻塞几十万次 ChannelState::apply 更好。
    }
}
