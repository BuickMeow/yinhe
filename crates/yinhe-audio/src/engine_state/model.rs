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

impl AudioEngine {
    pub(crate) fn load_model(&mut self, model: &Arc<YinModel>) {
        let audio_model = AudioModel::from_model(model);
        self.setup_percussion(&audio_model);

        self.cc_events = flatten_automation_to_cc_events(model, self.automation_density);
        self.chase_generation = self.chase_generation.wrapping_add(1);
        self.loop_chase = None;
        self.cc_cursor = 0;
        self.dispatched_skip = ChaseSkip::default();
        self.active_notes.clear();
        self.reset_track_vel(model.tracks.len());

        // 用当前 AM M/S 旁通集重建 lane 跳过掩码（模型已是新结构）。
        self.am_lane_skip = crate::audio_model::build_am_lane_skip(model, &self.am_ms);

        self.duration_samples =
            (crate::prepare_model::model_duration_seconds(model) * self.sample_rate as f64) as u64;

        // skip 的唯一来源：音频轨没有片段（无可听内容，note_count 恒为 0，
        // 不能按音符数判定）。空轨的音符本来就不会被 dispatch（没有音符），
        // 而自动化（CC/PB/RPN）必须照常发送——此前用 audible_count==0 过滤会
        // 误杀挂在无音符轨上的全部自动化事件（cyber-night 的根因）。
        self.skip_track = model
            .tracks
            .iter()
            .map(|t| t.kind == yinhe_core::TrackKind::Audio && t.audio_clips.is_empty())
            .collect();

        self.note_cursor = [yinhe_types::NoteCursor::default(); KEY_COUNT];
        self.current_tick = 0;
        self.loop_wrap_offset = 0;
        self.yin_model = Some(Arc::clone(model));
        self.model = Some(audio_model);
        self.apply_all_synth_params(model);
    }

    /// 应用某全局通道的工程级音源参数（含默认值，确保换工程时清空旧覆盖）。
    pub(crate) fn apply_synth_params(&mut self, channel: u8, params: yinhe_types::SynthParams) {
        let dense = self.channel_layout.dense_for(channel as usize);
        if dense == u32::MAX {
            return;
        }
        if let Some(cs) = self.cpu_synth.as_mut() {
            cs.set_synth_params(dense, params);
        }
    }

    /// 全量应用工程里的音源参数（加载/切歌后调用）。
    pub(crate) fn apply_all_synth_params(&mut self, model: &YinModel) {
        for ch in 0..model.synth_params.len().min(256) {
            self.apply_synth_params(ch as u8, model.synth_params[ch]);
        }
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
        self.loop_chase = None;
        self.duration_samples = prepared.duration_samples;
        // Skip is ignored here — we keep whatever the user set via SkipTracks.
        let yin_model = prepared.yin_model;
        self.model = Some(prepared.model);

        // 用当前 AM M/S 旁通集重建 lane 跳过掩码（模型可能是新结构）。
        self.am_lane_skip = crate::audio_model::build_am_lane_skip(&yin_model, &self.am_ms);
        self.yin_model = Some(yin_model);
        let track_count = self.yin_model.as_ref().map(|m| m.tracks.len()).unwrap_or(0);
        self.reset_track_vel(track_count);
        let model = self.yin_model.clone();
        if let Some(model) = model {
            self.apply_all_synth_params(&model);
        }

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
    ///
    /// 音符本体就在 `YinModel.notes[key]` 里，直接换模型引用即可；游标按当前
    /// tick 重新定位（256 次二分，O(log N)），不需要 worker 预构建任何增量。
    pub(crate) fn apply_notes_only(
        &mut self,
        model: AudioModel,
        yin_model: Arc<YinModel>,
        duration_samples: u64,
    ) {
        self.setup_percussion(&model);
        self.duration_samples = duration_samples;
        self.model = Some(model);

        // 块末重定位：`<= current_tick` 的音符已经派发完，用 cursor_after
        // 跳过同一 tick 上的音符，避免块边界上的音符被重复触发。
        let tick = self.current_tick;
        for key in 0..KEY_COUNT {
            self.note_cursor[key] = yin_model.notes[key].cursor_after(tick);
        }
        self.yin_model = Some(yin_model);
    }

    fn setup_percussion(&mut self, model: &AudioModel) {
        let declarations = model.percussion_declarations();
        self.apply_percussion_declarations(&declarations);
    }

    /// 把鼓组/乐器模式声明注入当前后端。
    ///
    /// 必须在**每次状态复位之后**重新调用：`ChannelState::new` / xsynth 的
    /// `ResetControl` / `CpuSynth::seek` 都会把 bank 打回 0，只靠"模型加载时
    /// 注入一次"会让播放/seek 后鼓组失效（关鼓开关也因此看起来没反应）。
    fn apply_percussion_declarations(&mut self, declarations: &[(u16, bool)]) {
        // yinhe CPU 后端经 `PercussionMode` 控制事件直接生效。
        if self.cpu_synth.is_some() {
            self.setup_percussion_cpu_synth(declarations);
            return;
        }
        if !self.cpu_synth_active() {
            return;
        }
        for &(src_ch, on) in declarations {
            let dense = self.channel_layout.dense_for(src_ch as usize);
            if dense == u32::MAX {
                continue;
            }
            self.channel_set.send_event(SynthEvent::Channel(
                dense,
                ChannelEvent::Config(ChannelConfigEvent::SetPercussionMode(on)),
            ));
        }
    }

    /// 复位后重新声明鼓组模式（seek / stop / loop 回绕调用）。
    pub(crate) fn reapply_percussion(&mut self) {
        let Some(model) = self.model.as_ref() else {
            return;
        };
        let declarations = model.percussion_declarations();
        self.apply_percussion_declarations(&declarations);
    }

    /// yinhe CPU 后端的鼓组/乐器模式注入（与 ChannelSet 路径同一份声明表）。
    fn setup_percussion_cpu_synth(&mut self, declarations: &[(u16, bool)]) {
        let sample = self.sample_position;
        let mut events: Vec<(u32, bool)> = Vec::new();
        for &(src_ch, on) in declarations {
            let dense = self.channel_layout.dense_for(src_ch as usize);
            if dense != u32::MAX {
                events.push((dense, on));
            }
        }
        if let Some(cs) = self.cpu_synth.as_mut() {
            for (dense, on) in events {
                cs.send_event(yinhe_synth::SynthEvent::Control {
                    sample,
                    channel: dense as u8,
                    event: yinhe_synth::ControlEvent::PercussionMode(on),
                });
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
        paths: &[String],
    ) {
        if dense == u32::MAX {
            return;
        }
        let sfz = paths
            .iter()
            .any(|p| p.to_ascii_lowercase().ends_with(".sfz"));
        self.sf_manager.apply_loaded_for_channel(
            channel,
            soundfonts,
            &mut self.channel_set,
            dense,
            sfz,
        );
    }
}
