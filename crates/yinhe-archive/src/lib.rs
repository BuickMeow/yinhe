use std::collections::HashMap;
use std::path::Path;

use thiserror::Error;
use unarc_rs::unified::{ArchiveFormat, ArchiveOptions, UnifiedArchive};
use unarc_rs::ArchiveError as UnarcError;

/// Error type for archive operations.
#[derive(Debug, Error)]
pub enum ArchiveError {
    #[error("不支持的压缩格式: {0}")]
    UnsupportedFormat(String),

    #[error("在压缩包中未找到文件: {0}")]
    FileNotFound(String),

    #[error("压缩包需要密码")]
    PasswordRequired,

    #[error("密码错误")]
    WrongPassword,

    #[error("压缩包解析错误: {0}")]
    Archive(String),

    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
}

/// Information about an entry in the archive.
#[derive(Debug, Clone)]
pub struct ArchiveEntry {
    /// File name (including path within the archive).
    pub name: String,
    /// Uncompressed size in bytes.
    pub size: u64,
}

/// Archive reader supporting multiple compression formats.
///
/// 所有支持的格式（ZIP / 7z / RAR / TAR / TGZ / TBZ / LHA/LZH / ARJ / ZOO 等）
/// 均通过 unarc-rs 统一处理：打开时一次性将 MIDI 文件解压到内存 HashMap，
/// 后续 `list_midi_files` / `read_file` 都是 O(1) HashMap 查找。
pub struct Archive {
    files: HashMap<String, Vec<u8>>,
}

impl Archive {
    /// Open an archive file without a password. Format is auto-detected from
    /// the file extension (falls back to magic-byte detection).
    ///
    /// 如果压缩包包含加密条目，返回 `ArchiveError::PasswordRequired`。
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ArchiveError> {
        Self::open_with_password(path, None)
    }

    /// Open an archive file with an optional password.
    pub fn open_with_password(
        path: impl AsRef<Path>,
        password: Option<&str>,
    ) -> Result<Self, ArchiveError> {
        let path = path.as_ref();
        tracing::info!("打开压缩包: {:?}", path);

        if ArchiveFormat::from_path(path).is_none() {
            return Err(ArchiveError::UnsupportedFormat(format!("{:?}", path)));
        }

        let options = match password {
            Some(p) if !p.is_empty() => ArchiveOptions::new().with_password(p),
            _ => ArchiveOptions::new(),
        };

        let mut archive = ArchiveFormat::open_path_with_options(path, options)
            .map_err(|e| classify_open_error(&e))?;

        let files = extract_midi_files(&mut archive)?;

        Ok(Self { files })
    }

    /// List all MIDI files (.mid/.midi) in the archive, sorted by name A-Z.
    pub fn list_midi_files(&self) -> Vec<ArchiveEntry> {
        let mut entries: Vec<ArchiveEntry> = self
            .files
            .iter()
            .map(|(name, data)| ArchiveEntry {
                name: name.clone(),
                size: data.len() as u64,
            })
            .collect();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        entries
    }

    /// Read a file from the archive by name.
    pub fn read_file(&self, name: &str) -> Result<Vec<u8>, ArchiveError> {
        self.files
            .get(name)
            .cloned()
            .ok_or_else(|| ArchiveError::FileNotFound(name.to_string()))
    }
}

/// 遍历 archive 中的所有条目，将 MIDI 文件解压到 HashMap。
fn extract_midi_files<R: std::io::Read + std::io::Seek>(
    archive: &mut UnifiedArchive<R>,
) -> Result<HashMap<String, Vec<u8>>, ArchiveError> {
    let mut files = HashMap::new();

    while let Some(entry) = archive
        .next_entry()
        .map_err(|e| classify_read_error(&e))?
    {
        let name = entry.name().to_string();
        if !is_midi_file(&name) {
            // 跳过非 MIDI 条目，但仍需调用 skip 以推进流。
            archive
                .skip(&entry)
                .map_err(|e| classify_read_error(&e))?;
            continue;
        }

        if entry.is_encrypted() && !archive.options().has_password() {
            return Err(ArchiveError::PasswordRequired);
        }

        let data = archive
            .read(&entry)
            .map_err(|e| classify_read_error(&e))?;
        files.insert(name, data);
    }

    Ok(files)
}

/// 将 unarc-rs 的打开阶段错误映射为 yinhe-archive 错误。
fn classify_open_error(e: &UnarcError) -> ArchiveError {
    let msg = e.to_string();
    if msg.contains("password") || msg.contains("encrypted") {
        ArchiveError::WrongPassword
    } else {
        ArchiveError::Archive(msg)
    }
}

/// 将 unarc-rs 的读取阶段错误映射为 yinhe-archive 错误。
fn classify_read_error(e: &UnarcError) -> ArchiveError {
    let msg = e.to_string();
    if msg.contains("password") || msg.contains("encrypted") {
        ArchiveError::WrongPassword
    } else {
        ArchiveError::Archive(msg)
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
    fn test_unsupported_format() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.unknown");
        std::fs::write(&path, b"garbage").unwrap();

        match Archive::open(&path) {
            Err(ArchiveError::UnsupportedFormat(_)) => {}
            Err(e) => panic!("expected UnsupportedFormat, got: {:?}", e),
            Ok(_) => panic!("expected error"),
        }
    }

    #[test]
    fn test_nonexistent_file() {
        let path = "/tmp/yinhe-archive-nonexistent-12345.zip";
        match Archive::open(path) {
            Err(_) => {}
            Ok(_) => panic!("expected error for nonexistent file"),
        }
    }

    #[test]
    fn test_zip_list_and_read() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("test.zip");

        let zip_file = std::fs::File::create(&zip_path).unwrap();
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

        let archive = Archive::open(&zip_path).unwrap();
        let entries = archive.list_midi_files();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "track1.mid");
        assert_eq!(entries[1].name, "track2.midi");

        let data = archive.read_file("track1.mid").unwrap();
        assert_eq!(data, b"MThd");

        let data = archive.read_file("track2.midi").unwrap();
        assert_eq!(data, b"MThd");
    }

    #[test]
    fn test_read_nonexistent() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("test.zip");

        let zip_file = std::fs::File::create(&zip_path).unwrap();
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
