//! Domino（TAKABO SOFT）剪贴板 `MidiPortalSequence` 解码。
//!
//! Domino 在 Windows 剪贴板注册私有格式 `MidiPortalSequence`，载荷：
//! `"PortalSequenceData"`（18 字节）+ u32 LE 未压缩长度 + zlib 压缩体。
//! 压缩体是递归 item 流：`[tag:u16 LE][len:u32 LE][payload]`。
//!
//! 顶层 `1002` 是 PPQ（u16）；`1003` 是轨道块。轨道块内的 `1003` 子项
//! 为 1 表示 Conductor 轨（跳过）。音符是 `2001` item，内部含：
//! `1001`（起始 tick u32）、`2001`（key u8）、`2002`（velocity u8）、
//! `2003`（gate u32）。Domino 的 256 键 mod 会把 key 写到 128..255，
//! 这里不裁剪。tick 是相对复制起点的值；格式本身带 PPQ，导入时按
//! 工程 PPQ 重采样。

use std::io::Read;

use flate2::read::ZlibDecoder;

use crate::clipboard::NotesClipboard;
use yinhe_types::Note;

/// 剪贴板载荷固定头。
pub const PORTAL_MAGIC: &[u8; 18] = b"PortalSequenceData";
/// 解压后硬上限 = 格式 u32 长度字段的上限。旧版 1GiB 是防御性自限，
/// 会把几百 MB 以上（约 2600 万音符）的合法剪辑也拒掉。
const MAX_DECOMPRESSED_BYTES: usize = u32::MAX as usize;
/// Domino 未写 PPQ item 时的默认值。
pub const DOMINO_PPQ: u32 = 480;
const ITEM_HDR: usize = 6;

/// Domino 剪贴板里的一个音符（相对复制起点，source PPQ 见 [`DominoClipboard::ppq`]）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DominoNote {
    pub start_tick: u32,
    pub gate_ticks: u32,
    pub key: u8,
    pub velocity: u8,
}

/// 一份 Domino 剪贴板：PPQ + 每轨音符（Conductor 轨不计数）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DominoClipboard {
    pub ppq: u32,
    pub tracks: Vec<Vec<DominoNote>>,
}

/// 按 `[tag u16][len u32][payload]` 遍历 item 流；要求完整消费。
fn walk_items(
    buf: &[u8],
    mut f: impl FnMut(u16, &[u8]) -> Result<(), String>,
) -> Result<(), String> {
    let mut pos = 0usize;
    while pos + ITEM_HDR <= buf.len() {
        let tag = u16::from_le_bytes([buf[pos], buf[pos + 1]]);
        let len =
            u32::from_le_bytes([buf[pos + 2], buf[pos + 3], buf[pos + 4], buf[pos + 5]]) as usize;
        let start = pos
            .checked_add(ITEM_HDR)
            .ok_or_else(|| "item 头溢出".to_string())?;
        let end = start
            .checked_add(len)
            .ok_or_else(|| "item 长度溢出".to_string())?;
        if end > buf.len() {
            return Err(format!("item 越界 tag={tag} len={len} pos={pos}"));
        }
        f(tag, &buf[start..end])?;
        pos = end;
    }
    if pos != buf.len() {
        return Err(format!("item 流未完整消费，剩余 {} 字节", buf.len() - pos));
    }
    Ok(())
}

fn read_u32(b: &[u8]) -> Result<u32, String> {
    if b.len() < 4 {
        return Err(format!("u32 越界：长度 {}", b.len()));
    }
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn zlib_decompress(comp: &[u8], expected: usize) -> Result<Vec<u8>, String> {
    let expected = expected.min(MAX_DECOMPRESSED_BYTES);
    let mut out = Vec::new();
    if expected > 0 {
        out.try_reserve_exact(expected)
            .map_err(|_| format!("Domino 剪贴板解压需要 {expected} 字节，内存不足"))?;
    }
    let dec = ZlibDecoder::new(comp);
    let mut limited = dec.take(MAX_DECOMPRESSED_BYTES as u64 + 1);
    limited
        .read_to_end(&mut out)
        .map_err(|e| format!("zlib 解压失败：{e}"))?;
    if out.len() > MAX_DECOMPRESSED_BYTES {
        return Err("zlib 解压输出超过 u32 长度上限".into());
    }
    Ok(out)
}

fn parse_note(payload: &[u8]) -> Result<Option<DominoNote>, String> {
    let mut start = 0u32;
    let mut key = None;
    let mut velocity = 100u8;
    let mut gate = 0u32;
    walk_items(payload, |tag, body| {
        match tag {
            1001 => start = read_u32(body)?,
            2001 if body.len() == 1 => key = Some(body[0]),
            2002 if body.len() == 1 => velocity = body[0].min(127),
            2003 => gate = read_u32(body)?,
            _ => {}
        }
        Ok(())
    })?;
    Ok(key.map(|key| DominoNote {
        start_tick: start,
        gate_ticks: gate,
        key,
        velocity,
    }))
}

fn parse_track(payload: &[u8]) -> Result<(bool, Vec<DominoNote>), String> {
    let mut conductor = false;
    let mut notes = Vec::new();
    walk_items(payload, |tag, body| {
        match tag {
            // 轨道块内的 1003 子项：1 = Domino 的 Conductor 轨。
            1003 if body.len() == 1 && body[0] == 1 => conductor = true,
            2001 => {
                if let Some(n) = parse_note(body)? {
                    notes.push(n);
                }
            }
            _ => {}
        }
        Ok(())
    })?;
    Ok((conductor, notes))
}

/// 解码 Domino `MidiPortalSequence` 原始载荷。
pub fn decode_domino_clipboard(raw: &[u8]) -> Result<DominoClipboard, String> {
    if raw.len() < 22 || &raw[..18] != PORTAL_MAGIC {
        return Err("不是 Domino PortalSequenceData 剪贴板数据".into());
    }
    let size = read_u32(&raw[18..22])? as usize;
    let body = zlib_decompress(&raw[22..], size)?;

    let mut ppq = DOMINO_PPQ;
    let mut tracks: Vec<Vec<DominoNote>> = Vec::new();
    walk_items(&body, |tag, payload| {
        match tag {
            1002 if payload.len() == 2 => {
                ppq = u16::from_le_bytes([payload[0], payload[1]]) as u32;
            }
            1003 => {
                let (conductor, notes) = parse_track(payload)?;
                if !conductor {
                    tracks.push(notes);
                }
            }
            _ => {}
        }
        Ok(())
    })?;
    if tracks.iter().all(|t| t.is_empty()) {
        return Err("Domino 数据中未找到可解析的音符记录".into());
    }
    Ok(DominoClipboard {
        ppq: ppq.max(1),
        tracks,
    })
}

/// 把 Domino 剪贴板转成 yinhe 剪贴板。
///
/// - tick/gate 从源 PPQ 重采样到 `target_ppq`。
/// - 第 i 条 Domino 轨道落到 `base_track + i`；超出 `track_count` 的轨道丢弃
///   （与 Domino 粘贴到末尾轨道时丢弃后续轨道一致）。
/// - key 保留 0..=255，支持 Domino 256 键 mod。
pub fn to_notes_clipboard(
    clip: &DominoClipboard,
    target_ppq: u32,
    base_track: u16,
    track_count: usize,
) -> NotesClipboard {
    let ratio = target_ppq.max(1) as f64 / clip.ppq.max(1) as f64;
    let mut materialized = Vec::new();
    for (i, notes) in clip.tracks.iter().enumerate() {
        let Some(track) = (base_track as usize)
            .checked_add(i)
            .filter(|&t| t < track_count)
        else {
            continue;
        };
        for n in notes {
            let start = (n.start_tick as f64 * ratio).round() as u32;
            let gate = (n.gate_ticks as f64 * ratio).round() as u32;
            materialized.push((
                Note {
                    id: 0,
                    start_tick: start,
                    end_tick: start.saturating_add(gate),
                    velocity: n.velocity,
                    track: track as u16,
                },
                n.key,
            ));
        }
    }
    NotesClipboard::from_materialized(materialized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::ZlibEncoder;
    use std::io::Write;

    fn item(tag: u16, payload: &[u8]) -> Vec<u8> {
        let mut v = Vec::with_capacity(ITEM_HDR + payload.len());
        v.extend_from_slice(&tag.to_le_bytes());
        v.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        v.extend_from_slice(payload);
        v
    }

    fn note_item(start: u32, gate: u32, key: u8, vel: u8) -> Vec<u8> {
        let mut f = Vec::new();
        f.extend(item(1001, &start.to_le_bytes()));
        f.extend(item(2001, &[key]));
        f.extend(item(2002, &[vel]));
        f.extend(item(2003, &gate.to_le_bytes()));
        item(2001, &f)
    }

    fn payload(ppq: u16, tracks: &[&[DominoNote]], conductor: bool) -> Vec<u8> {
        let mut body = item(1002, &ppq.to_le_bytes());
        if conductor {
            let mut t = item(1003, &[1]);
            t.extend(item(1000, &0u16.to_le_bytes()));
            body.extend(item(1003, &t));
        }
        for notes in tracks {
            let mut inner = Vec::new();
            for n in notes.iter() {
                inner.extend(note_item(n.start_tick, n.gate_ticks, n.key, n.velocity));
            }
            let mut t = Vec::new();
            t.extend(item(1000, &0u16.to_le_bytes()));
            t.extend(item(1001, &[0]));
            t.extend(inner);
            body.extend(item(1003, &t));
        }
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::fast());
        enc.write_all(&body).unwrap();
        let comp = enc.finish().unwrap();
        let mut raw = Vec::new();
        raw.extend_from_slice(PORTAL_MAGIC);
        raw.extend_from_slice(&(body.len() as u32).to_le_bytes());
        raw.extend_from_slice(&comp);
        raw
    }

    fn note(start: u32, gate: u32, key: u8, vel: u8) -> DominoNote {
        DominoNote {
            start_tick: start,
            gate_ticks: gate,
            key,
            velocity: vel,
        }
    }

    #[test]
    fn decodes_ppq_tracks_and_256_keys() {
        let a = [note(0, 480, 60, 100), note(480, 240, 200, 90)];
        let b = [note(240, 120, 127, 80)];
        let raw = payload(960, &[&a, &b], true);
        let clip = decode_domino_clipboard(&raw).expect("decode");
        assert_eq!(clip.ppq, 960);
        assert_eq!(clip.tracks.len(), 2, "Conductor 轨应跳过");
        assert_eq!(clip.tracks[0], a);
        assert_eq!(clip.tracks[1], b);

        // 960 → 1920 PPQ，第 i 条 Domino 轨落到 base+i。
        let cb = to_notes_clipboard(&clip, 1920, 2, 4);
        let out = cb.collect();
        assert_eq!(out.len(), 3);
        assert_eq!((out[0].0.start_tick, out[0].0.end_tick), (0, 960));
        assert_eq!((out[1].0.start_tick, out[1].0.end_tick), (960, 1440));
        assert_eq!(out[2].0.start_tick, 480);
        assert_eq!(out[0].1, 60);
        assert_eq!(out[1].1, 200, "256 键 mod 的 key 不能被裁到 127");
        assert_eq!(out[2].1, 127);
        assert_eq!(
            out.iter().map(|(n, _)| n.track).collect::<Vec<_>>(),
            vec![2, 2, 3]
        );

        // 目标工程只有 3 轨时，第 2 条 Domino 轨（base+1=3）丢弃。
        let cb = to_notes_clipboard(&clip, 1920, 2, 3);
        assert_eq!(cb.collect().len(), 2);
    }

    #[test]
    fn rejects_non_domino_and_corrupt_items() {
        assert!(decode_domino_clipboard(b"not domino").is_err());

        let mut raw = Vec::new();
        raw.extend_from_slice(PORTAL_MAGIC);
        raw.extend_from_slice(&u32::MAX.to_le_bytes());
        raw.extend_from_slice(&[0u8; 8]);
        assert!(decode_domino_clipboard(&raw).is_err());

        assert!(walk_items(&[0xEB, 0x03, 0xFF, 0xFF, 0xFF, 0xFF], |_, _| Ok(())).is_err());
    }
}
