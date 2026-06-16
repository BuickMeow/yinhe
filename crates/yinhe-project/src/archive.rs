use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;

use crate::events::{decode_delta_events, decode_notes_delta_gate, encode_delta_events, encode_notes_delta_gate, DeltaEvent, Note, NOTES_VERSION_DELTA_GATE};
use crate::header::{FileHeader, InnerHeader};

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
        let _ = std::marker::PhantomData::<T>;
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

    /// Store a single value as JSON text (human-readable).
    pub fn set_json<T: Serialize>(
        &mut self,
        path: impl Into<String>,
        header: FileHeader,
        value: &T,
    ) {
        let data = serde_json::to_vec(value).expect("json serialization failed");
        self.entries.insert(
            path.into(),
            ArchiveEntry {
                path: String::new(),
                header,
                data,
            },
        );
    }

    /// Read a single value stored by `set_json`.
    pub fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Option<T> {
        let entry = self.entries.get(path)?;
        serde_json::from_slice(&entry.data).ok()
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
        let body = encode_notes_delta_gate(notes);
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
        let notes = decode_notes_delta_gate(rest);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::*;
    use crate::header::*;
    use crate::paths::*;
    use crate::schema::*;

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
        archive.set_json(
            "project.json",
            FileHeader::new(*b"YHPR", 0, 0, 0),
            &proj,
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
        let proj: ProjectJson = loaded.get_json("project.json").unwrap();
        assert_eq!(proj.name, "Test Song");

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

    #[test]
    fn set_get_delta_events_with_inner() {
        let mut archive = ProjectArchive::new();
        let events = vec![
            CcEvent { tick: 0, value: 100 },
        ];
        let path = cc_path(0, "test-uuid", 7);
        archive.set_delta_events_with_inner(
            &path,
            FileHeader::new(magic::CC, 1, 0, 0),
            InnerHeader::new(0, 0),
            &events,
        );
        let (inner, decoded): (InnerHeader, Vec<CcEvent>) =
            archive.get_delta_events_with_inner(&path).unwrap();
        assert_eq!(inner.track_index, 0);
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].value, 100);
    }

    #[test]
    fn archive_get_remove() {
        let mut archive = ProjectArchive::new();
        let path = "test/entry.zst".to_string();
        let events: Vec<TempoEvent> = vec![TempoEvent { tick: 0, bpm: 120.0 }];
        archive.set_delta_events(&path, FileHeader::new(*b"TEST", 0, 0, 0), &events);

        assert!(archive.get::<Vec<TempoEvent>>(&path).is_some());
        archive.remove(&path);
        assert!(archive.get::<Vec<TempoEvent>>(&path).is_none());
    }
}
