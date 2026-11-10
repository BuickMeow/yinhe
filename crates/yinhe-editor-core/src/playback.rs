use std::time::Instant;

use yinhe_core::YinModel;

/// Playback state: tracks wall-clock time and converts to MIDI tick position.
pub struct PlaybackState {
    playing: bool,
    /// Wall-clock instant when playback started (or resumed).
    play_start_instant: Option<Instant>,
    /// The MIDI time (seconds) that corresponded to cursor_tick when playback started.
    play_start_time: f64,
    /// Playback speed multiplier (1.0 = normal).
    speed: f64,
    /// 循环开关。与 Events 行的 LOOP 行同源：行隐藏 = 关闭。
    loop_enabled: bool,
    /// 循环起点（tick，含）。
    loop_start_tick: u32,
    /// 循环终点（tick，不含）。
    loop_end_tick: u32,
}

impl Default for PlaybackState {
    fn default() -> Self {
        Self {
            playing: false,
            play_start_instant: None,
            play_start_time: 0.0,
            speed: 1.0,
            loop_enabled: false,
            loop_start_tick: 0,
            loop_end_tick: 0,
        }
    }
}

impl PlaybackState {
    pub fn is_playing(&self) -> bool {
        self.playing
    }

    pub fn speed(&self) -> f64 {
        self.speed
    }

    pub fn set_speed(&mut self, speed: f64) {
        self.speed = speed.max(0.1);
    }

    /// Toggle between play and pause.
    /// `cursor_tick` is the current cursor position when the action is triggered.
    pub fn toggle_play(&mut self, cursor_tick: f64, model: &YinModel) {
        if self.playing {
            // Pause: snapshot current position back to cursor_tick (handled by caller via current_tick)
            self.playing = false;
            self.play_start_instant = None;
        } else {
            // Start / resume
            self.play_start_time = model.tempo_map.tick_to_seconds(cursor_tick as u64);
            self.play_start_instant = Some(Instant::now());
            self.playing = true;
        }
    }

    /// 循环是否开启。
    pub fn loop_enabled(&self) -> bool {
        self.loop_enabled
    }

    /// 当前循环区间 `[start, end)`（tick）。
    pub fn loop_range(&self) -> (u32, u32) {
        (self.loop_start_tick, self.loop_end_tick)
    }

    /// 设置循环区间。`end <= start` 时拒绝并返回 false（保留原值）。
    pub fn set_loop_range(&mut self, start: u32, end: u32) -> bool {
        if end <= start {
            return false;
        }
        self.loop_start_tick = start;
        self.loop_end_tick = end;
        true
    }

    /// 开关循环。开启前区间必须有效（`end > start`），否则返回 false。
    pub fn set_loop_enabled(&mut self, enabled: bool) -> bool {
        if enabled && self.loop_end_tick <= self.loop_start_tick {
            return false;
        }
        self.loop_enabled = enabled;
        true
    }

    /// Stop playback and reset to beginning.
    pub fn stop(&mut self) {
        self.playing = false;
        self.play_start_instant = None;
        self.play_start_time = 0.0;
    }

    /// Compute the current tick position.
    /// Returns `None` when not playing (caller should keep its own cursor_tick).
    /// Returns `Some(tick)` when playing, and `reached_end` flag if playback hit the end.
    pub fn current_tick(&self, model: &YinModel) -> Option<(f64, bool)> {
        if !self.playing {
            return None;
        }
        let elapsed = self
            .play_start_instant
            .map(|inst| inst.elapsed().as_secs_f64() * self.speed)
            .unwrap_or(0.0);
        let current_time = self.play_start_time + elapsed;

        let mut tick = model.tempo_map.tick_at_time(current_time);
        // 循环开启：按区间长度取模，避免 UI 旋钮/时间码在循环外继续跑。
        // 注意这是 tick 域近似；播放头本身由引擎 sample position 驱动。
        if self.loop_enabled && self.loop_end_tick > self.loop_start_tick {
            let start = f64::from(self.loop_start_tick);
            let end = f64::from(self.loop_end_tick);
            if tick >= end {
                tick = start + (tick - start).rem_euclid(end - start);
            }
            return Some((tick.max(0.0), false));
        }
        // Always provide at least one bar of playable range
        let min_end = model.tempo_map.bar_divide();
        let end_tick = (model.tick_length as f64).max(min_end);

        if tick >= end_tick {
            Some((end_tick, true))
        } else {
            Some((tick.max(0.0), false))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_model() -> YinModel {
        let mut model = YinModel::default();
        model.meta.ppq = 480;
        // Pre-set tick_length so playback bounds work in tests.
        model.tick_length = 4800;
        model.rebuild();
        // rebuild() recomputes tick_length from notes; restore it.
        model.tick_length = 4800;
        // tempo_map already has a default 120 BPM segment after rebuild.
        model
    }

    #[test]
    fn default_is_not_playing() {
        let state = PlaybackState::default();
        assert!(!state.is_playing());
    }

    #[test]
    fn set_speed_clamps_minimum() {
        let mut state = PlaybackState::default();
        state.set_speed(0.01);
        assert!((state.speed() - 0.1).abs() < f64::EPSILON);
        state.set_speed(0.0);
        assert!((state.speed() - 0.1).abs() < f64::EPSILON);
    }

    #[test]
    fn loop_range_requires_positive_length() {
        let mut state = PlaybackState::default();
        assert!(!state.set_loop_range(480, 480));
        assert!(!state.set_loop_range(960, 480));
        assert_eq!(state.loop_range(), (0, 0));
        assert!(state.set_loop_range(0, 1920));
        assert_eq!(state.loop_range(), (0, 1920));
    }

    #[test]
    fn loop_cannot_enable_without_valid_range() {
        let mut state = PlaybackState::default();
        assert!(!state.set_loop_enabled(true));
        assert!(!state.loop_enabled());
        assert!(state.set_loop_range(0, 1920));
        assert!(state.set_loop_enabled(true));
        assert!(state.loop_enabled());
        assert!(state.set_loop_enabled(false));
        assert!(!state.loop_enabled());
    }

    #[test]
    fn toggle_play_sets_playing() {
        let mut state = PlaybackState::default();
        let model = make_test_model();
        state.toggle_play(0.0, &model);
        assert!(state.is_playing());
    }

    #[test]
    fn toggle_play_again_sets_paused() {
        let mut state = PlaybackState::default();
        let model = make_test_model();
        state.toggle_play(0.0, &model);
        state.toggle_play(0.0, &model);
        assert!(!state.is_playing());
    }

    #[test]
    fn stop_resets_to_beginning() {
        let mut state = PlaybackState::default();
        let model = make_test_model();
        state.toggle_play(0.0, &model);
        state.stop();
        assert!(!state.is_playing());
        assert_eq!(state.play_start_time, 0.0);
    }

    #[test]
    fn current_tick_none_when_not_playing() {
        let state = PlaybackState::default();
        let model = make_test_model();
        assert!(state.current_tick(&model).is_none());
    }
}
