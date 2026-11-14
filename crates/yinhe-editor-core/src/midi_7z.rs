//! 导出单个 MIDI 到 7z（LZMA2），供桌面/移动共用。
//!
//! 流程：先把 MIDI 流式写到目标同目录的临时 `.mid.part`，再用
//! `sevenz-rust2` 的 LZMA2 写入器压缩成 `.7z`。MIDI 本身不再整体进内存；
//! 临时文件在成功/失败后都会删除。

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use sevenz_rust2::encoder_options::Lzma2Options;
use sevenz_rust2::{ArchiveEntry, ArchiveWriter, EncoderConfiguration};
use yinhe_core::YinModel;
use yinhe_midi::MidiExportOptions;

/// 7z 内条目名：`song.7z` → `song.mid`；已经是 `.mid.7z` 时保留原 stem。
pub fn midi_entry_name(out_path: &Path) -> String {
    let stem = out_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("export");
    if stem.to_ascii_lowercase().ends_with(".mid") || stem.to_ascii_lowercase().ends_with(".midi") {
        stem.to_string()
    } else {
        format!("{stem}.mid")
    }
}

struct TempGuard(PathBuf);

impl Drop for TempGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

struct ProgressReader<'a, R: Read> {
    inner: R,
    total: u64,
    done: u64,
    last_report: u64,
    progress: &'a mut dyn FnMut(f32),
}

impl<R: Read> Read for ProgressReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.done = self.done.saturating_add(n as u64);
        // 1MB 粒度上报，避免 4KB 读循环频繁抢进度锁。
        if self.done.saturating_sub(self.last_report) >= (1 << 20) || self.done >= self.total {
            self.last_report = self.done;
            let frac = if self.total == 0 {
                1.0
            } else {
                (self.done as f32 / self.total as f32).clamp(0.0, 1.0)
            };
            (self.progress)(0.5 + frac * 0.5);
        }
        Ok(n)
    }
}

/// 压缩 MIDI 的容器格式。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MidiArchiveFormat {
    /// 7z + LZMA2，体积最小，推荐。
    #[default]
    SevenZ,
    /// zip + Deflate，兼容性最好，但对 MIDI 压缩率差。
    Zip,
}

impl MidiArchiveFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::SevenZ => "7z",
            Self::Zip => "zip",
        }
    }
    pub fn from_extension(ext: &str) -> Self {
        if ext.eq_ignore_ascii_case("zip") {
            Self::Zip
        } else {
            Self::SevenZ
        }
    }
}

/// 7z 压缩线程数：桌面最多 8 线程，移动端最多 2（LZMA2 每线程约百 MB 级内存）。
fn compression_threads() -> u32 {
    let available = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(4);
    if cfg!(any(target_os = "android", target_env = "ohos")) {
        available.min(2)
    } else {
        available.min(8)
    }
}

fn write_7z(
    file: File,
    out_path: &Path,
    entry_name: &str,
    total: u64,
    progress: &mut dyn FnMut(f32),
) -> Result<(), String> {
    let mut writer = ArchiveWriter::create(out_path).map_err(|e| format!("创建 7z 失败: {e}"))?;
    // 不加密头：目标只是压缩分享，加密会让兼容性变差。
    writer.set_encrypt_header(false);
    // LZMA2 多线程：单线程 LZMA2 在几千 MB 的 MIDI 上要跑很久。
    // 8MB chunk = preset 6 的字典大小，保证每个 worker 都能吃满一个块；
    // 移动端限制 2 线程，避免峰值内存过高。
    writer.set_content_methods(vec![EncoderConfiguration::from(
        Lzma2Options::from_level_mt(6, compression_threads(), 8 << 20),
    )]);
    let reader = ProgressReader {
        inner: file,
        total,
        done: 0,
        last_report: 0,
        progress,
    };
    writer
        .push_archive_entry(ArchiveEntry::new_file(entry_name), Some(reader))
        .map_err(|e| format!("写入 7z 失败: {e}"))?;
    writer.finish().map_err(|e| format!("完成 7z 失败: {e}"))?;
    Ok(())
}

fn write_zip(
    file: File,
    out_path: &Path,
    entry_name: &str,
    total: u64,
    progress: &mut dyn FnMut(f32),
) -> Result<(), String> {
    let out = File::create(out_path).map_err(|e| format!("创建 zip 失败: {e}"))?;
    let mut zip = zip::ZipWriter::new(out);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    zip.start_file(entry_name, options)
        .map_err(|e| format!("写入 zip 失败: {e}"))?;
    let mut reader = ProgressReader {
        inner: file,
        total,
        done: 0,
        last_report: 0,
        progress,
    };
    std::io::copy(&mut reader, &mut zip).map_err(|e| format!("写入 zip 失败: {e}"))?;
    zip.finish().map_err(|e| format!("完成 zip 失败: {e}"))?;
    Ok(())
}

/// 导出一个 MIDI 到压缩包（7z 或 zip）。
///
/// `progress` 收到 0.0..1.0：前 0.5 是写 MIDI，后 0.5 是压缩。
pub fn export_midi_archive(
    model: &YinModel,
    out_path: &Path,
    format: MidiArchiveFormat,
    opts: &MidiExportOptions,
    mut progress: impl FnMut(f32),
) -> Result<(), String> {
    let parent = out_path.parent().unwrap_or_else(|| Path::new("."));
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = parent.join(format!(".yinhe-{nonce}.mid.part"));
    let tmp_guard = TempGuard(tmp.clone());

    progress(0.02);
    yinhe_midi::write_to_file(model, &tmp, opts).map_err(|e| format!("生成 MIDI 失败: {e}"))?;
    progress(0.5);

    let total = fs::metadata(&tmp)
        .map_err(|e| format!("读取临时 MIDI 失败: {e}"))?
        .len();
    let file = File::open(&tmp).map_err(|e| format!("打开临时 MIDI 失败: {e}"))?;
    let entry_name = midi_entry_name(out_path);

    let result = match format {
        MidiArchiveFormat::SevenZ => write_7z(file, out_path, &entry_name, total, &mut progress),
        MidiArchiveFormat::Zip => write_zip(file, out_path, &entry_name, total, &mut progress),
    };

    drop(tmp_guard);
    if result.is_err() {
        let _ = fs::remove_file(out_path);
        return result;
    }
    progress(1.0);
    Ok(())
}

/// 兼容旧调用：导出 7z。
pub fn export_midi_7z(
    model: &YinModel,
    out_path: &Path,
    opts: &MidiExportOptions,
    progress: impl FnMut(f32),
) -> Result<(), String> {
    export_midi_archive(model, out_path, MidiArchiveFormat::SevenZ, opts, progress)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use yinhe_core::{NoteEvent, TrackData, YinModel};

    fn tiny_model() -> YinModel {
        let mut m = YinModel {
            tracks: vec![Arc::new(TrackData::new(0, 0))],
            ..Default::default()
        };
        m.load_track_notes(vec![vec![NoteEvent {
            id: 0,
            start_tick: 0,
            end_tick: 480,
            key: 60,
            velocity: 100,
        }]]);
        m.rebuild();
        m
    }

    #[test]
    fn exports_single_midi_into_7z() {
        let dir = std::env::temp_dir().join(format!(
            "yinhe-7z-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&dir).expect("temp dir");
        let archive = dir.join("song.7z");
        let mut last = 0.0;
        export_midi_7z(
            &tiny_model(),
            &archive,
            &MidiExportOptions::default(),
            |p| last = p,
        )
        .expect("export");
        assert!(archive.metadata().expect("archive").len() > 0);
        assert_eq!(last, 1.0);

        let out = dir.join("out");
        sevenz_rust2::decompress_file(&archive, &out).expect("decompress");
        let mid = out.join("song.mid");
        assert!(mid.exists(), "7z 内应有 song.mid");
        let bytes = fs::read(&mid).expect("read mid");
        assert!(bytes.starts_with(b"MThd"));
        let _ = fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod zip_tests {
    use super::*;
    use std::io::Read;
    use std::sync::Arc;
    use yinhe_core::{NoteEvent, TrackData, YinModel};

    #[test]
    fn exports_single_midi_into_zip() {
        let dir = std::env::temp_dir().join(format!(
            "yinhe-zip-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&dir).unwrap();
        let archive = dir.join("song.zip");
        let mut m = YinModel {
            tracks: vec![Arc::new(TrackData::new(0, 0))],
            ..Default::default()
        };
        m.load_track_notes(vec![vec![NoteEvent {
            id: 0,
            start_tick: 0,
            end_tick: 480,
            key: 60,
            velocity: 100,
        }]]);
        m.rebuild();
        export_midi_archive(
            &m,
            &archive,
            MidiArchiveFormat::Zip,
            &MidiExportOptions::default(),
            |_| {},
        )
        .expect("export");
        let file = File::open(&archive).unwrap();
        let mut zip = zip::ZipArchive::new(file).unwrap();
        let mut entry = zip.by_name("song.mid").expect("song.mid");
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        assert!(bytes.starts_with(b"MThd"));
        let _ = fs::remove_dir_all(&dir);
    }
}
