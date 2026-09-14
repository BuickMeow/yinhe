//! 跨实例剪贴板文件的编解码。
//!
//! 系统剪贴板只存引用文本 `yinhe-clip/1/<id>`，实际数据由复制方后台
//! 流式写入临时文件，粘贴方按引用读取。大内容不经过系统剪贴板的
//! 字符串，复制也不阻塞 UI。
//!
//! 文件布局（全小端）：
//! - header: magic `YHCL` + version u8 + kind u8
//! - notes: count u64，随后每条 12 字节
//!   `start_tick u32 | end_tick u32 | velocity u8 | key u8 | track u16`
//! - automation: clip_count u32，随后每个 clip:
//!   target（tag u8 + 参数）+ event_count u64，随后每个事件
//!   `tick u32 | value f32 | shape（tag u8 + 可选 4×f32）`

use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use yinhe_types::Note;
use yinhe_types::automation::{AutomationTarget, SegmentShape};

use crate::clipboard::{
    AutomationClip, AutomationClipboard, ClipboardContent, NotesClipboard, NotesClipboardData,
};

const MAGIC: &[u8; 4] = b"YHCL";
const VERSION: u8 = 1;
const KIND_NOTES: u8 = 0;
const KIND_AUTOMATION: u8 = 1;
const NOTE_ENTRY_SIZE: u64 = 12;

/// 系统剪贴板引用文本前缀。
pub const CLIP_REF_PREFIX: &str = "yinhe-clip/1/";

/// 生成系统剪贴板引用文本。
pub fn clip_ref(id: &str) -> String {
    format!("{CLIP_REF_PREFIX}{id}")
}

/// 解析系统剪贴板引用文本为 id。非本应用格式返回 None。
pub fn parse_clip_ref(text: &str) -> Option<&str> {
    text.trim()
        .strip_prefix(CLIP_REF_PREFIX)
        .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-'))
}

/// 剪贴板临时文件目录（系统临时目录下，系统会自行回收）。
pub fn clip_dir() -> PathBuf {
    std::env::temp_dir().join("yinhe-clipboard")
}

/// 某个 id 对应的数据文件路径。
pub fn clip_file_path(id: &str) -> PathBuf {
    clip_dir().join(format!("{id}.yhcl"))
}

/// 清理过期剪贴板临时文件。
///
/// 文件可能正被其他 yinhe 实例读取，只清超过 `max_age` 的旧文件。
pub fn cleanup_stale(max_age: Duration) {
    let Ok(entries) = fs::read_dir(clip_dir()) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("yhcl") {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else {
            continue;
        };
        let stale = now
            .duration_since(modified)
            .map(|age| age > max_age)
            .unwrap_or(false);
        if stale {
            let _ = fs::remove_file(&path);
        }
    }
}

/// 把音符剪贴板写入文件。
///
/// 快照来源走流式遍历，不在内存里物化全量数据。
pub fn write_notes(path: &Path, clipboard: &NotesClipboard) -> io::Result<()> {
    create_parent(path)?;
    let mut w = BufWriter::new(File::create(path)?);
    w.write_all(MAGIC)?;
    w.write_all(&[VERSION, KIND_NOTES])?;
    match &clipboard.data {
        NotesClipboardData::Materialized(notes) => {
            w.write_all(&(notes.len() as u64).to_le_bytes())?;
            for (note, key) in notes {
                write_note(&mut w, note, *key)?;
            }
        }
        NotesClipboardData::Snapshot {
            snapshot,
            selection,
        } => {
            let count = count_selected(snapshot, selection);
            w.write_all(&count.to_le_bytes())?;
            let mut result = Ok(());
            crate::batch_ops::for_each_selected(snapshot, selection, |note, key| {
                if result.is_ok() {
                    result = write_note(&mut w, note, key);
                }
            });
            result?;
        }
    }
    w.flush()
}

/// 把自动化剪贴板写入文件（内部先物化选择范围内的锚点）。
pub fn write_automation(path: &Path, clipboard: &AutomationClipboard) -> io::Result<()> {
    let clips = clipboard.collect();
    create_parent(path)?;
    let mut w = BufWriter::new(File::create(path)?);
    w.write_all(MAGIC)?;
    w.write_all(&[VERSION, KIND_AUTOMATION])?;
    w.write_all(&(clips.len() as u32).to_le_bytes())?;
    for clip in &clips {
        write_target(&mut w, &clip.target)?;
        w.write_all(&(clip.events.len() as u64).to_le_bytes())?;
        for (tick, value, shape) in &clip.events {
            w.write_all(&tick.to_le_bytes())?;
            w.write_all(&value.to_le_bytes())?;
            write_shape(&mut w, shape)?;
        }
    }
    w.flush()
}

/// 读取剪贴板文件。
pub fn read(path: &Path) -> io::Result<ClipboardContent> {
    let file_len = fs::metadata(path)?.len();
    let mut r = BufReader::new(File::open(path)?);

    let mut magic = [0u8; 4];
    r.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(invalid("bad magic"));
    }
    let mut version = [0u8; 1];
    r.read_exact(&mut version)?;
    if version[0] != VERSION {
        return Err(invalid("unsupported version"));
    }
    let mut kind = [0u8; 1];
    r.read_exact(&mut kind)?;
    match kind[0] {
        KIND_NOTES => {
            let count = read_u64(&mut r)?;
            if count > file_len / NOTE_ENTRY_SIZE + 1 {
                return Err(invalid("note count exceeds file size"));
            }
            let mut notes = Vec::with_capacity(count as usize);
            for _ in 0..count {
                let start_tick = read_u32(&mut r)?;
                let end_tick = read_u32(&mut r)?;
                let mut vb = [0u8; 2];
                r.read_exact(&mut vb)?;
                let track = read_u16(&mut r)?;
                notes.push((
                    Note {
                        id: 0,
                        start_tick,
                        end_tick,
                        velocity: vb[0],
                        track,
                    },
                    vb[1],
                ));
            }
            Ok(ClipboardContent::Notes(NotesClipboard::from_materialized(
                notes,
            )))
        }
        KIND_AUTOMATION => {
            let clip_count = read_u32(&mut r)?;
            if clip_count as u64 > file_len / 2 {
                return Err(invalid("clip count exceeds file size"));
            }
            let mut clips = Vec::with_capacity(clip_count as usize);
            for _ in 0..clip_count {
                let target = read_target(&mut r)?;
                let event_count = read_u64(&mut r)?;
                if event_count > file_len / 9 + 1 {
                    return Err(invalid("event count exceeds file size"));
                }
                let mut events = Vec::with_capacity(event_count as usize);
                for _ in 0..event_count {
                    let tick = read_u32(&mut r)?;
                    let value = read_f32(&mut r)?;
                    let shape = read_shape(&mut r)?;
                    events.push((tick, value, shape));
                }
                clips.push(AutomationClip { target, events });
            }
            Ok(ClipboardContent::Automation(
                AutomationClipboard::from_materialized(clips),
            ))
        }
        _ => Err(invalid("unknown clipboard kind")),
    }
}

// ── 私有辅助 ──

fn create_parent(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

fn invalid(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

fn count_selected(model: &yinhe_core::YinModel, selection: &yinhe_core::Selection) -> u64 {
    let mut count = 0u64;
    for &(tick_start, tick_end, key_lo, key_hi, track_lo, track_hi) in &selection.rects {
        for key in key_lo..=key_hi {
            count += model.notes[key as usize]
                .range(tick_start, tick_end)
                .filter(|n| n.track >= track_lo && n.track <= track_hi)
                .count() as u64;
        }
    }
    count
}

fn write_note(w: &mut impl Write, note: &Note, key: u8) -> io::Result<()> {
    w.write_all(&note.start_tick.to_le_bytes())?;
    w.write_all(&note.end_tick.to_le_bytes())?;
    w.write_all(&[note.velocity, key])?;
    w.write_all(&note.track.to_le_bytes())
}

fn write_target(w: &mut impl Write, target: &AutomationTarget) -> io::Result<()> {
    match target {
        AutomationTarget::CC { controller } => {
            w.write_all(&[0, *controller])?;
        }
        AutomationTarget::PitchBend => {
            w.write_all(&[1])?;
        }
        AutomationTarget::Rpn { parameter } => {
            w.write_all(&[2])?;
            w.write_all(&parameter.to_le_bytes())?;
        }
        AutomationTarget::Nrpn { parameter } => {
            w.write_all(&[3])?;
            w.write_all(&parameter.to_le_bytes())?;
        }
        AutomationTarget::Tempo => {
            w.write_all(&[4])?;
        }
        AutomationTarget::PluginParam {
            instrument_channel,
            param_id,
            name,
        } => {
            w.write_all(&[5])?;
            w.write_all(&instrument_channel.to_le_bytes())?;
            w.write_all(&param_id.to_le_bytes())?;
            let bytes = name.as_bytes();
            w.write_all(&(bytes.len() as u32).to_le_bytes())?;
            w.write_all(bytes)?;
        }
    }
    Ok(())
}

fn read_string(r: &mut impl Read) -> io::Result<String> {
    let len = read_u32(r)? as usize;
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    String::from_utf8(buf).map_err(|_| invalid("invalid utf-8 string"))
}

fn read_target(r: &mut impl Read) -> io::Result<AutomationTarget> {
    let mut tag = [0u8; 1];
    r.read_exact(&mut tag)?;
    Ok(match tag[0] {
        0 => {
            let mut v = [0u8; 1];
            r.read_exact(&mut v)?;
            AutomationTarget::CC { controller: v[0] }
        }
        1 => AutomationTarget::PitchBend,
        2 => AutomationTarget::Rpn {
            parameter: read_u16(r)?,
        },
        3 => AutomationTarget::Nrpn {
            parameter: read_u16(r)?,
        },
        4 => AutomationTarget::Tempo,
        5 => {
            let instrument_channel = read_u16(r)?;
            let param_id = read_u32(r)?;
            let name = read_string(r)?;
            AutomationTarget::PluginParam {
                instrument_channel,
                param_id,
                name,
            }
        }
        _ => return Err(invalid("unknown automation target")),
    })
}

fn write_shape(w: &mut impl Write, shape: &SegmentShape) -> io::Result<()> {
    match shape {
        SegmentShape::Step => w.write_all(&[0]),
        SegmentShape::Curve { x1, y1, x2, y2 } => {
            w.write_all(&[1])?;
            w.write_all(&x1.to_le_bytes())?;
            w.write_all(&y1.to_le_bytes())?;
            w.write_all(&x2.to_le_bytes())?;
            w.write_all(&y2.to_le_bytes())
        }
    }
}

fn read_shape(r: &mut impl Read) -> io::Result<SegmentShape> {
    let mut tag = [0u8; 1];
    r.read_exact(&mut tag)?;
    Ok(match tag[0] {
        0 => SegmentShape::Step,
        1 => SegmentShape::Curve {
            x1: read_f32(r)?,
            y1: read_f32(r)?,
            x2: read_f32(r)?,
            y2: read_f32(r)?,
        },
        _ => return Err(invalid("unknown segment shape")),
    })
}

fn read_u16(r: &mut impl Read) -> io::Result<u16> {
    let mut b = [0u8; 2];
    r.read_exact(&mut b)?;
    Ok(u16::from_le_bytes(b))
}

fn read_u32(r: &mut impl Read) -> io::Result<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}

fn read_u64(r: &mut impl Read) -> io::Result<u64> {
    let mut b = [0u8; 8];
    r.read_exact(&mut b)?;
    Ok(u64::from_le_bytes(b))
}

fn read_f32(r: &mut impl Read) -> io::Result<f32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(f32::from_le_bytes(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "yinhe-clip-test-{name}-{}.yhcl",
            std::process::id()
        ))
    }

    #[test]
    fn clip_ref_roundtrip() {
        let r = clip_ref("abc123");
        assert_eq!(parse_clip_ref(&r), Some("abc123"));
        assert_eq!(parse_clip_ref("hello"), None);
        assert_eq!(parse_clip_ref("yinhe-clip/1/"), None);
        assert_eq!(
            parse_clip_ref("  yinhe-clip/1/deadbeef \n"),
            Some("deadbeef")
        );
        assert_eq!(parse_clip_ref("yinhe-clip/1/../etc"), None);
    }

    #[test]
    fn notes_roundtrip_materialized() {
        let path = temp_path("notes-mat");
        let notes = vec![
            (
                Note {
                    id: 0,
                    start_tick: 100,
                    end_tick: 250,
                    velocity: 90,
                    track: 3,
                },
                60u8,
            ),
            (
                Note {
                    id: 0,
                    start_tick: 300,
                    end_tick: 400,
                    velocity: 10,
                    track: 0,
                },
                127u8,
            ),
        ];
        let clipboard = NotesClipboard::from_materialized(notes);
        write_notes(&path, &clipboard).unwrap();
        let back = read(&path).unwrap();
        let _ = fs::remove_file(&path);
        match back {
            ClipboardContent::Notes(cb) => {
                let collected = cb.collect();
                assert_eq!(collected.len(), 2);
                assert_eq!(collected[0].0.start_tick, 100);
                assert_eq!(collected[0].0.velocity, 90);
                assert_eq!(collected[0].1, 60);
                assert_eq!(collected[1].0.track, 0);
                assert_eq!(collected[1].1, 127);
            }
            _ => panic!("期望音符剪贴板"),
        }
    }

    #[test]
    fn automation_roundtrip() {
        let path = temp_path("automation");
        let clipboard = AutomationClipboard::from_materialized(vec![
            AutomationClip {
                target: AutomationTarget::Tempo,
                events: vec![
                    (0, 120.0, SegmentShape::Step),
                    (
                        480,
                        90.5,
                        SegmentShape::Curve {
                            x1: 0.25,
                            y1: -0.5,
                            x2: 0.1,
                            y2: 0.4,
                        },
                    ),
                ],
            },
            AutomationClip {
                target: AutomationTarget::CC { controller: 74 },
                events: vec![(96, 64.0, SegmentShape::Step)],
            },
            AutomationClip {
                target: AutomationTarget::Nrpn { parameter: 1234 },
                events: vec![],
            },
        ]);
        write_automation(&path, &clipboard).unwrap();
        let back = read(&path).unwrap();
        let _ = fs::remove_file(&path);
        match back {
            ClipboardContent::Automation(cb) => {
                let clips = cb.collect();
                assert_eq!(clips.len(), 3);
                assert!(matches!(clips[0].target, AutomationTarget::Tempo));
                assert_eq!(clips[0].events.len(), 2);
                assert_eq!(clips[0].events[1].0, 480);
                assert!((clips[0].events[1].1 - 90.5).abs() < f32::EPSILON);
                assert!(matches!(clips[0].events[1].2, SegmentShape::Curve { .. }));
                assert!(matches!(
                    clips[1].target,
                    AutomationTarget::CC { controller: 74 }
                ));
                assert!(matches!(
                    clips[2].target,
                    AutomationTarget::Nrpn { parameter: 1234 }
                ));
            }
            _ => panic!("期望自动化剪贴板"),
        }
    }

    #[test]
    fn rejects_bad_magic() {
        let path = temp_path("bad");
        fs::write(&path, b"NOPE").unwrap();
        assert!(read(&path).is_err());
        let _ = fs::remove_file(&path);
    }
}
