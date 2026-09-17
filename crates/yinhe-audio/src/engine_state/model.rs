//! 模型与音色库应用：LoadModel/UpdateNotes 结果落引擎、鼓组模式、音色库挂载。

use std::sync::Arc;

use xsynth_core::channel::{ChannelConfigEvent, ChannelEvent};
use xsynth_core::channel_group::SynthEvent;
use xsynth_core::soundfont::SoundfontBase;

use yinhe_core::YinModel;
use yinhe_types::KEY_COUNT;

use crate::audio_model::{AudioModel, PreparedModel, flatten_automation_to_cc_events};
use crate::channel::ChaseSkip;
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

    fn setup_percussion(&mut self, model: &AudioModel) {
        // GPU 模式的鼓组/乐器模式在 `build_gpu_events` 的 seek 注入中处理，
        // ChannelSet 不参与渲染（与 CPU `setup_percussion` 同序的注入在那里）。
        if !self.cpu_synth_active() {
            return;
        }
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
}
