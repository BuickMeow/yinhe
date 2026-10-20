//! Tempo map: a standalone time-mapping structure.
//!
//! Migrated from `yinhe-midi::MidiFile`'s tempo/time methods. Built once
//! when a YinModel loads (or its conductor changes); used by both UI
//! (cursor display) and audio engine (tick to sample conversion).

use serde::{Deserialize, Serialize};
use yinhe_types::TimeSigEvent;

/// One segment of constant tempo.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct TempoSegment {
    pub start_tick: u32,
    pub start_time: f64,
    pub micros_per_quarter: u64,
}

pub const DEFAULT_MPQ: u64 = 500_000; // 120 BPM
pub const DEFAULT_BPM: f64 = 120.0;

#[inline]
pub fn bpm_from_mpq(mpq: u64) -> f64 {
    if mpq == 0 {
        return DEFAULT_BPM;
    }
    60_000_000.0 / mpq as f64
}

#[inline]
pub fn mpq_from_bpm(bpm: f32) -> u64 {
    if bpm <= 0.0 {
        return DEFAULT_MPQ;
    }
    (60_000_000.0 / bpm as f64).round() as u64
}

#[inline]
pub fn ticks_to_seconds(dtick: u64, ticks_per_beat: u32, mpq: u64) -> f64 {
    if ticks_per_beat == 0 {
        return 0.0;
    }
    dtick as f64 * mpq as f64 / (ticks_per_beat as f64 * 1_000_000.0)
}

#[inline]
pub fn seconds_to_ticks(seconds: f64, ticks_per_beat: u32, mpq: u64) -> f64 {
    if mpq == 0 {
        return 0.0;
    }
    seconds * ticks_per_beat as f64 * 1_000_000.0 / mpq as f64
}

#[inline]
pub fn bar_divide(ticks_per_beat: u32, num: u8, den_pow: u8) -> f64 {
    let n = num as f64;
    let d = (1u32 << den_pow) as f64;
    ticks_per_beat as f64 * n / d * 4.0
}

#[inline]
pub fn bar_at_tick(tick: u64, bar_div: f64) -> u64 {
    if bar_div <= 0.0 {
        return 1;
    }
    (tick as f64 / bar_div).floor() as u64 + 1
}

#[inline]
pub fn total_bars(tick_length: u64, bar_div: f64) -> u64 {
    if bar_div <= 0.0 {
        return 1;
    }
    (tick_length as f64 / bar_div).ceil() as u64
}

/// Recompute `start_time` for tempo segments based on cumulative tick deltas.
pub fn recompute_tempo_start_times(segments: &mut [TempoSegment], ticks_per_beat: u32) {
    if segments.is_empty() {
        return;
    }
    segments[0].start_time = 0.0;
    for i in 1..segments.len() {
        let prev = segments[i - 1];
        let dtick = (segments[i].start_tick - prev.start_tick) as u64;
        let elapsed = ticks_to_seconds(dtick, ticks_per_beat, prev.micros_per_quarter);
        segments[i].start_time = prev.start_time + elapsed;
    }
}

/// Standalone time-mapping structure derived from a `YinModel.conductor`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TempoMap {
    pub ticks_per_beat: u32,
    pub tempo_segments: Vec<TempoSegment>,
    pub time_sig_events: Vec<TimeSigEvent>,
    pub time_sig_default: (u8, u8),
    pub tick_length: u64,
    /// 0 BPM 剪切段 `[start, end)`：播放/音频导出直接跳过。
    ///
    /// `tempo_segments` 里对应的 `start` 处有一个 `micros_per_quarter == 0`
    /// 的“跳转段”，让 tick→时间映射在剪切区间内保持常量。
    #[serde(default)]
    pub skip_ranges: Vec<(u32, u32)>,
    /// 剪切段在**原始时间轴**上的秒区间（音频片段/音频导出用）。
    ///
    /// `(start, end)` 的 `end - start` 是被剪掉的墙钟时长；`end` 之后
    /// 的音频时间要整体左移这个时长。
    #[serde(default)]
    pub skip_time_ranges: Vec<(f64, f64)>,
}

impl Default for TempoMap {
    fn default() -> Self {
        Self {
            ticks_per_beat: 480,
            tempo_segments: Vec::new(),
            time_sig_events: Vec::new(),
            time_sig_default: (4, 2),
            tick_length: 0,
            skip_ranges: Vec::new(),
            skip_time_ranges: Vec::new(),
        }
    }
}

impl TempoMap {
    fn find_segment_at_time(&self, time: f64) -> Option<&TempoSegment> {
        if self.tempo_segments.is_empty() {
            return None;
        }
        let idx = self
            .tempo_segments
            .partition_point(|s| s.start_time <= time);
        if idx == 0 {
            return None;
        }
        Some(&self.tempo_segments[idx - 1])
    }

    /// Convert seconds to MIDI tick (considering tempo changes).
    pub fn tick_at_time(&self, time: f64) -> f64 {
        let Some(seg) = self.find_segment_at_time(time) else {
            return seconds_to_ticks(time, self.ticks_per_beat, DEFAULT_MPQ);
        };
        let dt = time - seg.start_time;
        seg.start_tick as f64 + seconds_to_ticks(dt, self.ticks_per_beat, seg.micros_per_quarter)
    }

    /// Get BPM at a given time (seconds).
    pub fn bpm_at_time(&self, time: f64) -> f32 {
        // 0 BPM 剪切时间区间内显示 0（事件本身的值）。
        if self
            .skip_time_ranges
            .iter()
            .any(|&(start, end)| time >= start && time < end)
        {
            return 0.0;
        }
        let Some(seg) = self.find_segment_at_time(time) else {
            return DEFAULT_BPM as f32;
        };
        bpm_from_mpq(seg.micros_per_quarter) as f32
    }

    /// Get BPM at a given tick（原生 tempo 事件优先，无事件时用兜底段）。
    pub fn bpm_at_tick(&self, tick: u64) -> f32 {
        // 0 BPM 剪切段内显示 0（事件本身的值），而不是前一个有效速度。
        if self.is_skipped(tick.min(u32::MAX as u64) as u32) {
            return 0.0;
        }
        let count = self
            .tempo_segments
            .partition_point(|s| (s.start_tick as u64) <= tick);
        // 0 BPM 剪切段用 mpq=0 标记，不是有效速度；回退到最近的正常段。
        self.tempo_segments[..count]
            .iter()
            .rev()
            .find(|s| s.micros_per_quarter > 0)
            .map(|s| bpm_from_mpq(s.micros_per_quarter) as f32)
            .unwrap_or(DEFAULT_BPM as f32)
    }

    /// Convert absolute tick to seconds (considering tempo changes).
    pub fn tick_to_seconds(&self, tick: u64) -> f64 {
        let count = self
            .tempo_segments
            .partition_point(|s| (s.start_tick as u64) <= tick);
        if count == 0 {
            return ticks_to_seconds(tick, self.ticks_per_beat, DEFAULT_MPQ);
        }
        let seg = &self.tempo_segments[count - 1];
        let dtick = tick - seg.start_tick as u64;
        seg.start_time + ticks_to_seconds(dtick, self.ticks_per_beat, seg.micros_per_quarter)
    }

    /// Time signature active at the given tick.
    pub fn time_sig_at_tick(&self, tick: u32) -> (u8, u8) {
        if self.time_sig_events.is_empty() {
            return self.time_sig_default;
        }
        let count = self.time_sig_events.partition_point(|e| e.tick <= tick);
        let idx = count.saturating_sub(1);
        let ev = &self.time_sig_events[idx];
        (ev.numerator, ev.denominator)
    }

    /// Ticks per measure (with default time signature).
    pub fn bar_divide(&self) -> f64 {
        bar_divide(
            self.ticks_per_beat,
            self.time_sig_default.0,
            self.time_sig_default.1,
        )
    }

    pub fn bar_at_tick(&self, tick: u64) -> u64 {
        bar_at_tick(tick, self.bar_divide())
    }

    pub fn total_bars(&self) -> u64 {
        total_bars(self.tick_length, self.bar_divide())
    }

    /// Total duration in seconds.
    pub fn duration_seconds(&self) -> f64 {
        self.tick_to_seconds(self.tick_length)
    }

    /// tick 是否落在 0 BPM 剪切段内。
    pub fn is_skipped(&self, tick: u32) -> bool {
        self.skip_ranges
            .iter()
            .any(|&(start, end)| tick >= start && tick < end)
    }

    /// 音频片段（秒）经过 0 BPM 剪切后的可播放区间：
    /// - 起点在剪切段内 → `None`（整段丢弃）；
    /// - 跨过下一个剪切起点 → 在剪切起点截断；
    /// - 剪切之后的片段整体左移。
    pub fn cut_clip_times(&self, start: f64, end: f64) -> Option<(f64, f64)> {
        let start = start.max(0.0);
        if self
            .skip_time_ranges
            .iter()
            .any(|&(a, b)| start >= a && start < b)
        {
            return None;
        }
        let mut end = end.max(start);
        if let Some(&(cut, _)) = self.skip_time_ranges.iter().find(|&&(cut, _)| cut > start)
            && cut < end
        {
            end = cut;
        }
        let shift: f64 = self
            .skip_time_ranges
            .iter()
            .filter(|&&(_, b)| b <= start)
            .map(|&(a, b)| b - a)
            .sum();
        Some(((start - shift).max(0.0), (end - shift).max(0.0)))
    }

    /// 音符可播放的结束 tick：
    /// - 起点在剪切段内 → `None`（整个音符不发声）；
    /// - 结束跨过下一个剪切起点 → 截断到剪切起点。
    pub fn playable_note_end(&self, start: u32, end: u32) -> Option<u32> {
        if self.is_skipped(start) {
            return None;
        }
        let mut out = end.max(start.saturating_add(1));
        if let Some(&(cut, _)) = self.skip_ranges.iter().find(|&&(cut, _)| cut > start)
            && cut < out
        {
            out = cut;
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_tempo_map_uses_defaults() {
        let tm = TempoMap::default();
        assert_eq!(tm.ticks_per_beat, 480);
        assert_eq!(tm.time_sig_default, (4, 2));
        assert!((tm.tick_to_seconds(480) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn tick_to_seconds_with_tempo_change() {
        let tm = TempoMap {
            ticks_per_beat: 480,
            tempo_segments: vec![
                TempoSegment {
                    start_tick: 0,
                    start_time: 0.0,
                    micros_per_quarter: DEFAULT_MPQ,
                },
                TempoSegment {
                    start_tick: 480,
                    start_time: 0.5,
                    micros_per_quarter: 250_000,
                },
            ],
            ..Default::default()
        };
        assert!((tm.tick_to_seconds(480) - 0.5).abs() < 1e-9);
        assert!((tm.tick_to_seconds(960) - 0.75).abs() < 1e-9);
    }

    #[test]
    fn tick_at_time_inverse_basic() {
        let tm = TempoMap {
            ticks_per_beat: 480,
            tempo_segments: vec![TempoSegment {
                start_tick: 0,
                start_time: 0.0,
                micros_per_quarter: DEFAULT_MPQ,
            }],
            ..Default::default()
        };
        assert!((tm.tick_at_time(0.5) - 480.0).abs() < 1e-6);
    }

    #[test]
    fn bpm_at_time_picks_segment() {
        let tm = TempoMap {
            ticks_per_beat: 480,
            tempo_segments: vec![
                TempoSegment {
                    start_tick: 0,
                    start_time: 0.0,
                    micros_per_quarter: DEFAULT_MPQ,
                },
                TempoSegment {
                    start_tick: 480,
                    start_time: 0.5,
                    micros_per_quarter: 250_000,
                },
            ],
            ..Default::default()
        };
        assert!((tm.bpm_at_time(0.5) - 240.0).abs() < 0.01);
    }

    #[test]
    fn bpm_at_tick_picks_segment() {
        let tm = TempoMap {
            ticks_per_beat: 480,
            tempo_segments: vec![
                TempoSegment {
                    start_tick: 0,
                    start_time: 0.0,
                    micros_per_quarter: DEFAULT_MPQ,
                },
                TempoSegment {
                    start_tick: 480,
                    start_time: 0.5,
                    micros_per_quarter: 250_000,
                },
            ],
            ..Default::default()
        };
        assert!((tm.bpm_at_tick(0) - 120.0).abs() < 0.01);
        assert!((tm.bpm_at_tick(479) - 120.0).abs() < 0.01);
        assert!((tm.bpm_at_tick(480) - 240.0).abs() < 0.01);
        assert!((tm.bpm_at_tick(48_000) - 240.0).abs() < 0.01);
    }

    #[test]
    fn time_sig_lookup_picks_latest() {
        let tm = TempoMap {
            time_sig_events: vec![
                TimeSigEvent {
                    tick: 0,
                    numerator: 4,
                    denominator: 2,
                },
                TimeSigEvent {
                    tick: 1920,
                    numerator: 3,
                    denominator: 2,
                },
            ],
            time_sig_default: (4, 2),
            ..Default::default()
        };
        assert_eq!(tm.time_sig_at_tick(0), (4, 2));
        assert_eq!(tm.time_sig_at_tick(1920), (3, 2));
        assert_eq!(tm.time_sig_at_tick(2000), (3, 2));
    }

    #[test]
    fn recompute_start_times() {
        let mut segs = vec![
            TempoSegment {
                start_tick: 0,
                start_time: 999.0,
                micros_per_quarter: DEFAULT_MPQ,
            },
            TempoSegment {
                start_tick: 480,
                start_time: 999.0,
                micros_per_quarter: 250_000,
            },
        ];
        recompute_tempo_start_times(&mut segs, 480);
        assert!((segs[0].start_time - 0.0).abs() < 1e-9);
        assert!((segs[1].start_time - 0.5).abs() < 1e-9);
    }
}
