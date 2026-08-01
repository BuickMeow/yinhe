use std::path::{Path, PathBuf};

use thiserror::Error;
use unarc_rs::ArchiveError as UnarcError;
use unarc_rs::unified::{ArchiveFormat, ArchiveOptions, UnifiedArchive};

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
/// **懒加载设计**：打开时只遍历一次收集 MIDI 文件的元数据（文件名 + 大小），
/// 不解压文件内容。`read_file` 时才重新打开压缩包并解压指定文件。
///
/// 这样即使压缩包内含多个几 GB 的黑乐谱，内存占用也仅为元数据（几 KB），
/// 只有用户实际选中的那个文件才会被解压到内存。
#[derive(Clone)]
pub struct Archive {
    /// 压缩包路径，`read_file` 时重新打开。
    path: PathBuf,
    /// 密码（如有），`read_file` 时重新传入。
    password: Option<String>,
    /// 所有 MIDI 文件的元数据（打开时收集，不含文件内容）。
    midi_entries: Vec<ArchiveEntry>,
}

impl Archive {
    /// Open an archive file without a password. Format is auto-detected from
    /// the file extension.
    ///
    /// 如果压缩包包含加密条目，返回 `ArchiveError::PasswordRequired`。
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ArchiveError> {
        Self::open_with_password(path, None)
    }

    /// Open an archive file with an optional password.
    ///
    /// 只收集 MIDI 文件元数据，不解压内容。
    pub fn open_with_password(
        path: impl AsRef<Path>,
        password: Option<&str>,
    ) -> Result<Self, ArchiveError> {
        let path = path.as_ref().to_path_buf();
        tracing::info!("打开压缩包: {:?}", path);

        if ArchiveFormat::from_path(&path).is_none() {
            return Err(ArchiveError::UnsupportedFormat(format!("{:?}", path)));
        }

        let pw = password.map(|p| p.to_string()).filter(|p| !p.is_empty());
        let options = match &pw {
            Some(p) => ArchiveOptions::new().with_password(p),
            None => ArchiveOptions::new(),
        };

        let mut archive = ArchiveFormat::open_path_with_options(&path, options)
            .map_err(|e| classify_open_error(&e))?;

        let midi_entries = collect_midi_entries(&mut archive)?;

        Ok(Self {
            path,
            password: pw,
            midi_entries,
        })
    }

    /// List all MIDI files (.mid/.midi) in the archive, sorted by name A-Z.
    pub fn list_midi_files(&self) -> Vec<ArchiveEntry> {
        let mut entries = self.midi_entries.clone();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        entries
    }

    /// Read a file from the archive by name.
    ///
    /// 重新打开压缩包并顺序遍历到目标 entry，只解压该文件。
    pub fn read_file(&self, name: &str) -> Result<Vec<u8>, ArchiveError> {
        let options = match &self.password {
            Some(p) => ArchiveOptions::new().with_password(p),
            None => ArchiveOptions::new(),
        };

        let mut archive = ArchiveFormat::open_path_with_options(&self.path, options)
            .map_err(|e| classify_open_error(&e))?;

        while let Some(entry) = archive.next_entry().map_err(|e| classify_read_error(&e))? {
            if entry.name() == name {
                if entry.is_encrypted() && !archive.options().has_password() {
                    return Err(ArchiveError::PasswordRequired);
                }
                return archive.read(&entry).map_err(|e| classify_read_error(&e));
            }
            archive.skip(&entry).map_err(|e| classify_read_error(&e))?;
        }

        Err(ArchiveError::FileNotFound(name.to_string()))
    }
}

/// 遍历 archive 中的所有条目，收集 MIDI 文件元数据（不解压内容）。
///
/// 遇到加密的 MIDI 条目且未提供密码时，返回 `PasswordRequired`。
fn collect_midi_entries<R: std::io::Read + std::io::Seek>(
    archive: &mut UnifiedArchive<R>,
) -> Result<Vec<ArchiveEntry>, ArchiveError> {
    let mut entries = Vec::new();

    while let Some(entry) = archive.next_entry().map_err(|e| classify_read_error(&e))? {
        let name = entry.name().to_string();
        if is_midi_file(&name) {
            if entry.is_encrypted() && !archive.options().has_password() {
                return Err(ArchiveError::PasswordRequired);
            }
            entries.push(ArchiveEntry {
                name,
                size: entry.original_size(),
            });
        }
        // skip 所有条目（包括 MIDI），只收集元数据不解压内容。
        archive.skip(&entry).map_err(|e| classify_read_error(&e))?;
    }

    Ok(entries)
}

/// 将 unarc-rs 的打开阶段错误映射为 yinhe-archive 错误。
///
/// 用结构化 match 处理 unarc-rs 的 `ArchiveError` 变体，而不是字符串匹配。
/// 7z 等格式的加密错误在打开阶段（header 加密）就被底层库抛出，包裹在
/// `ExternalLibrary` 变体中。
fn classify_open_error(e: &UnarcError) -> ArchiveError {
    match e {
        UnarcError::PasswordRequired { .. } => ArchiveError::PasswordRequired,
        UnarcError::EncryptionRequired { .. } => ArchiveError::PasswordRequired,
        UnarcError::InvalidPassword { .. } => ArchiveError::WrongPassword,
        UnarcError::ExternalLibrary { library, message } => {
            if is_password_related_message(message) {
                // 底层库（sevenz-rust2 等）的密码错误，无法区分"需要密码"和"密码错误"
                // 先按"需要密码"处理，让用户有机会输入；若密码错误下次会返回 WrongPassword。
                ArchiveError::PasswordRequired
            } else {
                ArchiveError::Archive(format!("{} error: {}", library, message))
            }
        }
        other => ArchiveError::Archive(other.to_string()),
    }
}

/// 将 unarc-rs 的读取阶段错误映射为 yinhe-archive 错误。
///
/// 读取阶段（next_entry / read / skip）的错误通常表示密码错误。
fn classify_read_error(e: &UnarcError) -> ArchiveError {
    match e {
        UnarcError::PasswordRequired { .. } => ArchiveError::PasswordRequired,
        UnarcError::EncryptionRequired { .. } => ArchiveError::PasswordRequired,
        UnarcError::InvalidPassword { .. } => ArchiveError::WrongPassword,
        UnarcError::ExternalLibrary { library, message } => {
            if is_password_related_message(message) {
                ArchiveError::WrongPassword
            } else {
                ArchiveError::Archive(format!("{} error: {}", library, message))
            }
        }
        other => ArchiveError::Archive(other.to_string()),
    }
}

/// 判断底层库的错误消息是否与密码相关（大小写不敏感）。
fn is_password_related_message(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    lower.contains("password") || lower.contains("encrypted") || lower.contains("crc failed")
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
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

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
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        zip.start_file("readme.txt", options).unwrap();
        zip.write_all(b"hello").unwrap();
        zip.finish().unwrap();

        let archive = Archive::open(&zip_path).unwrap();
        let result = archive.read_file("nonexistent.mid");
        assert!(result.is_err(), "expected error for nonexistent file");
    }

    /// 验证 Archive 可以 Clone（懒加载架构下 Archive 很轻量）。
    #[test]
    fn test_archive_clone() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("test.zip");

        let zip_file = std::fs::File::create(&zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(zip_file);
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        zip.start_file("track.mid", options).unwrap();
        zip.write_all(b"MThd").unwrap();
        zip.finish().unwrap();

        let archive = Archive::open(&zip_path).unwrap();
        let cloned = archive.clone();
        assert_eq!(cloned.list_midi_files().len(), 1);

        // clone 后仍可读取文件
        let data = cloned.read_file("track.mid").unwrap();
        assert_eq!(data, b"MThd");
    }
}
