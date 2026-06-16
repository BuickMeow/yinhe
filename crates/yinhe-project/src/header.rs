use serde::{Deserialize, Serialize};

// ── Magic bytes ──

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

// ── File header ──

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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn file_header_new() {
        let h = FileHeader::new(*b"YHPR", 7, 3, 42);
        assert_eq!(h.magic, *b"YHPR");
        assert_eq!(h.version, 1);
        assert_eq!(h.port, 7);
        assert_eq!(h.channel, 3);
        assert_eq!(h.extra, 42);
    }
}
