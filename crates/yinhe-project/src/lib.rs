use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;

// ── Archive format ──

/// Magic bytes at the start of every .yin file.
pub const YIN_MAGIC: [u8; 4] = *b"YINH";
pub const YIN_VERSION: u32 = 1;

/// An entry inside the .yin archive.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArchiveEntry {
    /// Relative path, e.g. "conductor/tempo.zst" or "port_01/channel_01/abc123.zst".
    pub path: String,
    /// 8-byte self-describing header.
    pub header: FileHeader,
    /// Bincode-serialized event data (Vec<T> for the corresponding type).
    pub data: Vec<u8>,
}

/// The in-memory representation of a .yin project file.
#[derive(Clone, Debug, Default)]
pub struct ProjectArchive {
    pub entries: HashMap<String, ArchiveEntry>,
    /// zstd compression level for `write_to` (0 = default / 3).
    pub compression_level: i32,
}

impl ProjectArchive {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            compression_level: 0,
        }
    }

    pub fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Option<&ArchiveEntry> {
        self.entries.get(path)
    }

    pub fn get_events<T: serde::de::DeserializeOwned>(&self, path: &str) -> Option<Vec<T>> {
        let entry = self.entries.get(path)?;
        bincode::deserialize(&entry.data).ok()
    }

    pub fn set_events<T: Serialize>(
        &mut self,
        path: impl Into<String>,
        header: FileHeader,
        events: &[T],
    ) {
        let data = bincode::serialize(events).expect("bincode serialization failed");
        self.entries.insert(
            path.into(),
            ArchiveEntry {
                path: String::new(), // filled on write
                header,
                data,
            },
        );
    }

    /// Store a track's notes using the compact delta+gate varint encoding
    /// (FileHeader version is forced to NOTES_VERSION_DELTA_GATE).
    /// `notes` must be sorted by `start_tick`.
    pub fn set_notes(
        &mut self,
        path: impl Into<String>,
        mut header: FileHeader,
        notes: &[Note],
    ) {
        header.version = NOTES_VERSION_DELTA_GATE;
        let data = encode_notes_delta_gate(notes);
        self.entries.insert(
            path.into(),
            ArchiveEntry {
                path: String::new(),
                header,
                data,
            },
        );
    }

    /// Decode a notes entry, auto-detecting legacy bincode (version 1) vs
    /// delta+gate (version 2).
    pub fn get_notes(&self, path: &str) -> Option<Vec<Note>> {
        let entry = self.entries.get(path)?;
        if entry.header.version >= NOTES_VERSION_DELTA_GATE {
            Some(decode_notes_delta_gate(&entry.data))
        } else {
            bincode::deserialize(&entry.data).ok()
        }
    }

    pub fn remove(&mut self, path: &str) {
        self.entries.remove(path);
    }

    /// Write the archive to a .yin file (zstd compressed).
    pub fn write_to(&self, path: impl AsRef<Path>) -> std::io::Result<()> {
        let file = std::fs::File::create(path.as_ref())?;
        let mut writer = zstd::Encoder::new(file, self.compression_level)?;

        // Global header
        writer.write_all(&YIN_MAGIC)?;
        writer.write_all(&YIN_VERSION.to_le_bytes())?;

        // Build entry list with paths filled in
        let entries: Vec<ArchiveEntry> = self
            .entries
            .iter()
            .map(|(p, e)| ArchiveEntry {
                path: p.clone(),
                header: e.header,
                data: e.data.clone(),
            })
            .collect();

        let data = bincode::serialize(&entries).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::Other, e)
        })?;
        writer.write_all(&data)?;
        writer.finish()?;
        Ok(())
    }

    /// Read a .yin file into memory.
    pub fn read_from(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let file = std::fs::File::open(path.as_ref())?;
        let mut reader = zstd::Decoder::new(file)?;

        // Read and verify global header
        let mut magic = [0u8; 4];
        reader.read_exact(&mut magic)?;
        if magic != YIN_MAGIC {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "not a valid .yin file",
            ));
        }

        let mut version_buf = [0u8; 4];
        reader.read_exact(&mut version_buf)?;
        let _version = u32::from_le_bytes(version_buf);

        let mut rest = Vec::new();
        reader.read_to_end(&mut rest)?;
        let entries: Vec<ArchiveEntry> = bincode::deserialize(&rest).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::Other, e)
        })?;

        let map: HashMap<String, ArchiveEntry> = entries
            .into_iter()
            .map(|e| (e.path.clone(), e))
            .collect();

        Ok(Self {
            entries: map,
            compression_level: 0,
        })
    }
}

// ── File header ──

/// Magic bytes identifying each entry's data type.
pub mod magic {
    pub const TRACK_NOTES: [u8; 4] = *b"YHTK";
    pub const CC: [u8; 4] = *b"YHCC";
    pub const PC: [u8; 4] = *b"YHPC";
    pub const PITCH_BEND: [u8; 4] = *b"YHPB";
    pub const RPN: [u8; 4] = *b"YHRP";
    pub const TEMPO: [u8; 4] = *b"YHTM";
    pub const TIME_SIG: [u8; 4] = *b"YHTS";
    pub const KEY_SIG: [u8; 4] = *b"YHKS";
    pub const MARKER: [u8; 4] = *b"YHMK";
    pub const CUE: [u8; 4] = *b"YHCU";
    pub const LYRIC: [u8; 4] = *b"YHLY";
    pub const TEXT: [u8; 4] = *b"YHTX";
    pub const COPYRIGHT: [u8; 4] = *b"YHCP";
    pub const SMPTE_OFFSET: [u8; 4] = *b"YHSO";
}

/// 8-byte header for each entry, self-describing its type and origin.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct FileHeader {
    pub magic: [u8; 4],
    pub version: u8,
    pub port: u8,
    pub channel: u8,
    /// Type-specific: for track files this is the track index in mapping.json;
    /// for CC files this is the CC number; for RPN files this is the RPN number.
    pub extra: u8,
}

impl FileHeader {
    pub const SIZE: usize = 8;

    pub fn new(magic: [u8; 4], port: u8, channel: u8, extra: u8) -> Self {
        Self {
            magic,
            version: 1,
            port,
            channel,
            extra,
        }
    }
}

// ── Event types ──

/// A single note, stored per-track. Track/channel are in the file header.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Note {
    pub start_tick: u32,
    pub end_tick: u32,
    pub key: u8,
    pub velocity: u8,
}

/// FileHeader.version value indicating notes use the compact
/// delta-start + gate-duration varint encoding.
pub const NOTES_VERSION_DELTA_GATE: u8 = 2;

/// Encode notes as: count(varint) followed by per-note
/// (start_delta varint, duration varint, key u8, velocity u8).
/// `notes` must be sorted by `start_tick`.
pub fn encode_notes_delta_gate(notes: &[Note]) -> Vec<u8> {
    let mut out = Vec::with_capacity(notes.len() * 4 + 8);
    write_varint(&mut out, notes.len() as u64);
    let mut prev_start: u32 = 0;
    for n in notes {
        let delta = n.start_tick.saturating_sub(prev_start);
        let duration = n.end_tick.saturating_sub(n.start_tick);
        write_varint(&mut out, delta as u64);
        write_varint(&mut out, duration as u64);
        out.push(n.key);
        out.push(n.velocity);
        prev_start = n.start_tick;
    }
    out
}

/// Decode notes written by `encode_notes_delta_gate`.
pub fn decode_notes_delta_gate(buf: &[u8]) -> Vec<Note> {
    let mut cursor = 0usize;
    let count = match read_varint(buf, &mut cursor) {
        Some(c) => c as usize,
        None => return Vec::new(),
    };
    let mut notes = Vec::with_capacity(count);
    let mut prev_start: u32 = 0;
    for _ in 0..count {
        let Some(delta) = read_varint(buf, &mut cursor) else { break };
        let Some(duration) = read_varint(buf, &mut cursor) else { break };
        if cursor + 2 > buf.len() {
            break;
        }
        let key = buf[cursor];
        let velocity = buf[cursor + 1];
        cursor += 2;
        let start_tick = prev_start.wrapping_add(delta as u32);
        let end_tick = start_tick.wrapping_add(duration as u32);
        notes.push(Note {
            start_tick,
            end_tick,
            key,
            velocity,
        });
        prev_start = start_tick;
    }
    notes
}

/// LEB128-style unsigned varint writer.
fn write_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

/// LEB128-style unsigned varint reader. Returns None on truncation.
fn read_varint(buf: &[u8], cursor: &mut usize) -> Option<u64> {
    let mut result: u64 = 0;
    let mut shift: u32 = 0;
    loop {
        if *cursor >= buf.len() {
            return None;
        }
        let b = buf[*cursor];
        *cursor += 1;
        result |= ((b & 0x7F) as u64) << shift;
        if b & 0x80 == 0 {
            return Some(result);
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
}

/// A control change event, stored per-CC-number.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct CcEvent {
    pub tick: u32,
    pub value: u8,
}

/// A pitch bend event.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct PitchBendEvent {
    pub tick: u32,
    pub value: i16,
}

/// A program change event, stored per-channel.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct PcEvent {
    pub tick: u32,
    pub program: u8,
}

/// An RPN event, stored per-RPN-number.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct RpnEvent {
    pub tick: u32,
    pub value: u16,
}

// ── Conductor event types ──

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TempoEvent {
    pub tick: u32,
    pub bpm: f32,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct TimeSigEvent {
    pub tick: u32,
    pub numerator: u8,
    /// Denominator as power of 2 (e.g. 2 means 4).
    pub denominator_power: u8,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct KeySigEvent {
    pub tick: u32,
    /// Number of sharps (positive) or flats (negative).
    pub sf: i8,
    /// 0 = major, 1 = minor.
    pub mi: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TextEvent {
    pub tick: u32,
    pub text: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct SmpteOffsetEvent {
    pub tick: u32,
    pub hr: u8,
    pub mn: u8,
    pub se: u8,
    pub fr: u8,
    pub ff: u8,
}

// ── JSON structures ──

/// A single SoundFont entry stored in the project JSON.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SfEntryJson {
    pub path: String,
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// Project-level soundfont override for one port.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SfPortOverride {
    pub port: u8,
    pub entries: Vec<SfEntryJson>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectJson {
    pub version: u8,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub artist: String,
    /// Ticks per beat (quarter note).
    #[serde(default = "default_ppq")]
    pub ppq: u32,
    /// zstd compression level (0 = default / 3).
    #[serde(default = "default_zstd_level")]
    pub zstd_level: i32,
    /// Song description / notes.
    #[serde(default)]
    pub description: String,
    /// `true` = project mode (per-port SF).  `false` = global mode.
    #[serde(default)]
    pub soundfont_project_mode: bool,
    /// Per-port soundfont entries (only used in project mode).
    #[serde(default)]
    pub soundfont_overrides: Vec<SfPortOverride>,
}

fn default_ppq() -> u32 {
    480
}

fn default_zstd_level() -> i32 {
    0
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MappingJson {
    pub ports: Vec<PortMapping>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PortMapping {
    pub port: u8,
    pub channels: Vec<ChannelMapping>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChannelMapping {
    pub channel: u8,
    pub tracks: Vec<TrackMapping>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrackMapping {
    pub uuid: String,
    pub name: String,
    pub color: [f32; 3],
}

// ── Path helpers ──

/// Build the conductor entry path inside the archive.
pub fn conductor_path(name: &str) -> String {
    format!("conductor/{name}")
}

/// Build the port directory prefix.
pub fn port_prefix(port: u8) -> String {
    format!("port_{port:02}")
}

/// Build the channel directory prefix.
pub fn channel_prefix(port: u8, channel: u8) -> String {
    format!("port_{port:02}/channel_{channel:02}")
}

/// Build the full path for a track notes entry.
pub fn track_notes_path(port: u8, channel: u8, uuid: &str) -> String {
    format!("port_{port:02}/channel_{channel:02}/{uuid}.zst")
}

/// Build the full path for a CC entry.
pub fn cc_path(port: u8, channel: u8, cc_num: u8) -> String {
    format!("port_{port:02}/channel_{channel:02}/cc_{cc_num:03}.zst")
}

/// Build the full path for a pitch bend entry.
pub fn pitch_path(port: u8, channel: u8) -> String {
    format!("port_{port:02}/channel_{channel:02}/pitch.zst")
}

/// Build the full path for a program change entry (per-channel).
pub fn pc_path(port: u8, channel: u8) -> String {
    format!("port_{port:02}/channel_{channel:02}/pc.zst")
}

/// Build the full path for an RPN entry.
pub fn rpn_path(port: u8, channel: u8, rpn_num: u8) -> String {
    format!("port_{port:02}/channel_{channel:02}/rpn_{rpn_num}.zst")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_project() {
        let dir = std::env::temp_dir().join("yinhe_test_project");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.yin");

        let mut archive = ProjectArchive::new();

        // Add project.json
        let proj = ProjectJson {
            version: 1,
            name: "Test Song".into(),
            artist: "Test Artist".into(),
            ppq: 480,
            zstd_level: 0,
            description: String::new(),
        };
        archive.set_events(
            "project.json",
            FileHeader::new(*b"YHPR", 0, 0, 0),
            &[proj],
        );

        // Add track notes
        let notes = vec![
            Note {
                start_tick: 0,
                end_tick: 480,
                key: 60,
                velocity: 100,
            },
            Note {
                start_tick: 480,
                end_tick: 960,
                key: 64,
                velocity: 80,
            },
        ];
        archive.set_events(
            track_notes_path(1, 1, "abc123"),
            FileHeader::new(magic::TRACK_NOTES, 1, 1, 0),
            &notes,
        );

        // Add tempo
        let tempos = vec![
            TempoEvent {
                tick: 0,
                bpm: 120.0,
            },
            TempoEvent {
                tick: 1920,
                bpm: 140.0,
            },
        ];
        archive.set_events(
            conductor_path("tempo.zst"),
            FileHeader::new(magic::TEMPO, 0, 0, 0),
            &tempos,
        );

        // Write
        archive.write_to(&path).unwrap();

        // Read back
        let loaded = ProjectArchive::read_from(&path).unwrap();

        // Verify project.json
        let proj_events: Vec<ProjectJson> = loaded.get_events("project.json").unwrap();
        assert_eq!(proj_events[0].name, "Test Song");

        // Verify notes
        let note_events: Vec<Note> =
            loaded.get_events(&track_notes_path(1, 1, "abc123")).unwrap();
        assert_eq!(note_events.len(), 2);
        assert_eq!(note_events[0].start_tick, 0);
        assert_eq!(note_events[1].key, 64);

        // Verify tempo
        let tempo_events: Vec<TempoEvent> =
            loaded.get_events(&conductor_path("tempo.zst")).unwrap();
        assert_eq!(tempo_events.len(), 2);
        assert_eq!(tempo_events[1].bpm, 140.0);

        // Verify entry count
        assert_eq!(loaded.entries.len(), 3);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn path_helpers() {
        assert_eq!(conductor_path("tempo.zst"), "conductor/tempo.zst");
        assert_eq!(track_notes_path(1, 2, "abc"), "port_01/channel_02/abc.zst");
        assert_eq!(cc_path(1, 2, 7), "port_01/channel_02/cc_007.zst");
        assert_eq!(pitch_path(1, 2), "port_01/channel_02/pitch.zst");
        assert_eq!(rpn_path(1, 2, 0), "port_01/channel_02/rpn_0.zst");
    }

    #[test]
    fn delta_gate_roundtrip_basic() {
        let notes = vec![
            Note { start_tick: 0, end_tick: 480, key: 60, velocity: 100 },
            Note { start_tick: 240, end_tick: 720, key: 64, velocity: 90 },
            Note { start_tick: 480, end_tick: 960, key: 67, velocity: 80 },
            Note { start_tick: 1920, end_tick: 2400, key: 72, velocity: 110 },
        ];
        let encoded = encode_notes_delta_gate(&notes);
        let decoded = decode_notes_delta_gate(&encoded);
        assert_eq!(decoded.len(), notes.len());
        for (a, b) in notes.iter().zip(decoded.iter()) {
            assert_eq!(a.start_tick, b.start_tick);
            assert_eq!(a.end_tick, b.end_tick);
            assert_eq!(a.key, b.key);
            assert_eq!(a.velocity, b.velocity);
        }
    }

    #[test]
    fn delta_gate_empty() {
        let notes: Vec<Note> = Vec::new();
        let encoded = encode_notes_delta_gate(&notes);
        let decoded = decode_notes_delta_gate(&encoded);
        assert_eq!(decoded.len(), 0);
    }

    #[test]
    fn delta_gate_size_smaller_than_bincode_for_dense_notes() {
        // 1000 notes, each 1 tick apart, duration 10 — perfect for delta+gate.
        let mut notes = Vec::with_capacity(1000);
        for i in 0..1000u32 {
            notes.push(Note {
                start_tick: i,
                end_tick: i + 10,
                key: 60,
                velocity: 100,
            });
        }
        let dg = encode_notes_delta_gate(&notes);
        let bc = bincode::serialize(&notes).unwrap();
        // bincode fixint: 4+4+1+1 = 10 bytes per note + 8 length prefix = 10008
        // delta+gate: 1(count) + per-note (1+1+1+1) = ~4001
        assert!(dg.len() < bc.len(), "delta+gate {} should be smaller than bincode {}", dg.len(), bc.len());
        // Should be close to half
        assert!(dg.len() * 2 < bc.len(), "delta+gate {} should be < half of bincode {}", dg.len(), bc.len());
    }

    #[test]
    fn set_notes_via_archive_roundtrip() {
        let mut archive = ProjectArchive::new();
        let notes = vec![
            Note { start_tick: 100, end_tick: 200, key: 60, velocity: 100 },
            Note { start_tick: 300, end_tick: 400, key: 64, velocity: 90 },
        ];
        let path = track_notes_path(1, 1, "test");
        archive.set_notes(&path, FileHeader::new(magic::TRACK_NOTES, 1, 1, 0), &notes);

        // header.version should be auto-set to NOTES_VERSION_DELTA_GATE
        let entry = archive.entries.get(&path).unwrap();
        assert_eq!(entry.header.version, NOTES_VERSION_DELTA_GATE);

        let decoded = archive.get_notes(&path).unwrap();
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].start_tick, 100);
        assert_eq!(decoded[0].end_tick, 200);
        assert_eq!(decoded[1].key, 64);
    }
}
