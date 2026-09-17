//! CPU 合成器：对等 `GpuSynth` 的纯 CPU 渲染路径。
//!
//! 目标：行为对齐 xsynth（`parity` 测试逐样本对比），日后替代 xsynth 承担
//! CPU 合成。与 GPU 路径共用：
//! - `sfz_parser`：音色库解析与 (key, vel) 参数快照（`KeyInfo`）；
//! - `ChannelState`/`ChaseSkip`：CC/RPN/弯音/鼓组状态机与 chase 跳过；
//! - `SynthEvent`/`ControlEvent` 事件模型；
//! - `voice_render.wgsl` 的逐帧算法（CPU `voice.rs` 逐行复刻）。
//!
//! 与 GPU 路径的差异（有意）：
//! - `CpuVoice::time` 用 f64（对齐 xsynth `position: f64`，长曲无漂移）；
//! - 采样长度按帧计算（修正 GPU 立体声样本的 frames/elements 混用）；
//! - 事件按帧边界分段同步渲染（无 GPU 的段/指令流水线）；并行留待后续分片。

mod voice;

use std::path::PathBuf;

use yinhe_mixer::ChannelBuffers;

use crate::channel_state::{ChannelState, ChaseSkip, MAX_CHANNELS, is_env_effect_cc};
use crate::cpu_synth::voice::{CpuVoice, ENV_FINISHED, ENV_RELEASE};
use crate::gpu_synth::{ControlEvent, SynthEvent};
use crate::sfz_parser::{self, KeyMapEntry};

/// 默认全局 voice 上限（与 GpuSynth 一致）。
const DEFAULT_MAX_VOICES: usize = 8192;

/// dense 通道号 → 槽位索引；>= MAX_CHANNELS 返回 None（只支持 32 槽位）。
fn dense_channel(channel: usize) -> Option<usize> {
    (channel < MAX_CHANNELS).then_some(channel)
}

/// 纯 CPU 合成器（API 与 GpuSynth 对等）。
pub struct CpuSynth {
    sample_rate: u32,
    /// 每 dense 通道的音色库条目列表（dense 即槽位；与 GpuSynth 同结构）。
    port_key_maps: Vec<Vec<KeyMapEntry>>,
    channels: [ChannelState; MAX_CHANNELS],
    voices: Vec<CpuVoice>,
    /// 排序好的事件列表（NoteOn 自带 end_sample）。
    events: Vec<SynthEvent>,
    event_cursor: usize,
    /// 当前渲染位置（绝对 sample）。
    sample_position: u64,
    /// 最近一次 seek 的 event_cursor（chase_skip 的区间起点）。
    chase_base: usize,
    max_voices: usize,
    peak_voices: usize,
    /// 块内 damper 快照（voice 循环读取，避免借用冲突）。
    damper_flags: [bool; MAX_CHANNELS],
}

impl CpuSynth {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            port_key_maps: vec![Vec::new(); MAX_CHANNELS],
            channels: [ChannelState::new(sample_rate); MAX_CHANNELS],
            voices: Vec::new(),
            events: Vec::new(),
            event_cursor: 0,
            sample_position: 0,
            chase_base: 0,
            max_voices: DEFAULT_MAX_VOICES,
            peak_voices: 0,
            damper_flags: [false; MAX_CHANNELS],
        }
    }

    /// 加载某 dense 通道的音色库（登记 key map；CPU 无样本上传阶段）。
    pub fn load_dense_soundfonts(&mut self, dense: u32, paths: &[PathBuf]) -> Result<(), String> {
        let slot = dense as usize;
        if slot >= MAX_CHANNELS {
            return Err(format!("CPU 合成器仅支持 32 个通道（dense {dense} 超出）"));
        }
        let mut entries: Vec<KeyMapEntry> = Vec::new();
        for path in paths {
            let built = sfz_parser::build_key_maps(path, self.sample_rate)?;
            entries.extend(built);
        }
        self.port_key_maps[slot] = entries;
        Ok(())
    }

    /// 与 GpuSynth 对等的收尾钩子（CPU 无上传，保留以统一调用方流程）。
    pub fn finish_soundfont_load(&mut self) {}

    /// 批量加载事件（**要求按 sample 有序**，调用方负责），重置渲染位置到 0。
    pub fn load_events(&mut self, events: Vec<SynthEvent>) {
        debug_assert!(
            events.windows(2).all(|w| w[0].sample() <= w[1].sample()),
            "load_events 要求事件按 sample 有序（调用方负责排序）"
        );
        self.events = events;
        self.event_cursor = 0;
        self.voices.clear();
        self.channels = [ChannelState::new(self.sample_rate); MAX_CHANNELS];
        self.sample_position = 0;
        self.chase_base = 0;
    }

    pub fn sample_position(&self) -> u64 {
        self.sample_position
    }

    /// 当前活跃 voice 数（未结束）。
    pub fn voice_count(&self) -> usize {
        self.voices.iter().filter(|v| !v.finished()).count()
    }

    pub fn set_max_voices(&mut self, max: usize) {
        self.max_voices = max;
    }

    pub fn peak_voices(&self) -> usize {
        self.peak_voices
    }

    /// Seek 到指定位置：清 voice、重置通道状态、cursor 定位（与 GpuSynth 同语义）。
    pub fn seek(&mut self, sample: u64) {
        self.sample_position = sample;
        self.event_cursor = self.events.partition_point(|e| e.sample() < sample);
        self.chase_base = self.event_cursor;
        self.voices.clear();
        self.channels = [ChannelState::new(self.sample_rate); MAX_CHANNELS];
    }

    /// chase 跳过掩码（seek 后已实时处理的控制事件区间 `[chase_base, cursor)`）。
    pub fn chase_skip(&self) -> ChaseSkip {
        let mut skip = ChaseSkip::default();
        for ev in &self.events[self.chase_base..self.event_cursor] {
            let SynthEvent::Control { channel, event, .. } = ev else {
                continue;
            };
            let Some(ch) = dense_channel(*channel as usize) else {
                continue;
            };
            match event {
                ControlEvent::Raw(cc, _) => skip.cc_mask[ch] |= 1u128 << cc,
                ControlEvent::PitchBend(_) => skip.pitch_bend[ch] = true,
                ControlEvent::PitchBendSensitivity(_) => skip.pbs[ch] = true,
                ControlEvent::FineTune(_) => skip.fine_tune[ch] = true,
                ControlEvent::CoarseTune(_) => skip.coarse_tune[ch] = true,
                ControlEvent::ProgramChange(_) => skip.program[ch] = true,
                ControlEvent::PercussionMode(_) => {}
            }
        }
        skip
    }

    /// 应用 chase 通道状态快照（seek 后由外部驱动；frame = 0 → 下一块开头生效）。
    pub fn apply_chase(&mut self, dense: u32, events: &[ControlEvent]) {
        for &ev in events {
            self.process_control_channel(dense as u8, ev, 0);
        }
    }

    /// 投递一个事件（追加到事件队列）。`sample` 必须单调不减
    /// （引擎的 dispatch 按 tick/sample 顺序投递）；事件在渲染到其 sample
    /// 所在段起点时生效（与 GPU 的段边界语义一致）。
    pub fn send_event(&mut self, event: SynthEvent) {
        self.events.push(event);
    }

    /// 增量渲染 `[offset, offset + frames)` 到混音台 planar 缓冲（覆盖写本段区间）。
    ///
    /// 与 `render_to_mixer` 的差异：由调用方（engine 的逐段 dispatch）提供区间，
    /// `sample_position` 按 `frames` 推进；区间起点处消费所有已到事件。
    pub fn render_range(&mut self, buffers: &mut [ChannelBuffers], offset: usize, frames: usize) {
        if frames == 0 || buffers.is_empty() {
            return;
        }
        let sample_start = self.sample_position;

        // 覆盖语义：清零本段区间（与 ChannelSet::render_segment 一致）
        let n = buffers.len().min(MAX_CHANNELS);
        for buf in buffers.iter_mut().take(n) {
            buf.left[offset..offset + frames].fill(0.0);
            buf.right[offset..offset + frames].fill(0.0);
        }
        for buf in buffers.iter_mut().skip(n) {
            buf.left[offset..offset + frames].fill(0.0);
            buf.right[offset..offset + frames].fill(0.0);
        }

        for i in 0..MAX_CHANNELS {
            self.damper_flags[i] = self.channels[i].damper;
        }

        // 区间内按事件分段：事件在各自 sample 的帧生效（段 = 相邻事件之间）
        let range_end = sample_start + frames as u64;
        let mut fi = 0usize;
        while fi < frames {
            let sample = sample_start + fi as u64;
            while self.event_cursor < self.events.len()
                && self.events[self.event_cursor].sample() <= sample
            {
                let ev = self.events[self.event_cursor];
                self.event_cursor += 1;
                self.dispatch_event(&ev, fi as u32);
            }
            let next = self
                .events
                .get(self.event_cursor)
                .map(|e| e.sample())
                .unwrap_or(u64::MAX);
            let seg_end = next.min(range_end);
            let seg = (seg_end.saturating_sub(sample) as usize).min(frames - fi);
            if seg == 0 {
                // 同 sample 的事件已在上面消费完，next 必然 > sample；
                // 防御性推进避免死循环（异常事件数据兜底）。
                fi += 1;
                continue;
            }
            self.render_range_frames(buffers, offset + fi, fi, seg, sample);
            fi += seg;
        }

        // 段末：time 推进 + 清理结束 voice
        for v in self.voices.iter_mut() {
            v.advance_block(frames as u32);
        }
        self.voices.retain(|v| !v.finished());
        self.peak_voices = self.peak_voices.max(self.voice_count());
        self.sample_position = sample_start + frames as u64;
    }

    /// 渲染一整块（offset = 0；对等 GpuSynth 的 `render_to_mixer`）。
    pub fn render_to_mixer(&mut self, buffers: &mut [ChannelBuffers]) {
        let frames = buffers.first().map(|b| b.left.len()).unwrap_or(0);
        self.render_range(buffers, 0, frames);
    }

    /// 逐帧渲染 `[fi_start, fi_start + frames)`（区间内帧坐标，voice 的
    /// `start_offset` 同坐标系），输出直接累加进目标缓冲。字段级分离借用。
    fn render_range_frames(
        &mut self,
        buffers: &mut [ChannelBuffers],
        out_offset: usize,
        fi_start: usize,
        frames: usize,
        sample_start: u64,
    ) {
        let voices = &mut self.voices;
        let damper_flags = self.damper_flags;
        let n = buffers.len().min(MAX_CHANNELS);
        for i in 0..frames {
            let fi = fi_start + i;
            let sample = sample_start + i as u64;
            let out_i = out_offset + i;
            for v in voices.iter_mut() {
                // 到期释放（NoteOn 自带 end_sample；延音踏板按住时只标记）
                if !v.released && !v.held_by_damper && v.end_sample <= sample {
                    if damper_flags[v.channel as usize] {
                        v.held_by_damper = true;
                    } else {
                        v.signal_release(ENV_RELEASE);
                    }
                }
                let (l, r) = v.render_frame(fi as u32);
                if l != 0.0 || r != 0.0 {
                    let ch = v.channel as usize;
                    if ch < n {
                        buffers[ch].left[out_i] += l;
                        buffers[ch].right[out_i] += r;
                    }
                }
            }
        }
    }

    /// 事件派发（帧内；`frame` = 块内帧偏移）。
    fn dispatch_event(&mut self, ev: &SynthEvent, frame: u32) {
        match ev {
            SynthEvent::NoteOn {
                channel,
                key,
                velocity,
                end_sample,
                ..
            } => self.note_on(*channel, *key, *velocity, *end_sample, frame),
            SynthEvent::NoteOff { channel, key, .. } => self.note_off(*channel, *key),
            SynthEvent::Control { channel, event, .. } => {
                self.process_control_channel(*channel, *event, frame);
            }
        }
    }

    /// NoteOn：从 key map 快照创建 voice；超限淘汰最老的 release 中 voice。
    fn note_on(&mut self, channel: u8, key: u8, vel: u8, end_sample: u64, frame: u32) {
        let Some(ch_idx) = dense_channel(channel as usize) else {
            return;
        };
        let ch = self.channels[ch_idx];
        let entries = &self.port_key_maps[ch_idx];
        let Some(info) = sfz_parser::select_key_info_multi(entries, ch.bank, ch.program, key, vel)
        else {
            return;
        };
        self.voices.push(CpuVoice::new(
            info,
            channel,
            key,
            end_sample,
            frame,
            self.sample_rate,
            &ch,
        ));

        // 超限淘汰：优先杀最老的 release 中 voice，否则杀最老的（与 GpuSynth 同思路）
        while self.voices.len() > self.max_voices {
            let idx = self
                .voices
                .iter()
                .position(|v| v.env_stage == ENV_RELEASE)
                .unwrap_or(0);
            if !self.voices[idx].finished() {
                self.voices[idx].signal_release(ENV_FINISHED);
            } else {
                break; // 其余已被淘汰（块末统一清理）
            }
        }
    }

    /// NoteOff：释放该 (channel, key) 最老的未释放 voice（延音踏板按住时只标记）。
    fn note_off(&mut self, channel: u8, key: u8) {
        let Some(ch_idx) = dense_channel(channel as usize) else {
            return;
        };
        let damper = self.channels[ch_idx].damper;
        for v in self.voices.iter_mut() {
            if v.channel == channel
                && v.key == key
                && !v.finished()
                && !v.released
                && !v.held_by_damper
            {
                if damper {
                    v.held_by_damper = true;
                } else {
                    v.signal_release(ENV_RELEASE);
                }
                break;
            }
        }
    }

    /// 控制事件：更新通道状态并把变化传播到该通道的活跃 voice。
    fn process_control_channel(&mut self, channel: u8, event: ControlEvent, frame: u32) {
        let Some(ch_idx) = dense_channel(channel as usize) else {
            return;
        };
        let damper_released = self.channels[ch_idx].process_control(event);
        if damper_released {
            // 松开延音踏板：释放被保持的 voice（与 GpuSynth apply_chase 同语义）
            for v in self.voices.iter_mut() {
                if v.channel == channel && v.held_by_damper && !v.finished() {
                    v.held_by_damper = false;
                    v.signal_release(ENV_RELEASE);
                }
            }
        }
        // 弯音/调音变化：更新该通道活跃 voice 的速度（含 time 校正）
        if matches!(
            event,
            ControlEvent::PitchBend(_)
                | ControlEvent::PitchBendSensitivity(_)
                | ControlEvent::FineTune(_)
                | ControlEvent::CoarseTune(_)
        ) {
            let mult = self.channels[ch_idx].pitch_multiplier();
            for v in self.voices.iter_mut() {
                if v.channel == channel && !v.finished() {
                    v.set_speed(mult, frame);
                }
            }
        }
        // CC72/73/121：重算活跃 voice 的包络时长
        if is_env_effect_cc(&event) {
            let ch = self.channels[ch_idx];
            let sr = self.sample_rate;
            for v in self.voices.iter_mut() {
                if v.channel == channel && !v.finished() {
                    v.apply_env_update(&ch, sr);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffers(frames: usize) -> Vec<ChannelBuffers> {
        (0..2)
            .map(|_| ChannelBuffers {
                left: vec![0.0; frames],
                right: vec![0.0; frames],
            })
            .collect()
    }

    /// 无音色库时 note_on 不 panic、输出静音（select 落空静默）。
    #[test]
    fn note_on_without_soundfont_is_silent() {
        let mut synth = CpuSynth::new(48_000);
        synth.load_events(vec![SynthEvent::NoteOn {
            sample: 0,
            channel: 0,
            key: 60,
            velocity: 100,
            end_sample: 4800,
        }]);
        let mut bufs = buffers(512);
        synth.render_to_mixer(&mut bufs);
        assert_eq!(synth.voice_count(), 0);
        assert!(bufs.iter().all(|b| b.left.iter().all(|&v| v == 0.0)));
    }

    /// 事件在正确帧生效：NoteOn 在块中间（sample 256）时前半块静音。
    #[test]
    fn note_on_starts_at_event_frame() {
        let Some(sfz) = std::env::var_os("YINHE_TEST_SFZ") else {
            return; // 无测试音色库时跳过（CI）
        };
        let mut synth = CpuSynth::new(48_000);
        synth
            .load_dense_soundfonts(0, &[PathBuf::from(sfz)])
            .expect("load soundfont");
        synth.load_events(vec![SynthEvent::NoteOn {
            sample: 256,
            channel: 0,
            key: 60,
            velocity: 100,
            end_sample: 48_000,
        }]);
        let mut bufs = buffers(512);
        synth.render_to_mixer(&mut bufs);
        let head_energy: f32 = bufs[0].left[..256].iter().map(|v| v.abs()).sum();
        let tail_energy: f32 = bufs[0].left[256..].iter().map(|v| v.abs()).sum();
        assert_eq!(head_energy, 0.0, "起始帧前不得发声");
        assert!(tail_energy > 0.0, "起始帧后应有输出");
    }
}
