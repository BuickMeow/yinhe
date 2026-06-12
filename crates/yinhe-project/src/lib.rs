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

    /// Store conductor-scoped events using compact delta encoding (no inner header).
    pub fn set_delta_events<T: DeltaEvent>(
        &mut self,
        path: impl Into<String>,
        header: FileHeader,
        events: &[T],
    ) {
        let data = encode_delta_events(events);
        self.entries.insert(
            path.into(),
            ArchiveEntry {
                path: String::new(),
                header,
                data,
            },
        );
    }

    /// Read conductor-scoped events written by `set_delta_events`.
    pub fn get_delta_events<T: DeltaEvent>(&self, path: &str) -> Option<Vec<T>> {
        let entry = self.entries.get(path)?;
        Some(decode_delta_events(&entry.data))
    }

    /// Store track-scoped events with an inner header using compact delta encoding.
    pub fn set_delta_events_with_inner<T: DeltaEvent>(
        &mut self,
        path: impl Into<String>,
        header: FileHeader,
        inner: InnerHeader,
        events: &[T],
    ) {
        let body = encode_delta_events(events);
        let mut data = Vec::with_capacity(InnerHeader::SIZE + body.len());
        inner.write(&mut data);
        data.extend_from_slice(&body);
        self.entries.insert(
            path.into(),
            ArchiveEntry {
                path: String::new(),
                header,
                data,
            },
        );
    }

    /// Read track-scoped events written by `set_delta_events_with_inner`.
    pub fn get_delta_events_with_inner<T: DeltaEvent>(
        &self,
        path: &str,
    ) -> Option<(InnerHeader, Vec<T>)> {
        let entry = self.entries.get(path)?;
        let (inner, rest) = InnerHeader::read(&entry.data)?;
        let events = decode_delta_events(rest);
        Some((inner, events))
    }

    /// Store a track's notes using the compact delta encoding
    /// (FileHeader version is forced to NOTES_VERSION_DELTA_GATE). The
    /// 3-byte InnerHeader is prepended to the payload.
    /// `notes` must be sorted by `start_tick`.
    pub fn set_notes(
        &mut self,
        path: impl Into<String>,
        mut header: FileHeader,
        inner: InnerHeader,
        notes: &[Note],
    ) {
        header.version = NOTES_VERSION_DELTA_GATE;
        let body = encode_delta_events(notes);
        let mut data = Vec::with_capacity(InnerHeader::SIZE + body.len());
        inner.write(&mut data);
        data.extend_from_slice(&body);
        self.entries.insert(
            path.into(),
            ArchiveEntry {
                path: String::new(),
                header,
                data,
            },
        );
    }

    /// Decode a notes entry. Returns `(inner_header, notes)`.
    pub fn get_notes(&self, path: &str) -> Option<(InnerHeader, Vec<Note>)> {
        let entry = self.entries.get(path)?;
        let (inner, rest) = InnerHeader::read(&entry.data)?;
        let notes = decode_delta_events(rest);
        Some((inner, notes))
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

// ── Inner payload header ──
//
// Every track-scoped event/notes payload starts with a 3-byte inner header
// that identifies which logical track and global channel the data belongs to.
// The archive path itself uses a UUID to avoid collisions when multiple
// tracks share the same (port, channel) — the inner header makes each zst
// file self-describing without needing mapping.json.
//
// Layout:
//   [track_index: u16 LE][channel: u8]
//
// `channel` is the combined `port * 16 + raw_midi_channel` (0..=127), so
// the inner header alone fully identifies (track, port, channel). The path
// does not redundantly encode port/channel — only the UUID.

/// 3-byte inner header at the start of every track-scoped zst payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InnerHeader {
    /// 0-based MIDI track index.
    pub track_index: u16,
    /// Combined `port * 16 + raw_channel`, range 0..=127.
    /// `port = channel >> 4`, `raw_channel = channel & 0x0F`.
    pub channel: u8,
}

impl InnerHeader {
    pub const SIZE: usize = 3;

    pub fn new(track_index: u16, channel: u8) -> Self {
        Self { track_index, channel }
    }

    /// Combined port from `channel` (high nibble of the global channel byte).
    pub fn port(&self) -> u8 {
        self.channel >> 4
    }

    /// Raw MIDI channel 0..=15 (low nibble of the global channel byte).
    pub fn raw_channel(&self) -> u8 {
        self.channel & 0x0F
    }

    pub fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.track_index.to_le_bytes());
        out.push(self.channel);
    }

    /// Parse the inner header from the start of `buf`.
    /// Returns the parsed header and the rest of the buffer.
    pub fn read(buf: &[u8]) -> Option<(Self, &[u8])> {
        if buf.len() < Self::SIZE {
            return None;
        }
        let track_index = u16::from_le_bytes([buf[0], buf[1]]);
        let channel = buf[2];
        Some((Self { track_index, channel }, &buf[Self::SIZE..]))
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

/// Encode notes using the unified delta-event format.
/// `notes` must be sorted by `start_tick`.
pub fn encode_notes_delta_gate(notes: &[Note]) -> Vec<u8> {
    encode_delta_events(notes)
}

/// Decode notes written by `encode_notes_delta_gate`.
pub fn decode_notes_delta_gate(buf: &[u8]) -> Vec<Note> {
    decode_delta_events(buf)
}

/// LEB128-style unsigned varint writer.
pub fn write_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

/// LEB128-style unsigned varint reader. Returns None on truncation.
pub fn read_varint(buf: &[u8], cursor: &mut usize) -> Option<u64> {
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

/// Zigzag-encode a signed integer into an unsigned integer for varint encoding.
fn zigzag_encode(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

/// Zigzag-decode an unsigned integer back to a signed integer.
fn zigzag_decode(v: u64) -> i64 {
    let v = v as i64;
    (v >> 1) ^ -(v & 1)
}

// ── Delta event trait ──

pub trait DeltaEvent: Sized {
    fn tick(&self) -> u32;
    fn set_tick(&mut self, tick: u32);
    fn encode_payload(&self, out: &mut Vec<u8>);
    fn decode_payload(buf: &[u8], cursor: &mut usize) -> Option<Self>;
}

pub fn encode_delta_events<T: DeltaEvent>(events: &[T]) -> Vec<u8> {
    let mut out = Vec::with_capacity(events.len() * 4 + 8);
    write_varint(&mut out, events.len() as u64);
    let mut prev_tick: u32 = 0;
    for e in events {
        let delta = e.tick().saturating_sub(prev_tick);
        write_varint(&mut out, delta as u64);
        e.encode_payload(&mut out);
        prev_tick = e.tick();
    }
    out
}

pub fn decode_delta_events<T: DeltaEvent>(buf: &[u8]) -> Vec<T> {
    let mut cursor = 0usize;
    let count = match read_varint(buf, &mut cursor) {
        Some(c) => c as usize,
        None => return Vec::new(),
    };
    let mut events = Vec::with_capacity(count);
    let mut prev_tick: u32 = 0;
    for _ in 0..count {
        let Some(delta) = read_varint(buf, &mut cursor) else { break };
        let Some(mut ev) = T::decode_payload(buf, &mut cursor) else { break };
        let tick = prev_tick.wrapping_add(delta as u32);
        ev.set_tick(tick);
        events.push(ev);
        prev_tick = tick;
    }
    events
}

impl DeltaEvent for Note {
    fn tick(&self) -> u32 { self.start_tick }
    fn set_tick(&mut self, tick: u32) {
        let duration = self.end_tick.saturating_sub(self.start_tick);
        self.start_tick = tick;
        self.end_tick = tick.saturating_add(duration);
    }
    fn encode_payload(&self, out: &mut Vec<u8>) {
        let duration = self.end_tick.saturating_sub(self.start_tick);
        write_varint(out, duration as u64);
        out.push(self.key);
        out.push(self.velocity);
    }
    fn decode_payload(buf: &[u8], cursor: &mut usize) -> Option<Self> {
        let duration = read_varint(buf, cursor)? as u32;
        if *cursor + 2 > buf.len() { return None; }
        let key = buf[*cursor];
        let velocity = buf[*cursor + 1];
        *cursor += 2;
        Some(Note { start_tick: 0, end_tick: duration, key, velocity })
    }
}

/// A control change event, stored per-CC-number.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct CcEvent {
    pub tick: u32,
    pub value: u8,
}

impl DeltaEvent for CcEvent {
    fn tick(&self) -> u32 { self.tick }
    fn set_tick(&mut self, tick: u32) { self.tick = tick; }
    fn encode_payload(&self, out: &mut Vec<u8>) { out.push(self.value); }
    fn decode_payload(buf: &[u8], cursor: &mut usize) -> Option<Self> {
        if *cursor >= buf.len() { return None; }
        let value = buf[*cursor];
        *cursor += 1;
        Some(CcEvent { tick: 0, value })
    }
}

/// A pitch bend event.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct PitchBendEvent {
    pub tick: u32,
    pub value: i16,
}

impl DeltaEvent for PitchBendEvent {
    fn tick(&self) -> u32 { self.tick }
    fn set_tick(&mut self, tick: u32) { self.tick = tick; }
    fn encode_payload(&self, out: &mut Vec<u8>) {
        write_varint(out, zigzag_encode(self.value as i64));
    }
    fn decode_payload(buf: &[u8], cursor: &mut usize) -> Option<Self> {
        let raw = read_varint(buf, cursor)?;
        let value = zigzag_decode(raw) as i16;
        Some(PitchBendEvent { tick: 0, value })
    }
}

/// A program change event, stored per-channel.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct PcEvent {
    pub tick: u32,
    pub program: u8,
}

impl DeltaEvent for PcEvent {
    fn tick(&self) -> u32 { self.tick }
    fn set_tick(&mut self, tick: u32) { self.tick = tick; }
    fn encode_payload(&self, out: &mut Vec<u8>) { out.push(self.program); }
    fn decode_payload(buf: &[u8], cursor: &mut usize) -> Option<Self> {
        if *cursor >= buf.len() { return None; }
        let program = buf[*cursor];
        *cursor += 1;
        Some(PcEvent { tick: 0, program })
    }
}

/// An RPN event, stored per-RPN-number.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct RpnEvent {
    pub tick: u32,
    pub value: u16,
}

impl DeltaEvent for RpnEvent {
    fn tick(&self) -> u32 { self.tick }
    fn set_tick(&mut self, tick: u32) { self.tick = tick; }
    fn encode_payload(&self, out: &mut Vec<u8>) {
        write_varint(out, self.value as u64);
    }
    fn decode_payload(buf: &[u8], cursor: &mut usize) -> Option<Self> {
        let value = read_varint(buf, cursor)? as u16;
        Some(RpnEvent { tick: 0, value })
    }
}

// ── Conductor event types ──

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TempoEvent {
    pub tick: u32,
    pub bpm: f32,
}

impl DeltaEvent for TempoEvent {
    fn tick(&self) -> u32 { self.tick }
    fn set_tick(&mut self, tick: u32) { self.tick = tick; }
    fn encode_payload(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.bpm.to_le_bytes());
    }
    fn decode_payload(buf: &[u8], cursor: &mut usize) -> Option<Self> {
        if *cursor + 4 > buf.len() { return None; }
        let bpm = f32::from_le_bytes([buf[*cursor], buf[*cursor+1], buf[*cursor+2], buf[*cursor+3]]);
        *cursor += 4;
        Some(TempoEvent { tick: 0, bpm })
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct TimeSigEvent {
    pub tick: u32,
    pub numerator: u8,
    /// Denominator as power of 2 (e.g. 2 means 4).
    pub denominator_power: u8,
}

impl DeltaEvent for TimeSigEvent {
    fn tick(&self) -> u32 { self.tick }
    fn set_tick(&mut self, tick: u32) { self.tick = tick; }
    fn encode_payload(&self, out: &mut Vec<u8>) {
        out.push(self.numerator);
        out.push(self.denominator_power);
    }
    fn decode_payload(buf: &[u8], cursor: &mut usize) -> Option<Self> {
        if *cursor + 2 > buf.len() { return None; }
        let numerator = buf[*cursor];
        let denominator_power = buf[*cursor + 1];
        *cursor += 2;
        Some(TimeSigEvent { tick: 0, numerator, denominator_power })
    }
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
    /// Original MIDI track index (preserved across save/load for correct name mapping).
    #[serde(default)]
    pub track_index: u8,
    /// MIDI Channel Prefix (meta event 0x20) for this track, if present.
    /// Used as fallback for channel info on tracks with no note/CC events.
    #[serde(default)]
    pub channel_prefix: Option<u8>,
}

// ── Path helpers ──
//
// Track-scoped data uses `channels/{A01}/{uuid}/...` paths, where `A01`
// derives from the global channel (port * 16 + raw_ch): port letter A..H +
// 1-indexed raw channel 01..16. The UUID alone identifies the track; port/
// channel are also carried in the InnerHeader and in mapping.json. Multiple
// tracks sharing the same (port, channel) live under distinct UUIDs and
// never collide.

/// Build the conductor entry path inside the archive.
pub fn conductor_path(name: &str) -> String {
    format!("conductor/{name}")
}

/// Format a global channel (0..127) as `A01` style label.
/// port = global_channel / 16 → letter 'A' + port (A..H)
/// raw  = global_channel % 16 → 1-indexed two-digit "01".."16"
pub fn channel_label(global_channel: u8) -> String {
    let port = global_channel / 16;
    let raw = global_channel % 16;
    format!("{}{:02}", (b'A' + port) as char, raw + 1)
}

/// Build the directory prefix for a single track.
pub fn track_prefix(global_channel: u8, uuid: &str) -> String {
    format!("channels/{}/{}", channel_label(global_channel), uuid)
}

/// Build the full path for a track notes entry.
pub fn track_notes_path(global_channel: u8, uuid: &str) -> String {
    format!("{}/notes.zst", track_prefix(global_channel, uuid))
}

/// Build the full path for a CC entry for one track.
pub fn cc_path(global_channel: u8, uuid: &str, cc_num: u8) -> String {
    format!(
        "{}/cc_{cc_num:03}.zst",
        track_prefix(global_channel, uuid)
    )
}

/// Build the full path for a pitch bend entry for one track.
pub fn pitch_path(global_channel: u8, uuid: &str) -> String {
    format!("{}/pitch.zst", track_prefix(global_channel, uuid))
}

/// Build the full path for a program change entry for one track.
pub fn pc_path(global_channel: u8, uuid: &str) -> String {
    format!("{}/pc.zst", track_prefix(global_channel, uuid))
}

/// Build the full path for an RPN entry for one track.
pub fn rpn_path(global_channel: u8, uuid: &str, rpn_num: u8) -> String {
    format!("{}/rpn_{rpn_num}.zst", track_prefix(global_channel, uuid))
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
            soundfont_project_mode: false,
            soundfont_overrides: Vec::new(),
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
        archive.set_notes(
            track_notes_path(17, "abc123"),
            FileHeader::new(magic::TRACK_NOTES, 1, 1, 0),
            InnerHeader::new(0, 17),
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
        archive.set_delta_events(
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
        let (inner, note_events) = loaded.get_notes(&track_notes_path(17, "abc123")).unwrap();
        assert_eq!(inner.track_index, 0);
        assert_eq!(inner.channel, 17);
        assert_eq!(note_events.len(), 2);
        assert_eq!(note_events[0].start_tick, 0);
        assert_eq!(note_events[1].key, 64);

        // Verify tempo
        let tempo_events: Vec<TempoEvent> =
            loaded.get_delta_events(&conductor_path("tempo.zst")).unwrap();
        assert_eq!(tempo_events.len(), 2);
        assert_eq!(tempo_events[1].bpm, 140.0);

        // Verify entry count
        assert_eq!(loaded.entries.len(), 3);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn path_helpers() {
        assert_eq!(conductor_path("tempo.zst"), "conductor/tempo.zst");
        // global_channel = 0 → port 'A', raw_ch 0 → label "A01"
        assert_eq!(track_notes_path(0, "abc"), "channels/A01/abc/notes.zst");
        // global_channel = 17 = port 1 ('B') + raw_ch 1 (label "02") → "B02"
        assert_eq!(cc_path(17, "abc", 7), "channels/B02/abc/cc_007.zst");
        assert_eq!(pitch_path(0, "abc"), "channels/A01/abc/pitch.zst");
        assert_eq!(pc_path(0, "abc"), "channels/A01/abc/pc.zst");
        assert_eq!(rpn_path(0, "abc", 0), "channels/A01/abc/rpn_0.zst");
    }

    #[test]
    fn channel_label_format() {
        assert_eq!(channel_label(0), "A01");
        assert_eq!(channel_label(15), "A16");
        assert_eq!(channel_label(16), "B01");
        assert_eq!(channel_label(17), "B02");
        assert_eq!(channel_label(127), "H16");
    }

    #[test]
    fn inner_header_roundtrip() {
        let inner = InnerHeader::new(42, 0x35); // port=3, raw_ch=5
        let mut buf = Vec::new();
        inner.write(&mut buf);
        assert_eq!(buf.len(), InnerHeader::SIZE);
        let (decoded, rest) = InnerHeader::read(&buf).unwrap();
        assert_eq!(decoded.track_index, 42);
        assert_eq!(decoded.channel, 0x35);
        assert_eq!(decoded.port(), 3);
        assert_eq!(decoded.raw_channel(), 5);
        assert!(rest.is_empty());
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
        let path = track_notes_path(0x11, "test");
        archive.set_notes(
            &path,
            FileHeader::new(magic::TRACK_NOTES, 1, 1, 0),
            InnerHeader::new(5, 0x11),
            &notes,
        );

        // header.version should be auto-set to NOTES_VERSION_DELTA_GATE
        let entry = archive.entries.get(&path).unwrap();
        assert_eq!(entry.header.version, NOTES_VERSION_DELTA_GATE);

        let (inner, decoded) = archive.get_notes(&path).unwrap();
        assert_eq!(inner.track_index, 5);
        assert_eq!(inner.channel, 0x11);
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].start_tick, 100);
        assert_eq!(decoded[0].end_tick, 200);
        assert_eq!(decoded[1].key, 64);
    }
}
