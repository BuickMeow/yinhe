use std::cmp::Reverse;
use std::sync::Arc;
use xsynth_core::channel::{
    ChannelAudioEvent, ChannelConfigEvent, ChannelEvent, ChannelInitOptions,
};
use xsynth_core::channel_group::{
    ChannelGroup, ChannelGroupConfig, ParallelismOptions, SynthEvent, SynthFormat,
};
use xsynth_core::soundfont::SoundfontBase;
use xsynth_core::{AudioPipe, AudioStreamParams, ChannelCount};

use crate::audio_model::SortedCC;
use crate::channel::{ChannelState, ChaseSkip};
use crate::channel_layout::ChannelLayout;

const STEREO_CHANNELS: usize = 2;

/// 为同一 channel 的一组目标位置增量 chase：`targets` 必须升序，只扫一遍 cc_events。
/// 返回与 `targets` 一一对应的状态快照（用于预览整组音符的目标位置自动化）。
/// 事件比较在 tick 域（cc_events 已按 tick 排序）。
pub(crate) fn chase_channel_states(
    cc_events: &[SortedCC],
    channel: u32,
    targets: &[u32],
) -> Vec<ChannelState> {
    let mut states = Vec::with_capacity(targets.len());
    let mut state = ChannelState::default();
    let mut cursor = 0usize;
    for &target in targets {
        while cursor < cc_events.len() && cc_events[cursor].tick < target {
            if cc_events[cursor].channel == channel {
                state.apply(&cc_events[cursor].event);
            }
            cursor += 1;
        }
        states.push(state);
    }
    states
}

/// 预览合成器：与主引擎完全独立的 ChannelGroup + 音色 + 状态。
///
/// - 预览音不占主引擎 voice、不改主引擎通道状态（播放中的自动化零干扰）；
/// - 音色与主引擎共享 `Arc<dyn SoundfontBase>`（零拷贝）；
/// - 渲染时钟独立；NoteOff 后按 `voice_count() > 0` 继续渲染，余音自然衰减完才停，
///   因此余音不会被截断，也不需要"余音时长"阈值。
pub(crate) struct PreviewEngine {
    channel_group: ChannelGroup,
    /// 源通道 → dense 通道映射（与主引擎同一布局，dense 索引一致）。
    dense_map: [u32; 256],
    /// 每 port 的音色（与主引擎共享 Arc）。
    port_sfs: [Vec<Arc<dyn SoundfontBase>>; 16],
    /// 渲染时钟（预览输出帧数累计，与主引擎 sample_position 独立）。
    position: u64,
    /// 活跃预览音。
    voices: Vec<PreviewVoice>,
    /// 待触发预览音（按目标位置相对时值错开，最早音符立即触发）。
    pending: Vec<PendingNote>,
}

/// 预览组中的一个音符（渲染器组装好后提交）。
pub(crate) struct PreviewNoteIn {
    pub(crate) channel: u8,
    pub(crate) key: u8,
    pub(crate) velocity: u8,
    pub(crate) duration: Option<u64>,
    /// 目标位置自动化状态。
    pub(crate) state: ChannelState,
    /// 目标位置 sample（组内相对时值依据）。
    pub(crate) target_sample: u64,
}

/// 待触发预览音。
struct PendingNote {
    channel: u8,
    key: u8,
    velocity: u8,
    duration: Option<u64>,
    state: ChannelState,
    /// 预览时钟到达该位置时触发（相对组内最早音符的帧数）。
    trigger_at: u64,
}

/// 一个活跃预览音。
struct PreviewVoice {
    channel: u8,
    key: u8,
    /// 渲染帧数上限；`None` = 持续音（等 `PreviewStop`）。
    duration: Option<u64>,
    /// NoteOn 时的 position（渲染时钟）。到期检查用相对时长
    /// `position - start_position`，绝不能用绝对 `position`——
    /// 预览引擎运行一段时间后 position 已累计很大，直接比较会让
    /// 所有音符在第一帧就"到期"（四分音符也只响几毫秒）。
    start_position: u64,
}

impl PreviewEngine {
    pub(crate) fn new(layout: &ChannelLayout, sample_rate: u32) -> Self {
        let compacted_channels = layout.compacted_channels();
        let config = ChannelGroupConfig {
            channel_init_options: ChannelInitOptions {
                fade_out_killing: true,
            },
            format: SynthFormat::Custom {
                channels: compacted_channels,
            },
            audio_params: AudioStreamParams {
                sample_rate,
                channels: ChannelCount::Stereo,
            },
            parallelism: ParallelismOptions::AUTO_PER_CHANNEL,
        };
        Self {
            channel_group: ChannelGroup::new(config),
            dense_map: std::array::from_fn(|ch| layout.dense_for(ch)),
            port_sfs: std::array::from_fn(|_| Vec::new()),
            position: 0,
            voices: Vec::new(),
            pending: Vec::new(),
        }
    }

    /// 同步某 port 的音色（与主引擎共享 Arc，零拷贝）。
    pub(crate) fn set_port_soundfonts(
        &mut self,
        port: u8,
        soundfonts: Vec<Arc<dyn SoundfontBase>>,
    ) {
        self.port_sfs[port as usize] = soundfonts;
    }

    /// 预览 NoteOn：设置通道音色 + 目标位置自动化状态（含 Program）+ NoteOn。
    pub(crate) fn note_on(
        &mut self,
        channel: u8,
        key: u8,
        velocity: u8,
        duration: Option<u64>,
        state: &ChannelState,
    ) {
        let dense = self.dense_map[channel as usize];
        if dense == u32::MAX {
            return;
        }
        // 音色与主引擎同源（同一 Arc，零拷贝）。
        let port = (channel >> 4) & 0x0F;
        if !self.port_sfs[port as usize].is_empty() {
            let sfs = self.port_sfs[port as usize].clone();
            self.channel_group.send_event(SynthEvent::Channel(
                dense,
                ChannelEvent::Config(ChannelConfigEvent::SetSoundfonts(sfs)),
            ));
        }
        // 目标位置自动化状态（volume/pan/Program/PBS 等）。
        state.send_to(dense, &mut self.channel_group, &ChaseSkip::default());
        self.channel_group.send_event(SynthEvent::Channel(
            dense,
            ChannelEvent::Audio(ChannelAudioEvent::NoteOn { key, vel: velocity }),
        ));
        self.voices.push(PreviewVoice {
            channel,
            key,
            duration,
            start_position: self.position,
        });
    }

    /// 提交整组预览（替换旧组）：组内音符按目标位置**相对时值**错开触发——
    /// 最早音符立即响，其余音符延迟（目标位置差）后响，各自响自己的 gate。
    /// 例如旋律音符 start 相差 4800 帧（已按目标位置 Tempo/PPQ 换算），
    /// 预览时就错开 4800 帧依次触发。时值差来自真实乐曲位置，不用固定毫秒封顶。
    pub(crate) fn preview_notes(&mut self, notes: Vec<PreviewNoteIn>) {
        self.stop_all();
        let min = notes.iter().map(|n| n.target_sample).min().unwrap_or(0);
        // trigger_at 存绝对位置（组开始 + 相对时值差）：position 是累计渲染帧数，
        // 若只存相对差，预览引擎跑过一段时间后 position 已超过所有 trigger_at，
        // 整组会在同一帧全部触发 → 时值差听不到。
        let base = self.position;
        self.pending = notes
            .into_iter()
            .map(|n| PendingNote {
                channel: n.channel,
                key: n.key,
                velocity: n.velocity,
                duration: n.duration,
                state: n.state,
                trigger_at: base + (n.target_sample - min),
            })
            .collect();
        // 降序排序：trigger_at 最小的在末尾，flush_pending 用 pop() O(1) 弹出。
        // （升序 + remove(0) 是 O(n²) 元素移动，大组预览——选框全选拖动时
        // 可能上万音符——会在音频渲染线程卡顿。）
        self.pending.sort_by_key(|p| Reverse(p.trigger_at));
        self.flush_pending();
    }

    /// 渲染一帧预览音频（不推进主引擎位置）。到期的定长音符在此 NoteOff
    /// （余音继续渲染，voice 自然衰减完才消失）。
    pub(crate) fn render(&mut self, output: &mut [f32]) {
        let frames = output.len() / STEREO_CHANNELS;
        if frames == 0 {
            return;
        }
        self.channel_group.read_samples(output);
        self.position += frames as u64;
        self.flush_pending();

        let mut i = 0;
        while i < self.voices.len() {
            let done = self.voices[i]
                .duration
                .is_some_and(|d| self.position.saturating_sub(self.voices[i].start_position) >= d);
            if done {
                let v = self.voices.swap_remove(i);
                self.note_off(v.channel, v.key);
            } else {
                i += 1;
            }
        }
    }

    /// 是否处于预览状态（有活跃预览音、待触发音符或余音仍在响）——
    /// 渲染器据此决定是否输出预览。
    ///
    /// 不能只看 `voice_count()`：NoteOn 后 voice 是延迟 spawn 的（渲染后才出现），
    /// 若渲染条件只看 voice 数量，未播放时第一帧就会提前退出、永不渲染 → 预览无声。
    pub(crate) fn previewing(&self) -> bool {
        !self.voices.is_empty() || !self.pending.is_empty() || self.channel_group.voice_count() > 0
    }

    /// 停止全部预览音（NoteOff 与待触发音符；余音继续渲染直到自然衰减完）。
    pub(crate) fn stop_all(&mut self) {
        while let Some(v) = self.voices.pop() {
            self.note_off(v.channel, v.key);
        }
        self.pending.clear();
    }

    /// 触发所有到点的待触发音符（pending 按 trigger_at 降序，末尾最早触发）。
    fn flush_pending(&mut self) {
        while self
            .pending
            .last()
            .is_some_and(|p| p.trigger_at <= self.position)
        {
            // last() 刚确认非空，pop 必成功；不用 unwrap（见 AGENTS 约定）。
            if let Some(p) = self.pending.pop() {
                self.note_on(p.channel, p.key, p.velocity, p.duration, &p.state);
            }
        }
    }

    fn note_off(&mut self, channel: u8, key: u8) {
        let dense = self.dense_map[channel as usize];
        if dense != u32::MAX {
            self.channel_group.send_event(SynthEvent::Channel(
                dense,
                ChannelEvent::Audio(ChannelAudioEvent::NoteOff { key }),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_on_keeps_whole_group_and_duration_expires() {
        let layout = ChannelLayout::from_mask(vec![true; 16]);
        let mut engine = PreviewEngine::new(&layout, 48000);

        // 整组两个音符（不同通道）同时 NoteOn → 都保留（回归：旧实现单音符状态只响最后一个）。
        engine.note_on(0, 60, 100, Some(4800), &ChannelState::default());
        engine.note_on(1, 64, 90, None, &ChannelState::default());
        assert_eq!(engine.voices.len(), 2);

        // 渲染 10 帧（512 帧/次 = 5120 帧）：定长的到期 NoteOff，持续音保留。
        let mut out = vec![0.0f32; 1024];
        for _ in 0..10 {
            engine.render(&mut out);
        }
        assert_eq!(engine.voices.len(), 1, "定长音符到期移除，持续音保留");

        engine.stop_all();
        assert!(engine.voices.is_empty());
    }

    #[test]
    fn stop_all_keeps_rendering_until_voices_decay() {
        // NoteOff 后 voice 仍活跃（余音），previewing 继续为真 —— 渲染器据此
        // 持续输出余音，而不是在 NoteOff 瞬间截断。
        let layout = ChannelLayout::from_mask(vec![true; 16]);
        let mut engine = PreviewEngine::new(&layout, 48000);
        engine.note_on(0, 60, 100, None, &ChannelState::default());
        engine.stop_all();
        assert!(engine.voices.is_empty());
        // 无音色库时 NoteOn 不产生 voice，previewing 为 false；有音色时由 voice_count 决定。
        assert!(!engine.previewing());
    }

    #[test]
    fn previewing_true_before_voice_spawns() {
        // 回归测试：NoteOn 后 voice 延迟 spawn（渲染后才出现），previewing 必须
        // 在 voices 非空时立即为真，否则未播放时渲染器第一帧就提前退出、预览无声。
        let layout = ChannelLayout::from_mask(vec![true; 16]);
        let mut engine = PreviewEngine::new(&layout, 48000);
        assert!(!engine.previewing());
        engine.note_on(0, 60, 100, None, &ChannelState::default());
        assert!(
            engine.previewing(),
            "NoteOn 后即使 voice 未 spawn 也必须为真"
        );
        // 定长到期 NoteOff 后余音仍在时也为真
        engine.note_on(0, 62, 100, Some(4800), &ChannelState::default());
        let mut out = vec![0.0f32; 1024];
        for _ in 0..30 {
            out.fill(0.0);
            engine.render(&mut out);
        }
        assert_eq!(engine.voices.len(), 1, "定长音符到期移除，持续音保留");
        assert!(engine.previewing(), "持续音还在");
        engine.stop_all();
        assert!(!engine.previewing(), "全部停止且无音色库（无 voice）后结束");
    }

    #[test]
    fn preview_notes_stagger_by_relative_position() {
        // 回归测试：移动多个不同起点的音符时，预览按目标位置相对时值错开触发，
        // 而不是所有音符同时演奏（旧实现忽略音符间的时值关系）。
        let layout = ChannelLayout::from_mask(vec![true; 16]);
        let mut engine = PreviewEngine::new(&layout, 48000);

        // 两个音符：B 比 A 晚 4800 帧（目标位置差）
        engine.preview_notes(vec![
            PreviewNoteIn {
                channel: 0,
                key: 60,
                velocity: 100,
                duration: None,
                state: ChannelState::default(),
                target_sample: 1000,
            },
            PreviewNoteIn {
                channel: 0,
                key: 64,
                velocity: 90,
                duration: None,
                state: ChannelState::default(),
                target_sample: 5800,
            },
        ]);
        assert_eq!(engine.voices.len(), 1, "最早音符立即触发");
        assert_eq!(engine.pending.len(), 1, "第二个音符延迟等待");

        let mut out = vec![0.0f32; 1024];
        // 渲染 9 帧（4608 帧 < 4800）：第二个音符仍未触发
        for _ in 0..9 {
            engine.render(&mut out);
        }
        assert_eq!(engine.voices.len(), 1, "时值差未到，不触发");
        assert_eq!(engine.pending.len(), 1);
        // 第 10 帧（5120 帧 >= 4800）：触发第二个音符
        engine.render(&mut out);
        assert_eq!(engine.voices.len(), 2, "时值差到达后触发");
        assert!(engine.pending.is_empty());

        // 全部停止：voices 与 pending 都清空
        engine.stop_all();
        assert!(engine.voices.is_empty());
        assert!(engine.pending.is_empty());
        assert!(!engine.previewing());
    }

    #[test]
    fn preview_notes_stagger_by_real_time_value() {
        // 用户报告：错开必须按目标位置的真实时值（BPM/tick/PPQ 换算的帧数），
        // 不能把远音符压到一个固定毫秒点一起触发（听感像同时和弦）。
        let layout = ChannelLayout::from_mask(vec![true; 16]);
        let mut engine = PreviewEngine::new(&layout, 48000);

        engine.preview_notes(vec![
            PreviewNoteIn {
                channel: 0,
                key: 60,
                velocity: 100,
                duration: None,
                state: ChannelState::default(),
                target_sample: 0,
            },
            PreviewNoteIn {
                channel: 0,
                key: 64,
                velocity: 90,
                duration: None,
                state: ChannelState::default(),
                target_sample: 100_000, // 真实时值差 100000 帧（> 2 秒）
            },
        ]);
        assert_eq!(engine.voices.len(), 1);
        assert_eq!(engine.pending.len(), 1);

        let mut out = vec![0.0f32; 1024];
        // 渲染 195 帧（99840 帧 < 100000）：第二音符必须还没触发
        // （若像旧实现那样封顶 150ms=7200 帧，这里早就全部触发了）。
        for _ in 0..195 {
            engine.render(&mut out);
        }
        assert_eq!(engine.voices.len(), 1, "真实时值差未到，不触发");
        // 第 196 帧（100352 >= 100000）：到点触发
        engine.render(&mut out);
        assert_eq!(engine.voices.len(), 2, "真实时值差到点后触发");
        assert!(engine.pending.is_empty());
    }

    #[test]
    fn note_duration_not_truncated_after_long_running() {
        // 回归测试（用户报告：普通四分音符也被判成很短的音符）：
        // 预览引擎运行一段时间（position 已累计）后 NoteOn 的定长音符，
        // 必须响满自己的 duration —— 到期判断用相对时长，而不是绝对 position。
        let layout = ChannelLayout::from_mask(vec![true; 16]);
        let mut engine = PreviewEngine::new(&layout, 48000);

        // 先跑 200 帧，position 累计 102400
        let mut out = vec![0.0f32; 1024];
        for _ in 0..200 {
            engine.render(&mut out);
        }

        // 四分音符 0.5s = 24000 帧
        engine.note_on(0, 60, 100, Some(24_000), &ChannelState::default());
        assert_eq!(engine.voices.len(), 1);

        // 渲染 20 帧（10240 帧 < 24000）：音符必须还在
        // （旧实现 position 102400 >= 24000，第一帧就把音符 NoteOff 了）。
        for _ in 0..20 {
            engine.render(&mut out);
        }
        assert_eq!(engine.voices.len(), 1, "四分音符响满时长，不能提前截断");

        // 渲染满 24000 帧（相对起点）后到期移除
        for _ in 0..30 {
            engine.render(&mut out);
        }
        assert!(engine.voices.is_empty(), "到时后正常 NoteOff");
    }

    #[test]
    fn preview_notes_stagger_after_long_running() {
        // 回归测试：预览引擎运行一段时间（position 已累计）后提交新组，
        // 时值差仍然生效（trigger_at 是绝对位置，不能被累计 position 吞掉）。
        let layout = ChannelLayout::from_mask(vec![true; 16]);
        let mut engine = PreviewEngine::new(&layout, 48000);

        // 先跑一段：position 累计到 100000 帧
        let mut out = vec![0.0f32; 1024];
        for _ in 0..200 {
            engine.render(&mut out);
        }

        engine.preview_notes(vec![
            PreviewNoteIn {
                channel: 0,
                key: 60,
                velocity: 100,
                duration: None,
                state: ChannelState::default(),
                target_sample: 5000,
            },
            PreviewNoteIn {
                channel: 0,
                key: 64,
                velocity: 90,
                duration: None,
                state: ChannelState::default(),
                target_sample: 10000,
            },
        ]);
        assert_eq!(engine.voices.len(), 1, "最早音符立即触发");
        assert_eq!(engine.pending.len(), 1);
        // 渲染 9 帧（+4608 < 5000）：第二个仍未触发
        for _ in 0..9 {
            engine.render(&mut out);
        }
        assert_eq!(engine.voices.len(), 1, "绝对位置基准下时值差仍生效");
        // 第 10 帧（+5120 >= 5000）：触发
        engine.render(&mut out);
        assert_eq!(engine.voices.len(), 2);
    }
}
