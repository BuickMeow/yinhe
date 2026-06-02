use std::time::Instant;

use yinhe_midi::MidiFile;

/// Playback state: tracks wall-clock time and converts to MIDI tick position.
pub struct PlaybackState {
    playing: bool,
    /// Wall-clock instant when playback started (or resumed).
    play_start_instant: Option<Instant>,
    /// The MIDI time (seconds) that corresponded to cursor_tick when playback started.
    play_start_time: f64,
    /// Playback speed multiplier (1.0 = normal).
    speed: f64,
}

impl Default for PlaybackState {
    fn default() -> Self {
        Self {
            playing: false,
            play_start_instant: None,
            play_start_time: 0.0,
            speed: 1.0,
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
    pub fn toggle_play(&mut self, cursor_tick: f64, midi: &MidiFile) {
        if self.playing {
            // Pause: snapshot current position back to cursor_tick (handled by caller via current_tick)
            self.playing = false;
            self.play_start_instant = None;
        } else {
            // Start / resume
            self.play_start_time = midi.tick_to_seconds(cursor_tick as u32);
            self.play_start_instant = Some(Instant::now());
            self.playing = true;
        }
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
    pub fn current_tick(&self, midi: &MidiFile) -> Option<(f64, bool)> {
        if !self.playing {
            return None;
        }
        let elapsed = self
            .play_start_instant
            .map(|inst| inst.elapsed().as_secs_f64() * self.speed)
            .unwrap_or(0.0);
        let current_time = self.play_start_time + elapsed;

        let tick = midi.tick_at_time(current_time);
        let end_tick = midi.tick_length as f64;

        if tick >= end_tick {
            Some((end_tick, true))
        } else {
            Some((tick.max(0.0), false))
        }
    }
}
