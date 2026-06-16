pub mod archive;
pub mod conversion;
pub mod events;
pub mod header;
pub mod paths;
pub mod schema;
mod varint;

// Re-export core types for backward compatibility
pub use archive::{ArchiveEntry, ProjectArchive, YIN_MAGIC, YIN_VERSION};
pub use events::{
    CcEvent, DeltaEvent, KeySigEvent, Note, PcEvent, PitchBendEvent, RpnEvent, SmpteOffsetEvent,
    TempoEvent, TextEvent, TimeSigEvent, NOTES_VERSION_DELTA_GATE, decode_delta_events,
    decode_notes_delta_gate, encode_delta_events, encode_notes_delta_gate,
};
pub use header::{FileHeader, InnerHeader, magic};
pub use paths::{cc_path, channel_label, conductor_path, pc_path, pitch_path, rpn_path,
                track_notes_path, track_prefix};
pub use schema::{
    ChannelMapping, MappingJson, PortMapping, ProjectJson, SfEntryJson, SfPortOverride,
    TrackMapping,
};
pub use varint::{read_varint, write_varint, zigzag_decode, zigzag_encode};
