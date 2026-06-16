use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Cursor, Read};
use std::path::Path;

use thiserror::Error;

/// Error type for archive operations.
#[derive(Debug, Error)]
pub enum ArchiveError {
    #[error("不支持的压缩格式: {0}")]
    UnsupportedFormat(String),

    #[error("在压缩包中未找到文件: {0}")]
    FileNotFound(String),

    #[error("IO 错误: {0}")]
    Io(#[from] io::Error),

    #[error("ZIP 解析错误: {0}")]
    Zip(String),

    #[error("7Z 错误: {0}")]
    SevenZ(String),

    #[error("TAR 错误: {0}")]
    Tar(String),
}

/// Information about an entry in the archive.
#[derive(Debug, Clone)]
pub struct ArchiveEntry {
    /// File name (including path within the archive).
    pub name: String,
    /// Uncompressed size in bytes.
    pub size: u64,
}

/// Archive format detected from file extension.
#[derive(Debug)]
enum Format {
    Zip,
    #[cfg(feature = "sevenz")]
    SevenZ,
    #[cfg(feature = "tar-gz")]
    TarGz,
    #[cfg(feature = "tar-xz")]
    TarXz,
    Tar,
}

/// Archive reader supporting multiple compression formats.
pub struct Archive {
    inner: ArchiveInner,
}

enum ArchiveInner {
    /// ZIP: lazy decompression with random access (RefCell for interior mutability).
    Zip(RefCell<zip::ZipArchive<File>>),
    /// 7z / TAR: fully decompressed into memory HashMap for O(1) random access.
    Memory(HashMap<String, Vec<u8>>),
}

impl Archive {
    /// Open an archive file. Format is auto-detected from the file extension.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ArchiveError> {
        let path = path.as_ref();
        let format = Self::detect_format(path)?;
        tracing::info!("打开压缩包: {:?} (格式: {:?})", path, format);

        match format {
            Format::Zip => Self::open_zip(path),
            #[cfg(feature = "sevenz")]
            Format::SevenZ => Self::open_sevenz(path),
            #[cfg(feature = "tar-gz")]
            Format::TarGz => Self::open_tar(path, Format::TarGz),
            #[cfg(feature = "tar-xz")]
            Format::TarXz => Self::open_tar(path, Format::TarXz),
            Format::Tar => Self::open_tar(path, Format::Tar),
        }
    }

    /// List all MIDI files (.mid/.midi) in the archive, sorted by name A-Z.
    pub fn list_midi_files(&self) -> Vec<ArchiveEntry> {
        match &self.inner {
            ArchiveInner::Zip(zip) => self.list_midi_files_zip(&mut zip.borrow_mut()),
            ArchiveInner::Memory(map) => {
                let mut entries: Vec<ArchiveEntry> = map
                    .keys()
                    .filter(|name| is_midi_file(name))
                    .map(|name| ArchiveEntry {
                        name: name.clone(),
                        size: map[name].len() as u64,
                    })
                    .collect();
                entries.sort_by(|a, b| a.name.cmp(&b.name));
                entries
            }
        }
    }

    /// Read a file from the archive by name.
    pub fn read_file(&self, name: &str) -> Result<Vec<u8>, ArchiveError> {
        match &self.inner {
            ArchiveInner::Zip(zip) => self.read_file_zip(&mut zip.borrow_mut(), name),
            ArchiveInner::Memory(map) => map
                .get(name)
                .cloned()
                .ok_or_else(|| ArchiveError::FileNotFound(name.to_string())),
        }
    }

    // ── ZIP implementation ──

    fn open_zip(path: &Path) -> Result<Self, ArchiveError> {
        let file = File::open(path).map_err(ArchiveError::Io)?;
        let archive = zip::ZipArchive::new(file).map_err(|e| ArchiveError::Zip(e.to_string()))?;
        Ok(Self {
            inner: ArchiveInner::Zip(RefCell::new(archive)),
        })
    }

    fn list_midi_files_zip(&self, zip: &mut zip::ZipArchive<File>) -> Vec<ArchiveEntry> {
        let mut entries: Vec<ArchiveEntry> = Vec::new();
        for i in 0..zip.len() {
            if let Ok(file) = zip.by_index(i) {
                let name = file.name().to_string();
                if is_midi_file(&name) {
                    entries.push(ArchiveEntry {
                        name,
                        size: file.size(),
                    });
                }
            }
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        entries
    }

    fn read_file_zip(&self, zip: &mut zip::ZipArchive<File>, name: &str) -> Result<Vec<u8>, ArchiveError> {
        let mut file = zip.by_name(name).map_err(|e| ArchiveError::Zip(e.to_string()))?;
        let mut buf = Vec::with_capacity(file.size() as usize);
        file.read_to_end(&mut buf).map_err(ArchiveError::Io)?;
        Ok(buf)
    }

    // ── 7Z implementation ──

    #[cfg(feature = "sevenz")]
    fn open_sevenz(path: &Path) -> Result<Self, ArchiveError> {
        let mut reader = sevenz_rust::SevenZReader::open(path, sevenz_rust::Password::empty())
            .map_err(|e| ArchiveError::SevenZ(e.to_string()))?;

        let mut map = HashMap::new();
        reader
            .for_each_entries(|entry, reader| {
                let name = entry.name().to_string();
                if is_midi_file(&name) {
                    let mut buf = Vec::with_capacity(entry.size() as usize);
                    reader.read_to_end(&mut buf)
                        .map_err(sevenz_rust::Error::io)?;
                    map.insert(name, buf);
                }
                Ok(true)
            })
            .map_err(|e| ArchiveError::SevenZ(e.to_string()))?;

        Ok(Self {
            inner: ArchiveInner::Memory(map),
        })
    }

    // ── TAR implementation ──

    #[cfg(any(feature = "tar-gz", feature = "tar-xz"))]
    fn open_tar(path: &Path, format: Format) -> Result<Self, ArchiveError> {
        let mut file = File::open(path).map_err(ArchiveError::Io)?;

        // First, decompress the entire file into memory
        let mut raw = Vec::new();
        match format {
            #[cfg(feature = "tar-gz")]
            Format::TarGz => {
                use flate2::read::GzDecoder;
                let mut decoder = GzDecoder::new(&mut file);
                decoder.read_to_end(&mut raw).map_err(ArchiveError::Io)?;
            }
            #[cfg(feature = "tar-xz")]
            Format::TarXz => {
                let mut compressed = Vec::new();
                file.read_to_end(&mut compressed).map_err(ArchiveError::Io)?;
                lzma_rs::xz_decompress(&mut compressed.as_slice(), &mut raw)
                    .map_err(|e| ArchiveError::Tar(e.to_string()))?;
            }
            Format::Tar => {
                file.read_to_end(&mut raw).map_err(ArchiveError::Io)?;
            }
            _ => unreachable!(),
        }

        // Parse tar structure and extract MIDI files
        let mut archive = tar::Archive::new(Cursor::new(&raw));
        let mut map = HashMap::new();

        for entry in archive.entries().map_err(|e| ArchiveError::Tar(e.to_string()))? {
            let mut entry = entry.map_err(ArchiveError::Io)?;
            let name = entry.path().map_err(ArchiveError::Io)?.to_string_lossy().to_string();

            if is_midi_file(&name) {
                let mut buf = Vec::with_capacity(entry.size() as usize);
                entry.read_to_end(&mut buf).map_err(ArchiveError::Io)?;
                map.insert(name, buf);
            }
        }

        Ok(Self {
            inner: ArchiveInner::Memory(map),
        })
    }

    // ── Format detection ──

    fn detect_format(path: &Path) -> Result<Format, ArchiveError> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();

        let stem_ext = path
            .file_name()
            .and_then(|f| f.to_str())
            .and_then(|f| {
                let parts: Vec<&str> = f.rsplit('.').collect();
                if parts.len() >= 2 {
                    Some(parts[1].to_lowercase())
                } else {
                    None
                }
            });

        match ext.as_str() {
            "zip" => Ok(Format::Zip),
            #[cfg(feature = "sevenz")]
            "7z" => Ok(Format::SevenZ),
            "gz" => {
                #[cfg(feature = "tar-gz")]
                {
                    if stem_ext.as_deref() == Some("tar") {
                        return Ok(Format::TarGz);
                    }
                }
                Err(ArchiveError::UnsupportedFormat(format!(
                    "不支持的 .gz 文件: {:?}",
                    path
                )))
            }
            "xz" => {
                #[cfg(feature = "tar-xz")]
                {
                    if stem_ext.as_deref() == Some("tar") {
                        return Ok(Format::TarXz);
                    }
                }
                Err(ArchiveError::UnsupportedFormat(format!(
                    "不支持的 .xz 文件: {:?}",
                    path
                )))
            }
            "tgz" => {
                #[cfg(feature = "tar-gz")]
                return Ok(Format::TarGz);
                #[cfg(not(feature = "tar-gz"))]
                Err(ArchiveError::UnsupportedFormat(
                    "需要启用 tar-gz feature".to_string(),
                ))
            }
            "txz" => {
                #[cfg(feature = "tar-xz")]
                return Ok(Format::TarXz);
                #[cfg(not(feature = "tar-xz"))]
                Err(ArchiveError::UnsupportedFormat(
                    "需要启用 tar-xz feature".to_string(),
                ))
            }
            "tar" => Ok(Format::Tar),
            _ => Err(ArchiveError::UnsupportedFormat(format!(
                "不支持的文件格式: {:?}",
                path
            ))),
        }
    }
}

/// Check if a filename is a MIDI file (case-insensitive).
fn is_midi_file(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(".mid") || lower.ends_with(".midi")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    #[test]
    fn test_zip_list_and_read() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("test.zip");

        // Create a ZIP with 2 MIDI files and 1 txt file
        let zip_file = File::create(&zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(zip_file);
        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);

        zip.start_file("track1.mid", options).unwrap();
        zip.write_all(b"MThd").unwrap();

        zip.start_file("track2.midi", options).unwrap();
        zip.write_all(b"MThd").unwrap();

        zip.start_file("readme.txt", options).unwrap();
        zip.write_all(b"not a midi").unwrap();

        zip.finish().unwrap();

        // Test list_midi_files
        let archive = Archive::open(&zip_path).unwrap();
        let entries = archive.list_midi_files();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "track1.mid");
        assert_eq!(entries[1].name, "track2.midi");

        // Test read_file
        let data = archive.read_file("track1.mid").unwrap();
        assert_eq!(data, b"MThd");

        let data = archive.read_file("track2.midi").unwrap();
        assert_eq!(data, b"MThd");
    }

    #[test]
    fn test_unsupported_format() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.unknown");
        File::create(&path).unwrap();

        match Archive::open(&path) {
            Err(ArchiveError::UnsupportedFormat(msg)) => {
                assert!(msg.contains("不支持的文件格式"));
            }
            _ => panic!("expected UnsupportedFormat error"),
        }
    }

    #[test]
    fn test_read_nonexistent() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("test.zip");

        // Create ZIP with one non-MIDI file
        let zip_file = File::create(&zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(zip_file);
        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("readme.txt", options).unwrap();
        zip.write_all(b"hello").unwrap();
        zip.finish().unwrap();

        let archive = Archive::open(&zip_path).unwrap();
        let result = archive.read_file("nonexistent.mid");
        assert!(result.is_err(), "expected error for nonexistent file");
    }
}
