use serde::{Deserialize, Serialize};

/// Program Change event. Bank MSB/LSB are stored alongside for SF2 mapping.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct PcEvent {
    pub tick: u32,
    pub program: u8,
    pub bank_msb: u8,
    pub bank_lsb: u8,
}
