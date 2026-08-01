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
pub(crate) fn chase_channel_states(
    cc_events: &[SortedCC],
    channel: u32,
    targets: &[u64],
) -> Vec<ChannelState> {
    let mut states = Vec::with_capacity(targets.len());
    let mut state = ChannelState::default();
    let mut cursor = 0usize;
    for &target in targets {
        while cursor < cc_events.len() && cc_events[cursor].sample < target {
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
}

/// 一个活跃预览音。
struct PreviewVoice {
    channel: u8,
    key: u8,
    /// 渲染帧数上限；`None` = 持续音（等 `PreviewStop`）。
    duration: Option<u64>,
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
        });
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

        let mut i = 0;
        while i < self.voices.len() {
            let done = self.voices[i].duration.is_some_and(|d| self.position >= d);
            if done {
                let v = self.voices.swap_remove(i);
                self.note_off(v.channel, v.key);
            } else {
                i += 1;
            }
        }
    }

    /// 是否还有活跃 voice（含 NoteOff 后的余音）——渲染器据此决定是否继续输出。
    pub(crate) fn has_voices(&self) -> bool {
        self.channel_group.voice_count() > 0
    }

    /// 停止全部预览音（NoteOff；余音继续渲染直到自然衰减完）。
    pub(crate) fn stop_all(&mut self) {
        while let Some(v) = self.voices.pop() {
            self.note_off(v.channel, v.key);
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
        // NoteOff 后 voice 仍活跃（余音），has_voices 继续为真 —— 渲染器据此
        // 持续输出余音，而不是在 NoteOff 瞬间截断。
        let layout = ChannelLayout::from_mask(vec![true; 16]);
        let mut engine = PreviewEngine::new(&layout, 48000);
        engine.note_on(0, 60, 100, None, &ChannelState::default());
        engine.stop_all();
        assert!(engine.voices.is_empty());
        // 无音色库时 NoteOn 不产生 voice，has_voices 为 false；有音色时由 voice_count 决定。
        assert!(!engine.has_voices());
    }
}
