//! yinhe CPU 预览合成器：独立 `CpuSynth` 实例，音色与主引擎共享 key map。
//!
//! 与 `XsynthPreview` 语义对齐（替换式/叠加式提交、块内精确触发与到期、
//! 余音自然衰减），但合成后端是 yinhe-synth 的 `CpuSynth`，音色走进程级
//! key map 缓存（与主引擎 `YinheCpu`/`YinheGpu` 共享同一份 `Arc`，不重复解析）。
//!
//! 主引擎为 yinhe 时，预览不再额外加载 xsynth 版音色库（省一份 ~GB 级内存）。

use std::cmp::Reverse;

use yinhe_mixer::ChannelBuffers;
use yinhe_synth::{CpuSynth, SynthEvent};

use super::{PreviewNoteIn, STEREO_CHANNELS};
use crate::channel::{ChannelState, ChaseSkip};
use crate::channel_layout::ChannelLayout;

/// 预览合成器：独立 CpuSynth + 预览时钟 + 活跃/待触发音符。
pub(crate) struct YinhePreview {
    synth: CpuSynth,
    /// 源通道 → dense（与主引擎同一布局）。
    dense_map: [u32; 256],
    /// planar 渲染缓冲（每 dense 一个；仅用前 compacted 个）。
    buffers: Vec<ChannelBuffers>,
    /// 预览时钟（渲染帧累计，与主引擎样本位置独立）。
    position: u64,
    /// 活跃预览音。
    voices: Vec<Voice>,
    /// 待触发预览音（按目标位置相对时值错开）。
    pending: Vec<Pending>,
}

struct Pending {
    channel: u8,
    key: u8,
    velocity: u8,
    duration: Option<u64>,
    state: ChannelState,
    trigger_at: u64,
}

struct Voice {
    channel: u8,
    key: u8,
    duration: Option<u64>,
    start_position: u64,
}

impl YinhePreview {
    pub(crate) fn new(layout: &ChannelLayout, sample_rate: u32, interpolation: u32) -> Self {
        let mut synth = CpuSynth::new(sample_rate);
        synth.set_interpolation(interpolation);
        let buffers = (0..layout.compacted_channels() as usize)
            .map(|_| ChannelBuffers {
                left: Vec::new(),
                right: Vec::new(),
            })
            .collect();
        Self {
            synth,
            dense_map: std::array::from_fn(|ch| layout.dense_for(ch)),
            buffers,
            position: 0,
            voices: Vec::new(),
            pending: Vec::new(),
        }
    }

    /// 按 dense 槽位加载 key map（走进程级缓存，与主引擎共享）。
    pub(crate) fn load_soundfonts(
        &mut self,
        denses: &[u32],
        paths: &[std::path::PathBuf],
    ) -> Result<(), String> {
        self.synth.load_dense_soundfonts_many(denses, paths)
    }

    /// 提交整组预览。语义与 `XsynthPreview::preview_notes` 一致。
    pub(crate) fn preview_notes(&mut self, notes: Vec<PreviewNoteIn>, exclusive: bool) {
        if exclusive {
            self.stop_all();
        }
        self.pending.clear();
        let min = notes.iter().map(|n| n.target_sample).min().unwrap_or(0);
        let base = self.position;
        self.pending = notes
            .into_iter()
            .map(|n| Pending {
                channel: n.channel,
                key: n.key,
                velocity: n.velocity,
                duration: n.duration,
                state: n.state,
                trigger_at: base + (n.target_sample - min),
            })
            .collect();
        // 降序：trigger_at 最小的在末尾，pop() O(1) 弹出。
        self.pending.sort_by_key(|p| Reverse(p.trigger_at));
        self.flush_pending_at(self.position);
    }

    pub(crate) fn render(&mut self, output: &mut [f32]) {
        let frames = output.len() / STEREO_CHANNELS;
        if frames == 0 {
            return;
        }
        output.fill(0.0);
        let block_end = self.position + frames as u64;
        let mut offset_frames = 0usize;
        let mut cursor = self.position;

        while cursor < block_end {
            self.flush_pending_at(cursor);
            self.expire_voices_at(cursor);
            let next = self.next_event_boundary(block_end);
            let seg_frames = (next - cursor) as usize;
            self.render_segment(output, offset_frames, seg_frames);
            offset_frames += seg_frames;
            cursor = next;
        }
        self.flush_pending_at(block_end);
        self.expire_voices_at(block_end);
        self.position = block_end;
    }

    pub(crate) fn previewing(&self) -> bool {
        !self.voices.is_empty() || !self.pending.is_empty() || self.synth.voice_count() > 0
    }

    pub(crate) fn stop_all(&mut self) {
        while let Some(v) = self.voices.pop() {
            self.note_off(v.channel, v.key);
        }
        self.pending.clear();
    }

    pub(crate) fn stop_key(&mut self, key: u8) {
        self.pending.retain(|p| p.key != key);
        let mut i = 0;
        while i < self.voices.len() {
            if self.voices[i].key == key {
                let v = self.voices.swap_remove(i);
                self.note_off(v.channel, v.key);
            } else {
                i += 1;
            }
        }
    }

    /// 渲染 `frames` 帧到 planar buffers，再下混成交错 stereo 写入 `output`。
    fn render_segment(&mut self, output: &mut [f32], frame_offset: usize, frames: usize) {
        if frames == 0 {
            return;
        }
        for b in &mut self.buffers {
            if b.left.len() < frames {
                b.left.resize(frames, 0.0);
                b.right.resize(frames, 0.0);
            }
        }
        if self.buffers.is_empty() {
            return;
        }
        self.synth.render_range(&mut self.buffers, 0, frames);
        for i in 0..frames {
            let mut l = 0.0f32;
            let mut r = 0.0f32;
            for b in &self.buffers {
                l += b.left[i];
                r += b.right[i];
            }
            let o = (frame_offset + i) * STEREO_CHANNELS;
            output[o] += l;
            output[o + 1] += r;
        }
    }

    fn flush_pending_at(&mut self, pos: u64) {
        while self.pending.last().is_some_and(|p| p.trigger_at <= pos) {
            if let Some(p) = self.pending.pop() {
                self.note_on_at(
                    p.channel,
                    p.key,
                    p.velocity,
                    p.duration,
                    &p.state,
                    p.trigger_at,
                );
            }
        }
    }

    fn note_on_at(
        &mut self,
        channel: u8,
        key: u8,
        velocity: u8,
        duration: Option<u64>,
        state: &ChannelState,
        start_position: u64,
    ) {
        let dense = self.dense_map[channel as usize];
        if dense == u32::MAX {
            return;
        }
        let sample = start_position;
        // 目标位置自动化状态：ChannelState → yinhe 控制事件（含 Program/RPN/CC）。
        let skip = ChaseSkip::default();
        for ev in state.events_to_send(channel as usize, &skip) {
            if let Some(ce) = crate::engine_gpu::to_backend_control_event(&ev) {
                self.synth.send_event(SynthEvent::Control {
                    sample,
                    channel: dense as u8,
                    event: ce,
                });
            }
        }
        let end_sample = duration
            .map(|d| sample.saturating_add(d))
            .unwrap_or(u64::MAX);
        self.synth.send_event(SynthEvent::NoteOn {
            sample,
            channel: dense as u8,
            key,
            velocity,
            end_sample,
        });
        self.voices.push(Voice {
            channel,
            key,
            duration,
            start_position,
        });
    }

    fn note_off(&mut self, channel: u8, key: u8) {
        let dense = self.dense_map[channel as usize];
        if dense != u32::MAX {
            self.synth.send_event(SynthEvent::NoteOff {
                sample: self.position,
                channel: dense as u8,
                key,
            });
        }
    }

    fn next_event_boundary(&self, block_end: u64) -> u64 {
        let mut next = block_end;
        if let Some(p) = self.pending.last() {
            next = next.min(p.trigger_at);
        }
        for v in &self.voices {
            if let Some(d) = v.duration {
                next = next.min(v.start_position.saturating_add(d));
            }
        }
        next
    }

    fn expire_voices_at(&mut self, pos: u64) {
        let mut i = 0;
        while i < self.voices.len() {
            let due = self.voices[i]
                .duration
                .is_some_and(|d| pos.saturating_sub(self.voices[i].start_position) >= d);
            if due {
                let v = self.voices.swap_remove(i);
                self.note_off(v.channel, v.key);
            } else {
                i += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// 写一个 48kHz 单声道正弦 wav + 引用它的 sfz，返回 sfz 路径。
    fn write_tone(dir: &std::path::Path) -> PathBuf {
        let wav_path = dir.join("tone.wav");
        let sfz_path = dir.join("tone.sfz");
        let sr = 48_000u32;
        let len = 48_000usize;
        let mut w = hound::WavWriter::create(
            &wav_path,
            hound::WavSpec {
                channels: 1,
                sample_rate: sr,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for i in 0..len {
            let s = (2.0 * std::f32::consts::PI * 440.0 * i as f32 / sr as f32).sin() * 0.6;
            w.write_sample((s * i16::MAX as f32) as i16).unwrap();
        }
        w.finalize().unwrap();
        std::fs::write(&sfz_path, "<region>\nsample=tone.wav key=60\n").unwrap();
        sfz_path
    }

    #[test]
    fn yinhe_preview_produces_audio() {
        let dir = tempfile::tempdir().unwrap();
        let sfz = write_tone(dir.path());

        let layout = ChannelLayout::from_mask(vec![true; 16]);
        let mut p = YinhePreview::new(&layout, 48_000, 0);
        p.load_soundfonts(&[0], &[sfz]).unwrap();
        p.preview_notes(
            vec![PreviewNoteIn {
                channel: 0,
                key: 60,
                velocity: 100,
                duration: None,
                state: ChannelState::default(),
                target_sample: 0,
            }],
            true,
        );
        assert!(p.previewing(), "NoteOn 后应立即视为预览中");

        let mut out = vec![0.0f32; 1024];
        let mut peak = 0.0f32;
        for _ in 0..30 {
            out.fill(0.0);
            p.render(&mut out);
            for v in &out {
                peak = peak.max(v.abs());
            }
        }
        assert!(peak > 0.0, "yinhe 预览应产出可听信号（peak={peak}）");

        p.stop_all();
        assert!(p.voices.is_empty(), "stop_all 清空活跃音");
        assert!(p.pending.is_empty(), "stop_all 清空待触发音");
    }
}
