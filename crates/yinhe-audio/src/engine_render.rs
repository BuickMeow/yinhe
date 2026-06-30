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
            let next_event_sample = self
                .next_event_sample(rendered_until, block_end)
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
            }

            if rendered_until >= block_end {
                break;
            }

            // CC/PB/RPN 在 render 之后发出，确保只在当前 sample 位置生效，
            // 不会影响之前已渲染的音频段。
            self.dispatch_cc_until(rendered_until);
            // NoteOn/NoteOff 在 render 之后发出，保证精确切分
            self.dispatch_notes_at(rendered_until);
        }

        self.sample_position = block_end;
    }

    /// 返回下一个需要切分渲染的位置（音符边界 + CC 事件位置）。
    pub(crate) fn next_event_sample(&self, rendered_until: u64, block_end: u64) -> Option<u64> {
        let mut next: Option<u64> = None;

        if let Some(ref yin_model) = self.yin_model {
            let segments = &yin_model.tempo_map.tempo_segments;
            let tpb = yin_model.tempo_map.ticks_per_beat;
            let sr = self.sample_rate as f64;

            for key in 0..128usize {
                let cursor = self.note_cursor[key];
                let notes = yin_model.notes[key].as_slice();
                let mut idx = cursor;
                while idx < notes.len() {
                    let n = &notes[idx];
                    if n.velocity <= 1 {
                        idx += 1;
                        continue;
                    }
                    let ch = self.model.as_ref().map(|m| m.track_channel(n.track as usize) as usize).unwrap_or(0);
                    if !self.active_mask.get(ch).copied().unwrap_or(false) {
                        idx += 1;
                        continue;
                    }
                    break;
                }
                if let Some(note) = notes.get(idx) {
                    let start_sample = tick_to_sample(note.start_tick as u64, segments, tpb, sr);
                    if start_sample >= rendered_until && start_sample < block_end {
                        next = Some(next.map_or(start_sample, |s| s.min(start_sample)));
                    }
                }
            }

            for note in &self.active_notes {
                if note.end_sample >= rendered_until && note.end_sample < block_end {
                    next = Some(next.map_or(note.end_sample, |s| s.min(note.end_sample)));
                }
            }
        }

        // 也考虑 CC 事件位置，确保快速变换的 CC 在正确的 sample 位置生效。
        if self.cc_cursor < self.cc_events.len() {
            let cc_sample = self.cc_events[self.cc_cursor].sample;
            if cc_sample >= rendered_until && cc_sample < block_end {
                next = Some(next.map_or(cc_sample, |s| s.min(cc_sample)));
            }
        }

        next
    }

    /// 批量发出 sample ≤ cutoff 的所有 CC / PitchBend / RPN 事件。
    pub(crate) fn dispatch_cc_until(&mut self, sample: u64) {
        while self.cc_cursor < self.cc_events.len() && self.cc_events[self.cc_cursor].sample <= sample {
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
    }

    /// 发出 sample 位置处的 NoteOn 和已结束的 NoteOff。
    pub(crate) fn dispatch_notes_at(&mut self, sample: u64) {
        if let Some(ref yin_model) = self.yin_model.clone() {
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
                    let start_sample = tick_to_sample(note.start_tick as u64, segments, tpb, sr);
                    if start_sample > sample {
                        break;
                    }
                    let track = note.track as usize;
                    let ch = self.model.as_ref().map(|m| m.track_channel(track) as usize).unwrap_or(0);
                    if !self.active_mask.get(ch).copied().unwrap_or(false) {
                        cursor += 1;
                        continue;
                    }
                    if !self.skip_track.get(track).copied().unwrap_or(false) {
                        let dense = self.channel_map.get(ch).copied().unwrap_or(u32::MAX);
                        if dense != u32::MAX {
                            self.channel_group.send_event(SynthEvent::Channel(
                                dense,
                                ChannelEvent::Audio(ChannelAudioEvent::NoteOn {
                                    key: key as u8,
                                    vel: note.velocity,
                                }),
                            ));
                            let end_sample = tick_to_sample(note.end_tick as u64, segments, tpb, sr);
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
        }

        let channel_map = &self.channel_map;
        self.ended_notes.clear();
        self.active_notes.retain(|an| {
            if an.end_sample <= sample {
                self.ended_notes.push(*an);
                false
            } else {
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
}