use xsynth_core::channel::{ChannelAudioEvent, ChannelEvent};
use xsynth_core::channel_group::SynthEvent;
use xsynth_core::AudioPipe;

use crate::audio_model::{ActiveNote, tick_to_sample};
use crate::engine::AudioEngine;

/// Number of output channels (stereo).
const STEREO_CHANNELS: usize = 2;

impl AudioEngine {
    pub(crate) fn render(&mut self, output: &mut [f32]) {
        let frames = output.len() / STEREO_CHANNELS;
        if frames == 0 || !self.playing {
            output.fill(0.0);
            return;
        }

        let block_start = self.sample_position;
        let block_end = block_start + frames as u64;
        let mut rendered_until = block_start;
        let mut offset_frames = 0usize;

        while rendered_until < block_end {
            // 单次 dispatch + find_next，128 桶只扫一遍
            let next_event_sample = self
                .dispatch_and_find_next(rendered_until, block_end)
                .unwrap_or(block_end)
                .max(rendered_until)
                .min(block_end);

            if next_event_sample > rendered_until {
                let segment_frames = (next_event_sample - rendered_until) as usize;
                let start = offset_frames * STEREO_CHANNELS;
                let end = (offset_frames + segment_frames) * STEREO_CHANNELS;
                self.channel_group.read_samples(&mut output[start..end]);
                rendered_until = next_event_sample;
                offset_frames += segment_frames;
            } else {
                // 所有事件已分发完，渲染剩余部分
                let remaining = block_end - rendered_until;
                let segment_frames = remaining as usize;
                let start = offset_frames * STEREO_CHANNELS;
                let end = (offset_frames + segment_frames) * STEREO_CHANNELS;
                self.channel_group.read_samples(&mut output[start..end]);
                rendered_until = block_end;
                break;
            }
        }

        self.sample_position = block_end;
    }

    /// 在 `sample` 位置分发所有事件（CC + NoteOn + NoteOff），
    /// 同时返回 `(sample, block_end)` 范围内下一个事件的位置。
    ///
    /// 合并了原来 `next_event_sample`、`dispatch_cc_until`、`dispatch_notes_at`
    /// 三个函数的职责，128 桶只扫描一次，消除重复的 `tick_to_sample` 计算。
    pub(crate) fn dispatch_and_find_next(&mut self, sample: u64, block_end: u64) -> Option<u64> {
        let mut next: Option<u64> = None;

        // ── CC 事件 ──
        while self.cc_cursor < self.cc_events.len()
            && self.cc_events[self.cc_cursor].sample <= sample
        {
            let cc = &self.cc_events[self.cc_cursor];
            let dense = self
                .channel_map
                .get(cc.channel as usize)
                .copied()
                .unwrap_or(u32::MAX);
            if dense != u32::MAX {
                self.channel_group
                    .send_event(SynthEvent::Channel(dense, ChannelEvent::Audio(cc.event)));
            }
            self.cc_cursor += 1;
        }
        if self.cc_cursor < self.cc_events.len() {
            let cc_sample = self.cc_events[self.cc_cursor].sample;
            if cc_sample < block_end {
                next = Some(next.map_or(cc_sample, |s| s.min(cc_sample)));
            }
        }

        // ── NoteOn + 找下一个 NoteOn 边界（单次 128 桶扫描）──
        if let Some(ref yin_model) = self.yin_model {
            let segments = &yin_model.tempo_map.tempo_segments;
            let tpb = yin_model.tempo_map.ticks_per_beat;
            let sr = self.sample_rate as f64;

            for key in 0..128usize {
                let notes = yin_model.notes[key].as_slice();
                let mut cursor = self.note_cursor[key];

                while cursor < notes.len() {
                    let note = &notes[cursor];
                    if note.velocity <= 1 {
                        cursor += 1;
                        continue;
                    }
                    let start_sample =
                        tick_to_sample(note.start_tick as u64, segments, tpb, sr);
                    if start_sample > sample {
                        // 该桶下一个待处理音符 → 记录为边界候选
                        if start_sample < block_end {
                            next = Some(next.map_or(start_sample, |s| s.min(start_sample)));
                        }
                        break;
                    }
                    // start_sample ≤ sample → dispatch NoteOn
                    let track = note.track as usize;
                    let ch = self
                        .model
                        .as_ref()
                        .map(|m| m.track_channel(track) as usize)
                        .unwrap_or(0);
                    if self.active_mask.get(ch).copied().unwrap_or(false)
                        && !self.skip_track.get(track).copied().unwrap_or(false)
                    {
                        let dense =
                            self.channel_map.get(ch).copied().unwrap_or(u32::MAX);
                        if dense != u32::MAX {
                            self.channel_group.send_event(SynthEvent::Channel(
                                dense,
                                ChannelEvent::Audio(ChannelAudioEvent::NoteOn {
                                    key: key as u8,
                                    vel: note.velocity,
                                }),
                            ));
                            let end_sample =
                                tick_to_sample(note.end_tick as u64, segments, tpb, sr);
                            self.active_notes.push(ActiveNote {
                                key: key as u8,
                                channel: ch as u8,
                                end_sample,
                            });
                        }
                    }
                    cursor += 1;
                }
                self.note_cursor[key] = cursor;
            }

            // ── NoteOff + 找下一个 NoteOff 边界（单次 active_notes 遍历）──
            let channel_map = &self.channel_map;
            self.ended_notes.clear();
            self.active_notes.retain(|an| {
                if an.end_sample <= sample {
                    self.ended_notes.push(*an);
                    false
                } else {
                    if an.end_sample < block_end {
                        next = Some(next.map_or(an.end_sample, |s| s.min(an.end_sample)));
                    }
                    true
                }
            });
            for an in &self.ended_notes {
                if let Some(&dense) = channel_map.get(an.channel as usize) {
                    if dense != u32::MAX {
                        self.channel_group.send_event(SynthEvent::Channel(
                            dense,
                            ChannelEvent::Audio(ChannelAudioEvent::NoteOff { key: an.key }),
                        ));
                    }
                }
            }
        }

        next
    }
}
