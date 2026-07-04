use serde::{Deserialize, Serialize};

/// Tempo event (BPM at a specific tick).
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct TempoEvent {
    pub tick: u32,
    pub bpm: f64,
}
