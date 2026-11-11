//! 音频片段渲染（M0 从 `render.rs` 拆出；M1 接入 warp / 4 点插值）。
//!
//! 完全按块内绝对时间渲染，无跨块状态：seek / 暂停 / 导出天然正确。
//! 音频通道是覆盖写；PDC 由混音台统一补偿。

use std::collections::HashMap;
use std::sync::Arc;

use crate::audio_model::AudioModel;
use crate::audio_source::DecodedAudio;
use crate::channel_layout::ChannelLayout;
use yinhe_core::{TrackKind, YinModel};
use yinhe_mixer::MixerGraph;

/// 一帧立体声素材的 4 点 Catmull-Rom 插值。
///
/// `pos` 是素材内采样位置（可带小数）；越界按首尾样本 clamp，避免读越界。
fn cubic_sample(buffer: &[f32], pos: f64) -> f32 {
    if buffer.is_empty() {
        return 0.0;
    }
    let last = buffer.len() - 1;
    let base = pos.floor();
    let frac = pos - base;
    let i = base as isize;
    let idx = |k: isize| k.clamp(0, last as isize) as usize;
    let y0 = buffer[idx(i - 1)];
    let y1 = buffer[idx(i)];
    let y2 = buffer[idx(i + 1)];
    let y3 = buffer[idx(i + 2)];
    let t = frac as f32;
    let a = -0.5 * y0 + 1.5 * y1 - 1.5 * y2 + 0.5 * y3;
    let b = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
    let c = -0.5 * y0 + 0.5 * y2;
    ((a * t + b) * t + c) * t + y1
}

/// 音频片段渲染器：一条音频轨的所有 clip 混进对应音频通道。
pub(super) struct AudioClipRenderer<'a> {
    pub model: &'a YinModel,
    pub audio_model: &'a AudioModel,
    pub sources: &'a HashMap<String, Arc<DecodedAudio>>,
    pub mixer: &'a mut MixerGraph,
    pub layout: &'a ChannelLayout,
    pub skip_track: &'a [bool],
    pub sample_rate: f64,
}

impl AudioClipRenderer<'_> {
    pub(super) fn render_block(
        &mut self,
        block_start_sample: u64,
        offset_frames: usize,
        frames: usize,
    ) {
        if self.layout.audio_channels().is_empty() {
            return;
        }
        let sr = self.sample_rate;
        let block_start = block_start_sample as i64;
        let block_end = block_start + frames as i64;
        for (track_idx, track) in self.model.tracks.iter().enumerate() {
            if track.kind != TrackKind::Audio || track.audio_clips.is_empty() {
                continue;
            }
            if self.skip_track.get(track_idx).copied().unwrap_or(false) {
                continue;
            }
            let Some(ach) = track.audio_channel else {
                continue;
            };
            let dense = self.layout.audio_dense_for(ach);
            if dense == u32::MAX {
                continue;
            }
            let dense = dense as usize;
            for (ci, clip) in track.audio_clips.iter().enumerate() {
                let Some(pcm) = self.sources.get(&clip.source) else {
                    continue;
                };
                if pcm.frames == 0 || clip.duration_seconds <= 0.0 {
                    continue;
                }
                // 0 BPM 剪切：片段起点在剪切内 → 丢弃；跨剪切起点 → 截断；
                // 剪切之后的片段整体左移，与 tick/音符使用同一时间轴。
                let Some((adj_start, adj_end)) = self
                    .model
                    .tempo_map
                    .cut_clip_times(clip.start_seconds, clip.end_seconds())
                else {
                    continue;
                };
                let adj_duration = adj_end - adj_start;
                if adj_duration <= 0.0 {
                    continue;
                }
                let clip_start = (adj_start * sr).round() as i64;
                let clip_end = (adj_end * sr).round() as i64;
                let from = clip_start.max(block_start);
                let to = clip_end.min(block_end);
                if to <= from {
                    continue;
                }
                // 预计算的 (淡入, 淡出)：索引与 audio_clips 对齐，O(1) 无扫描。
                let Some(&(fade_in, fade_out)) = self
                    .audio_model
                    .clip_fades
                    .get(track_idx)
                    .and_then(|f| f.get(ci))
                else {
                    continue;
                };
                let Some(cb) = self.mixer.channel_buffers_mut(dense) else {
                    continue;
                };
                let out_end = offset_frames + frames;
                for g in from..to {
                    let n = offset_frames + (g - block_start) as usize;
                    if n >= out_end {
                        break;
                    }
                    // 片段内已播时长（秒）；淡入淡出与素材偏移都以它为准。
                    let p = g as f64 / sr - adj_start;
                    let gain =
                        clip.gain * yinhe_core::clip_fade_gain(p, adj_duration, fade_in, fade_out);
                    // 旧工程（无 warp）：素材按原速 1:1 播放。
                    let legacy_src = if clip.reversed {
                        clip.offset_seconds + (adj_duration - p)
                    } else {
                        clip.offset_seconds + p
                    };
                    let src_sec = clip
                        .source_seconds_at_progress(p / adj_duration, clip.reversed)
                        .unwrap_or(legacy_src);
                    if src_sec < 0.0 {
                        continue;
                    }
                    let src_pos = src_sec * sr;
                    if src_pos >= pcm.frames as f64 {
                        continue;
                    }
                    let left = cubic_sample(&pcm.left, src_pos);
                    let right = cubic_sample(&pcm.right, src_pos);
                    cb.left[n] += left * gain;
                    cb.right[n] += right * gain;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::cubic_sample;

    #[test]
    fn cubic_hits_integer_positions_exactly() {
        let buf = [0.0f32, 1.0, 4.0, 9.0];
        for (i, &want) in buf.iter().enumerate() {
            assert!((cubic_sample(&buf, i as f64) - want).abs() < 1e-5);
        }
    }

    #[test]
    fn cubic_interpolates_between_samples() {
        let buf = [0.0f32, 0.0, 1.0, 1.0];
        let mid = cubic_sample(&buf, 1.5);
        assert!(mid > 0.0 && mid < 1.0, "mid={mid}");
    }

    #[test]
    fn cubic_clamps_out_of_range() {
        let buf = [0.0f32, 0.5, 1.0];
        assert!((cubic_sample(&buf, -10.0) - 0.0).abs() < 1e-5);
        assert!((cubic_sample(&buf, 99.0) - 1.0).abs() < 1e-5);
    }
}
