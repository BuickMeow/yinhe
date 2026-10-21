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
/// **懒加载设计**：打开时只遍历一次收集所有文件的元数据（文件名 + 大小），
/// 不解压文件内容。`read_file` / `extract_all` 时才重新打开压缩包并解压。
///
/// MIDI 选择、音色库导入等共用这一层；调用方按扩展名过滤条目。
#[derive(Clone)]
pub struct Archive {
    /// 压缩包路径，`read_file` 时重新打开。
    path: PathBuf,
    /// 密码（如有），`read_file` 时重新传入。
    password: Option<String>,
    /// 所有文件的元数据（打开时收集，不含文件内容）。
    entries: Vec<ArchiveEntry>,
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

        let entries = collect_entries(&mut archive)?;

        Ok(Self {
            path,
            password: pw,
            entries,
        })
    }

    /// 压缩包文件的原始路径（用于「最近文件」记录外层路径）。
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// List all MIDI files (.mid/.midi) in the archive, sorted by name A-Z.
    pub fn list_midi_files(&self) -> Vec<ArchiveEntry> {
        self.list_files(&["mid", "midi"])
    }

    /// List files with the given extensions (case-insensitive), sorted by name.
    ///
    /// `exts` 不带点，例如 `&["sf2", "sf3", "sfz"]`。
    pub fn list_files(&self, exts: &[&str]) -> Vec<ArchiveEntry> {
        let mut entries: Vec<ArchiveEntry> = self
            .entries
            .iter()
            .filter(|e| {
                let lower = e.name.to_ascii_lowercase();
                exts.iter().any(|ext| {
                    let ext = ext.trim_start_matches('.').to_ascii_lowercase();
                    lower.ends_with(&format!(".{ext}"))
                })
            })
            .cloned()
            .collect();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        entries
    }

    /// List every file in the archive, sorted by name.
    pub fn list_all(&self) -> Vec<ArchiveEntry> {
        let mut entries = self.entries.clone();
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

    /// 解压整个压缩包到 `dest`，保留条目内的相对路径。
    ///
    /// `progress(done_bytes, total_bytes)` 用于 UI 进度；目录条目会创建空目录。
    /// 返回实际写出的文件数。拒绝 `..` / 绝对路径等越界条目。
    pub fn extract_all<F>(&self, dest: &Path, mut progress: F) -> Result<usize, ArchiveError>
    where
        F: FnMut(u64, u64),
    {
        let options = self.options();
        let mut archive = ArchiveFormat::open_path_with_options(&self.path, options)
            .map_err(|e| classify_open_error(&e))?;
        let total: u64 = self.entries.iter().map(|e| e.size).sum();
        let mut done = 0u64;
        std::fs::create_dir_all(dest)?;
        let mut files = 0usize;
        while let Some(entry) = archive.next_entry().map_err(|e| classify_read_error(&e))? {
            let name = entry.name().to_string();
            let is_dir = name.ends_with('/') || name.ends_with('\\');
            let Some(rel) = safe_relative_path(&name) else {
                return Err(ArchiveError::Archive(format!("压缩包内路径越界: {name}")));
            };
            let out = dest.join(rel);
            if is_dir {
                std::fs::create_dir_all(&out)?;
                archive.skip(&entry).map_err(|e| classify_read_error(&e))?;
                continue;
            }
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let data = archive.read(&entry).map_err(|e| classify_read_error(&e))?;
            std::fs::write(&out, &data)?;
            files += 1;
            done = done.saturating_add(data.len() as u64);
            progress(done, total.max(done));
        }
        Ok(files)
    }

    /// 解压单个条目到 `dest`（保留条目内的相对路径），返回写出的绝对路径。
    pub fn extract_entry(&self, name: &str, dest: &Path) -> Result<PathBuf, ArchiveError> {
        let Some(rel) = safe_relative_path(name) else {
            return Err(ArchiveError::Archive(format!("压缩包内路径越界: {name}")));
        };
        let out = dest.join(rel);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let data = self.read_file(name)?;
        std::fs::write(&out, data)?;
        Ok(out)
    }

    fn options(&self) -> ArchiveOptions {
        match &self.password {
            Some(p) => ArchiveOptions::new().with_password(p),
            None => ArchiveOptions::new(),
        }
    }
}

/// 压缩包内相对路径安全校验：拒绝绝对路径、`..`、盘符。
fn safe_relative_path(name: &str) -> Option<PathBuf> {
    let normalized = name.replace('\\', "/");
    let mut out = PathBuf::new();
    for part in normalized.trim_start_matches('/').split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." || part.contains(':') {
            return None;
        }
        out.push(part);
    }
    if out.as_os_str().is_empty() {
        None
    } else {
        Some(out)
    }
}

/// 遍历 archive 中的所有条目，收集元数据（不解压内容）。
///
/// 遇到加密条目且未提供密码时，返回 `PasswordRequired`。
fn collect_entries<R: std::io::Read + std::io::Seek>(
    archive: &mut UnifiedArchive<R>,
) -> Result<Vec<ArchiveEntry>, ArchiveError> {
    let mut entries = Vec::new();

    while let Some(entry) = archive.next_entry().map_err(|e| classify_read_error(&e))? {
        if entry.is_encrypted() && !archive.options().has_password() {
            return Err(ArchiveError::PasswordRequired);
        }
        entries.push(ArchiveEntry {
            name: entry.name().to_string(),
            size: entry.original_size(),
        });
        // skip 所有条目，只收集元数据不解压内容。
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

    #[test]
    fn test_zip_list_files_and_extract_all() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("fonts.zip");
        let zip_file = std::fs::File::create(&zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(zip_file);
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        zip.start_file("Piano/Instrument.sfz", options).unwrap();
        zip.write_all(b"<region> sample=Samples/a.wav").unwrap();
        zip.start_file("Piano/Samples/a.wav", options).unwrap();
        zip.write_all(b"RIFF").unwrap();
        zip.start_file("track.mid", options).unwrap();
        zip.write_all(b"MThd").unwrap();
        zip.start_file("readme.txt", options).unwrap();
        zip.write_all(b"text").unwrap();
        zip.finish().unwrap();

        let archive = Archive::open(&zip_path).unwrap();
        assert_eq!(archive.list_all().len(), 4);
        assert_eq!(archive.list_midi_files().len(), 1);
        let sf = archive.list_files(&["sfz", "wav"]);
        assert_eq!(sf.len(), 2);
        assert_eq!(sf[0].name, "Piano/Instrument.sfz");
        assert_eq!(sf[1].name, "Piano/Samples/a.wav");

        let out = tempfile::tempdir().unwrap();
        let mut last = (0u64, 0u64);
        let files = archive
            .extract_all(out.path(), |done, total| last = (done, total))
            .unwrap();
        assert_eq!(files, 4);
        assert_eq!(last.0, last.1);
        assert!(last.0 > 0);
        assert_eq!(
            std::fs::read(out.path().join("Piano/Instrument.sfz")).unwrap(),
            b"<region> sample=Samples/a.wav"
        );
        assert_eq!(
            std::fs::read(out.path().join("Piano/Samples/a.wav")).unwrap(),
            b"RIFF"
        );
    }

    #[test]
    fn test_safe_relative_path() {
        assert!(safe_relative_path("../evil.wav").is_none());
        assert!(safe_relative_path("a/../../evil.wav").is_none());
        assert!(safe_relative_path("C:/evil.wav").is_none());
        assert_eq!(
            safe_relative_path("./Samples/a.wav"),
            Some(PathBuf::from("Samples/a.wav"))
        );
        assert_eq!(
            safe_relative_path("Piano\\Samples\\a.wav"),
            Some(PathBuf::from("Piano/Samples/a.wav"))
        );
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
