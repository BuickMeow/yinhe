/// Identifies an automatable parameter.
///
/// This enum is the unified key for all automation data — CC, PitchBend,
/// RPN, Velocity, and future VST parameters. Each variant maps to a lane
/// of `(tick, value)` events sorted by tick.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AutomationTarget {
    /// MIDI CC 0–127.
    CC { controller: u8 },
    /// MIDI Pitch Bend (0–16383, center 8192).
    PitchBend,
    /// RPN 0: Pitch Bend Sensitivity (in semitones, 0–24 typical).
    PitchBendSensitivity,
    /// RPN 1: Fine Tune (in cents, ±50 typical).
    FineTune,
    /// RPN 2: Coarse Tune (in semitones).
    CoarseTune,
    /// Per-note velocity (extracted from NoteOn events).
    Velocity,
    /// BPM / Tempo automation (from conductor track tempo_segments).
    Tempo,
    // Future: VSTParam { plugin_id: u32, param_id: u32 },
}

impl AutomationTarget {
    /// Maximum raw value for this target (used to normalize bar heights).
    pub fn max_value(&self) -> u16 {
        match self {
            AutomationTarget::CC { .. } => 127,
            AutomationTarget::PitchBend => 16383,
            AutomationTarget::PitchBendSensitivity => 127,
            AutomationTarget::FineTune => 100, // ±50 cents → range 100
            AutomationTarget::CoarseTune => 24,
            AutomationTarget::Velocity => 127,
            AutomationTarget::Tempo => 300, // BPM max
        }
    }

    /// Default / center value (used to draw a reference line).
    pub fn default_value(&self) -> u16 {
        match self {
            AutomationTarget::CC { .. } => 0,
            AutomationTarget::PitchBend => 8192,
            AutomationTarget::PitchBendSensitivity => 2,
            AutomationTarget::FineTune => 50, // center of 0..100
            AutomationTarget::CoarseTune => 0,
            AutomationTarget::Velocity => 0,
            AutomationTarget::Tempo => 120, // default BPM
        }
    }

    /// Whether this target has a non-zero center (PitchBend, FineTune).
    pub fn has_center_line(&self) -> bool {
        matches!(
            self,
            AutomationTarget::PitchBend | AutomationTarget::FineTune
        )
    }

    /// Human-readable display name for the dropdown.
    pub fn display_name(&self) -> String {
        match self {
            AutomationTarget::CC { controller } => {
                let name = cc_name(*controller);
                if name.is_empty() {
                    format!("CC {}", controller)
                } else {
                    format!("CC {} ({})", controller, name)
                }
            }
            AutomationTarget::PitchBend => "Pitch Bend".into(),
            AutomationTarget::PitchBendSensitivity => "PB Sensitivity (RPN 0)".into(),
            AutomationTarget::FineTune => "Fine Tune (RPN 1)".into(),
            AutomationTarget::CoarseTune => "Coarse Tune (RPN 2)".into(),
            AutomationTarget::Velocity => "Velocity".into(),
            AutomationTarget::Tempo => "Tempo (BPM)".into(),
        }
    }
}

/// Common MIDI CC names (standard GM/GS assignments).
fn cc_name(cc: u8) -> &'static str {
    match cc {
        0 => "Bank Select MSB",
        1 => "Mod Wheel",
        2 => "Breath",
        4 => "Foot",
        5 => "Portamento Time",
        6 => "Data Entry MSB",
        7 => "Volume",
        8 => "Balance",
        10 => "Pan",
        11 => "Expression",
        32 => "Bank Select LSB",
        38 => "Data Entry LSB",
        64 => "Sustain",
        65 => "Portamento",
        66 => "Sostenuto",
        67 => "Soft Pedal",
        68 => "Legato",
        71 => "Resonance",
        72 => "Release",
        73 => "Attack",
        74 => "Cutoff",
        84 => "Portamento Control",
        91 => "Reverb",
        92 => "Tremolo",
        93 => "Chorus",
        94 => "Detune",
        95 => "Phaser",
        100 => "RPN LSB",
        101 => "RPN MSB",
        _ => "",
    }
}

/// A single automation event: a value at a point in time.
#[derive(Clone, Debug)]
pub struct AutomationEvent {
    pub tick: u32,
    /// Raw value. Range depends on the target (0–127 for CC, 0–16383 for PB, etc.).
    pub value: u16,
    pub channel: u8,
    pub track: u16,
}

/// A sorted lane of automation events for one parameter.
#[derive(Clone, Debug)]
pub struct AutomationLane {
    pub target: AutomationTarget,
    /// Events sorted by `tick`.
    pub events: Vec<AutomationEvent>,
}

impl AutomationLane {
    /// Returns a slice of events whose `tick` falls in `[start_tick, end_tick)`.
    ///
    /// Uses binary search since `events` is sorted by tick.
    pub fn events_in_range(&self, start_tick: u32, end_tick: u32) -> &[AutomationEvent] {
        let lo = self.events.partition_point(|e| e.tick < start_tick);
        let hi = self.events.partition_point(|e| e.tick < end_tick);
        &self.events[lo..hi]
    }

    /// Chase: find the last event for `channel` before `target_tick`.
    ///
    /// Returns `None` if no event exists before the target tick for this channel.
    pub fn chase_value(&self, target_tick: u32, channel: u8) -> Option<u16> {
        // Binary search for the last event at or before target_tick on this channel.
        let idx = self.events.partition_point(|e| e.tick < target_tick);
        // Scan backwards to find a matching channel.
        self.events[..idx]
            .iter()
            .rposition(|e| e.channel == channel)
            .map(|i| self.events[i].value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_lane(target: AutomationTarget, ticks: &[u32]) -> AutomationLane {
        AutomationLane {
            target,
            events: ticks
                .iter()
                .map(|&t| AutomationEvent {
                    tick: t,
                    value: 64,
                    channel: 0,
                    track: 0,
                })
                .collect(),
        }
    }

    #[test]
    fn test_events_in_range() {
        let lane = make_lane(
            AutomationTarget::CC { controller: 7 },
            &[100, 200, 300, 400, 500],
        );
        let slice = lane.events_in_range(150, 450);
        assert_eq!(slice.len(), 3);
        assert_eq!(slice[0].tick, 200);
        assert_eq!(slice[2].tick, 400);
    }

    #[test]
    fn test_events_in_range_empty() {
        let lane = make_lane(AutomationTarget::CC { controller: 7 }, &[100, 200]);
        assert!(lane.events_in_range(300, 400).is_empty());
    }

    #[test]
    fn test_chase_value_found() {
        let mut lane = AutomationLane {
            target: AutomationTarget::CC { controller: 7 },
            events: vec![
                AutomationEvent {
                    tick: 100,
                    value: 80,
                    channel: 0,
                    track: 0,
                },
                AutomationEvent {
                    tick: 200,
                    value: 100,
                    channel: 0,
                    track: 0,
                },
                AutomationEvent {
                    tick: 300,
                    value: 60,
                    channel: 0,
                    track: 0,
                },
            ],
        };
        // Chase at tick 250 → should return value 100 (event at tick 200)
        assert_eq!(lane.chase_value(250, 0), Some(100));
        // Chase at tick 300 → should return value 100 (event before 300, not at 300)
        assert_eq!(lane.chase_value(300, 0), Some(100));
    }

    #[test]
    fn test_chase_value_none() {
        let lane = make_lane(AutomationTarget::CC { controller: 7 }, &[200, 300]);
        // Chase at tick 100 → no events before
        assert_eq!(lane.chase_value(100, 0), None);
    }

    #[test]
    fn test_chase_value_channel_filter() {
        let lane = AutomationLane {
            target: AutomationTarget::CC { controller: 7 },
            events: vec![
                AutomationEvent {
                    tick: 100,
                    value: 80,
                    channel: 0,
                    track: 0,
                },
                AutomationEvent {
                    tick: 100,
                    value: 90,
                    channel: 1,
                    track: 1,
                },
                AutomationEvent {
                    tick: 200,
                    value: 100,
                    channel: 0,
                    track: 0,
                },
            ],
        };
        assert_eq!(lane.chase_value(300, 0), Some(100));
        assert_eq!(lane.chase_value(300, 1), Some(90));
        assert_eq!(lane.chase_value(300, 2), None);
    }

    #[test]
    fn test_display_names() {
        assert_eq!(
            AutomationTarget::CC { controller: 7 }.display_name(),
            "CC 7 (Volume)"
        );
        assert_eq!(
            AutomationTarget::CC { controller: 99 }.display_name(),
            "CC 99"
        );
        assert_eq!(AutomationTarget::PitchBend.display_name(), "Pitch Bend");
        assert_eq!(AutomationTarget::Velocity.display_name(), "Velocity");
    }

    #[test]
    fn test_max_and_default_values() {
        assert_eq!(AutomationTarget::CC { controller: 0 }.max_value(), 127);
        assert_eq!(AutomationTarget::CC { controller: 0 }.default_value(), 0);
        assert_eq!(AutomationTarget::PitchBend.max_value(), 16383);
        assert_eq!(AutomationTarget::PitchBend.default_value(), 8192);
        assert!(AutomationTarget::PitchBend.has_center_line());
        assert!(!AutomationTarget::CC { controller: 0 }.has_center_line());
    }
}
