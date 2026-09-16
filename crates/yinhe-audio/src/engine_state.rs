use std::collections::BinaryHeap;
use std::sync::Arc;

use xsynth_core::channel::{ChannelAudioEvent, ChannelConfigEvent, ChannelEvent};
use xsynth_core::channel_group::SynthEvent;
use xsynth_core::soundfont::SoundfontBase;

use yinhe_core::YinModel;
use yinhe_mixer::PluginEvent;
use yinhe_types::KEY_COUNT;

use crate::audio_model::{
    ActiveNote, AudibleNote, AudioModel, PreparedModel, flatten_automation_to_cc_events,
};
use crate::channel::{ChannelState, ChaseSkip};
use crate::engine::AudioEngine;
use crate::prepare_model::build_audible_notes;

impl AudioEngine {
    pub(crate) fn load_model(&mut self, model: &Arc<YinModel>) {
        let audio_model = AudioModel::from_model(model);
        self.setup_percussion(&audio_model);

        self.cc_events = flatten_automation_to_cc_events(model, self.automation_density);
        self.chase_generation = self.chase_generation.wrapping_add(1);
        self.cc_cursor = 0;
        self.dispatched_skip = ChaseSkip::default();
        self.active_notes.clear();

        // 用当前 AM M/S 旁通集重建 lane 跳过掩码（模型已是新结构）。
        self.am_lane_skip = crate::audio_model::build_am_lane_skip(model, &self.am_ms);

        self.duration_samples =
            (crate::prepare_model::model_duration_seconds(model) * self.sample_rate as f64) as u64;

        // skip 的唯一来源：音频轨没有片段（无可听内容，note_count 恒为 0，
        // 不能按音符数判定）。MIDI 轨不按 audible_count 跳过：空轨的音符本来
        // 就不在 audible_notes 里（无需过滤），而自动化（CC/PB/RPN）必须照常
        // 发送——此前用 audible_count==0 过滤会误杀挂在无音符轨上的全部
        // 自动化事件（cyber-night 全部 17 万 CC/PB 被丢弃的根因）。
        self.skip_track = model
            .tracks
            .iter()
            .map(|t| t.kind == yinhe_core::TrackKind::Audio && t.audio_clips.is_empty())
            .collect();

        self.note_cursor = [0; KEY_COUNT];
        self.current_tick = 0;
        self.yin_model = Some(Arc::clone(model));
        self.audible_notes = build_audible_notes(model);
        self.model = Some(audio_model);
    }

    /// Apply a `PreparedModel` computed on a worker thread.
    ///
    /// `anchor`：seek 目标采样位置。**必须是听音（消费）位置**而非渲染前沿，
    /// 否则非显式 reload（自动化编辑 / undo / M/S 掩码重建）会前跳播放位置。
    /// 用户显式 seek（Play/Seek/Stop）在命令层直接 seek，与本方法无关。
    pub(crate) fn apply_prepared_model(&mut self, prepared: PreparedModel, anchor: u64) {
        self.setup_percussion(&prepared.model);

        self.cc_events = prepared.cc_events;
        // cc_events 变了，旧 generation 的 chase 结果必须丢弃
        self.chase_generation = self.chase_generation.wrapping_add(1);
        self.duration_samples = prepared.duration_samples;
        // Skip is ignored here — we keep whatever the user set via SkipTracks.
        let yin_model = prepared.yin_model;
        self.audible_notes = prepared.audible_notes;
        self.model = Some(prepared.model);

        // 用当前 AM M/S 旁通集重建 lane 跳过掩码（模型可能是新结构）。
        self.am_lane_skip = crate::audio_model::build_am_lane_skip(&yin_model, &self.am_ms);
        self.yin_model = Some(yin_model);

        // Seek to the audible position to avoid triggering all notes
        // before the current position (which would cause voice stealing).
        // 方案 B：seek_to 不再同步 chase —— renderer 在 apply_prepared_model 返回后
        // 发 PrepareChase 给 worker 异步计算 channel state。
        self.seek_to(anchor);

        // If Play arrived while loading, seek now
        if let Some(from_sample) = self.pending_play_from_sample.take() {
            let t = std::time::Instant::now();
            self.seek_to(from_sample);
            self.playing = true;
            crate::audio_renderer::play_log(&format!(
                "[play] 挂起的 Play 生效（seek={:?}）",
                t.elapsed()
            ));
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
    /// 构建 chase 跳过掩码：`dispatched_skip` 中自 seek 以来实际发送的控制器。
    /// dispatch 在发送每个 CC/PB/RPN/PC 事件时打点，`seek_to` 清零；
    /// 与旧实现（按 `[chase_cc_base, cc_cursor)` 区间扫描）不同，mute 期间
    /// 被越过但未发送的事件不会被误标——unmute 后 chase 能恢复这些控制器。
    pub(crate) fn chase_skip(&self) -> ChaseSkip {
        self.dispatched_skip
    }

    pub(crate) fn apply_chase_result(
        &mut self,
        states: &[Option<ChannelState>; 256],
        plugin_params: &[(u8, u32, f32)],
    ) {
        let skip = self.chase_skip();
        for ch in 0..256u32 {
            let dense = self.channel_layout.dense_for(ch as usize);
            if dense == u32::MAX {
                continue;
            }
            // 无事件通道（如被 mute 轨独占的通道）不触碰：保持当前状态不重置。
            let Some(state) = &states[ch as usize] else {
                continue;
            };
            state.send_to(dense, &mut self.channel_set, &skip);
            // 通道级 DSP CC 回填给 yinhe-dsp 模块（与 xsynth 的 skip 语义一致：
            // 已被 dispatch 的 CC 不覆盖，避免旧值打回新值）。
            for &cc in yinhe_dsp::cc::DSP_CHANNEL_CCS {
                if skip.cc_mask[ch as usize] & (1u128 << cc) == 0 {
                    self.mixer
                        .broadcast_channel_cc(dense as usize, cc, state.dsp_cc_value(cc));
                }
            }
        }
        // 插件参数 chase：seek 后插件已 reset（值丢失），把目标位置的
        // lane 值写回对应乐器实例（归一化值，下一块 process 生效）。
        for &(ch, param_id, value) in plugin_params {
            let Some(dense) = self.channel_plugin_dense(ch) else {
                continue;
            };
            if let Some(Some(slot)) = self.instruments.get_mut(dense) {
                slot.events.push(PluginEvent::ParamValue {
                    time: 0,
                    param_id,
                    value: f64::from(value),
                });
            }
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

    /// 加载某源通道的音色库（同步路径：测试/直连命令用）。
    pub(crate) fn load_soundfont_for_channel(&mut self, channel: u8, paths: &[String]) {
        let dense = self.channel_layout.dense_for(channel as usize);
        if dense == u32::MAX {
            return;
        }
        let _ = self
            .sf_manager
            .load_for_channel(channel, paths, &mut self.channel_set, dense);
    }

    /// 应用 worker 加载完成的音色库（按源通道索引；dense 由调用方预计算）。
    pub(crate) fn apply_loaded_soundfont_for_channel(
        &mut self,
        channel: u8,
        dense: u32,
        soundfonts: Vec<Arc<dyn SoundfontBase>>,
    ) {
        if dense == u32::MAX {
            return;
        }
        self.sf_manager
            .apply_loaded_for_channel(channel, soundfonts, &mut self.channel_set, dense);
    }

    /// 暂停：传输停止 + 停掉乐器插件的挂音。
    ///
    /// 插件在停止状态仍被空闲渲染持续 process（GUI 键盘/尾音语义），
    /// 不杀挂音就会在暂停时一直响（连续音色如三角波尤其明显）。
    /// tick 未推进 → 音符未结束，恢复时按跨点音符重启。
    pub(crate) fn pause(&mut self) {
        self.playing = false;
        let all = std::mem::take(&mut self.active_notes);
        let mut remaining = std::collections::BinaryHeap::new();
        for std::cmp::Reverse(an) in all.into_iter() {
            if an.is_instrument {
                if let Some(Some(slot)) = self.instruments.get_mut(an.dense as usize) {
                    slot.events.push(PluginEvent::NoteOff {
                        time: 0,
                        channel: an.clap_channel,
                        key: an.key,
                        velocity: 0.0,
                    });
                }
            } else {
                remaining.push(std::cmp::Reverse(an));
            }
        }
        self.active_notes = remaining;
        // 试听音同样停掉（暂停不该有预览在响）。
        self.preview_instrument_stop(None, None);
    }

    /// 恢复（Pause 后）：重启 pause 期间被杀掉的**乐器**跨点音符。
    /// xsynth 音符不受影响（暂停不杀 xsynth voice，恢复自然续响）。
    pub(crate) fn resume(&mut self) {
        self.playing = true;
        self.restart_crossing_instrument_notes();
    }

    /// 重启当前 tick 处仍有效的乐器跨点音符（pause→resume 用）。
    fn restart_crossing_instrument_notes(&mut self) {
        let tick = self.current_tick;
        for key in 0..KEY_COUNT {
            let notes = self.audible_notes[key].as_slice();
            let cursor = notes.partition_point(|n| n.start_tick < tick);
            let mut to_restart: Vec<AudibleNote> = Vec::new();
            for n in &notes[..cursor] {
                if n.end_tick > tick {
                    let ch = self
                        .model
                        .as_ref()
                        .map(|m| m.track_channel(n.track as usize))
                        .unwrap_or(0);
                    if self.channel_plugin_dense(ch).is_some() {
                        to_restart.push(*n);
                    }
                }
            }
            for n in to_restart {
                self.restart_note(key, &n);
            }
        }
    }

    /// 重启一个跨点音符（seek / unmute 复用）：NoteOn + 记入 active_notes。
    /// `key`：桶索引。`n`：audible_notes 里的源音符（已是当前 tick 之前的跨点音符，
    /// 由调用方过滤 end_tick > tick）。同步检查 skip_track（mute 轨跳过不重启）。
    fn restart_note(&mut self, key: usize, n: &AudibleNote) {
        let track = n.track as usize;
        let ch = self
            .model
            .as_ref()
            .map(|m| m.track_channel(track) as usize)
            .unwrap_or(0);
        if self.skip_track.get(track).copied().unwrap_or(false) {
            return;
        }
        if let Some(dense) = self.channel_plugin_dense(ch as u8) {
            // 插件通道：chase 重启的音符喂乐器实例（time 0 = 下一块开头）。
            if let Some(Some(slot)) = self.instruments.get_mut(dense) {
                slot.events.push(PluginEvent::NoteOn {
                    time: 0,
                    channel: (ch & 0x0F) as u8,
                    key: key as u8,
                    velocity: n.velocity as f64 / 127.0,
                });
                self.active_notes.push(std::cmp::Reverse(ActiveNote {
                    key: key as u8,
                    dense: dense as u32,
                    clap_channel: (ch & 0x0F) as u8,
                    is_instrument: true,
                    end_tick: n.end_tick,
                    track: track as u16,
                }));
            }
            return;
        }
        let dense = self.channel_layout.dense_for(ch);
        if dense == u32::MAX {
            return;
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
            dense,
            clap_channel: 0,
            is_instrument: false,
            end_tick: n.end_tick,
            track: track as u16,
        }));
    }

    /// 即时 mute：从 active_notes 精确移除该轨在响音符并发 NoteOff
    ///（不误伤同通道其他轨道；CPU/GPU 路径统一为即时静音语义）。
    fn kill_track_notes(&mut self, track: u16) {
        let all = std::mem::take(&mut self.active_notes);
        let mut remaining = BinaryHeap::new();
        for std::cmp::Reverse(an) in all.into_iter() {
            if an.track != track {
                remaining.push(std::cmp::Reverse(an));
                continue;
            }
            if an.is_instrument {
                if let Some(Some(slot)) = self.instruments.get_mut(an.dense as usize) {
                    slot.events.push(PluginEvent::NoteOff {
                        time: 0,
                        channel: an.clap_channel,
                        key: an.key,
                        velocity: 0.0,
                    });
                }
            } else if an.dense != u32::MAX {
                self.channel_set.send_event(SynthEvent::Channel(
                    an.dense,
                    ChannelEvent::Audio(ChannelAudioEvent::NoteOff { key: an.key }),
                ));
            }
        }
        self.active_notes = remaining;
    }

    /// 即时 unmute：重启该轨的跨点音符（start < current_tick < end）。
    /// 代价 O(该轨在各 key 桶 [..cursor] 的音符数)，只扫受影响轨道。
    fn restart_track_crossing_notes(&mut self, track: u16) {
        let tick = self.current_tick;
        for key in 0..KEY_COUNT {
            let notes = self.audible_notes[key].as_slice();
            let cursor = notes.partition_point(|n| n.start_tick < tick);
            let mut to_restart: Vec<AudibleNote> = Vec::new();
            for n in &notes[..cursor] {
                if n.track == track && n.end_tick > tick {
                    to_restart.push(*n);
                }
            }
            for n in to_restart {
                self.restart_note(key, &n);
            }
        }
    }

    /// 应用新的轨道 skip 掩码，按 diff 即时处理：新 mute 的轨立即停音，
    /// 新 unmute 的轨立即重启跨点音符。`self.skip_track` 同步更新为 `new`。
    pub(crate) fn apply_skip_mask(&mut self, old: &[bool], new: &[bool]) {
        self.skip_track = new.to_vec();
        let n = old.len().max(new.len());
        for i in 0..n {
            let was = old.get(i).copied().unwrap_or(false);
            let is = new.get(i).copied().unwrap_or(false);
            if was == is {
                continue;
            }
            if is {
                // 新 mute：立即停掉该轨在响音符。
                self.kill_track_notes(i as u16);
            } else {
                // 新 unmute：立即重启该轨跨点音符。
                self.restart_track_crossing_notes(i as u16);
            }
        }
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
        // 乐器实例随 seek 清空内部状态（尾音/envelope/挂音）与事件累积。
        for inst in self.instruments.iter_mut().flatten() {
            inst.processor.reset();
            inst.events.clear();
        }
        // 插件预览的 NoteOff 调度随 seek 作废（reset 已清插件挂音）。
        self.plugin_previews.clear();

        self.sample_position = sample;
        self.current_tick = self.sample_to_tick(sample);
        self.note_cursor = [0; KEY_COUNT];
        self.cc_cursor = 0;
        self.active_notes.clear();

        self.cc_cursor = self
            .cc_events
            .partition_point(|cc| cc.tick < self.current_tick);
        // 自 seek 点起重新打点：之前 dispatch 的控制器全部作废，chase 恢复全量生效
        //（跳过掩码为空，应用 chase 时不跳过任何控制器）。
        self.dispatched_skip = ChaseSkip::default();

        // Reset note cursors to the correct position based on pre-built audible_notes.
        // 桶内 start_tick 严格升序，partition_point 谓词单调，结果正确（修 P0-2）。
        let tick = self.current_tick;
        for key in 0..KEY_COUNT {
            let notes = self.audible_notes[key].as_slice();
            let cursor = notes.partition_point(|n| n.start_tick < tick);
            self.note_cursor[key] = cursor;

            // 扫描 seek 点之前开始、seek 点之后才结束的所有音符，全部重启（修 P2-10）。
            // 桶按 start_tick 升序，但 end_tick 不保证有序，必须线性扫 [..cursor]。
            // 黑乐谱叠层场景下 cursor 前通常有几十个跨点音符，O(cursor) 完全可接受。
            // 先收集再逐条重启（避免借用冲突，`restart_note` 需 &mut self）。
            let mut to_restart: Vec<AudibleNote> = Vec::new();
            for n in &notes[..cursor] {
                if n.end_tick > tick {
                    to_restart.push(*n);
                }
            }
            for n in to_restart {
                self.restart_note(key, &n);
            }
        }

        // 方案 B：chase（恢复 CC/PitchBend/RPN 等控制器值）移到 worker 线程异步计算。
        // renderer 在 seek_to 返回后发 PrepareChase，worker 算完回传 ChaseResult，
        // 由 apply_chase_result 应用。期间 channel state 是 ResetControl 后的初始值，
        // 渲染短暂静音 —— 比 renderer 线程同步阻塞几十万次 ChannelState::apply 更好。
    }
}
