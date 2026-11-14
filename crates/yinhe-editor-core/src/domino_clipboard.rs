//! Domino（TAKABO SOFT）剪贴板 `MidiPortalSequence` 解码。
//!
//! Domino 在 Windows 剪贴板注册私有格式 `MidiPortalSequence`，载荷：
//! `"PortalSequenceData"`（18 字节）+ u32 LE 未压缩长度 + zlib 压缩体。
//! 压缩体是递归分块流：`[id:u8][kind:u8][len:u32 LE][payload]`。
//!
//! 音符在顶层 `eb`(0xeb, kind 0x03) 块的 `d1`(0xd1, kind 0x07) 子记录里，
//! 每条记录内是 `e9/03`（起始 tick u32）、`d1/07`（key u8）、`d2/07`
//! （velocity u8）、`d3/07`（gate u32）。Domino 的时间基准固定为 480 PPQ。
//! 格式本身不含逐音符通道，导入时使用目标轨道的通道。

use std::io::Read;

use flate2::read::ZlibDecoder;

use crate::clipboard::NotesClipboard;
use yinhe_types::Note;

/// 剪贴板载荷固定头。
pub const PORTAL_MAGIC: &[u8; 18] = b"PortalSequenceData";
/// 解压后硬上限：损坏的 size 字段不得触发巨量分配/无限解压。
const MAX_DECOMPRESSED_BYTES: usize = 1 << 30;
/// Domino 剪贴板的固定 PPQ。
pub const DOMINO_PPQ: u32 = 480;
const CHUNK_HDR: usize = 6;

/// Domino 剪贴板里的一个音符（绝对 tick，480 PPQ）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DominoNote {
    pub start_tick: u32,
    pub gate_ticks: u32,
    pub key: u8,
    pub velocity: u8,
}

struct Chunk<'a> {
    id: u8,
    kind: u8,
    payload: &'a [u8],
}

/// 按 `[id][kind][len][payload]` 切分块流；要求完整消费。
fn parse_chunks(buf: &[u8]) -> Result<Vec<Chunk<'_>>, String> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos + CHUNK_HDR <= buf.len() {
        let id = buf[pos];
        let kind = buf[pos + 1];
        let len =
            u32::from_le_bytes([buf[pos + 2], buf[pos + 3], buf[pos + 4], buf[pos + 5]]) as usize;
        let start = pos
            .checked_add(CHUNK_HDR)
            .ok_or_else(|| "分块头溢出".to_string())?;
        let end = start
            .checked_add(len)
            .ok_or_else(|| "分块长度溢出".to_string())?;
        if end > buf.len() {
            return Err(format!(
                "分块越界 id=0x{id:02x} kind=0x{kind:02x} len={len} pos={pos}"
            ));
        }
        out.push(Chunk {
            id,
            kind,
            payload: &buf[start..end],
        });
        pos = end;
    }
    if pos != buf.len() {
        return Err(format!("分块流未完整消费，剩余 {} 字节", buf.len() - pos));
    }
    Ok(out)
}

fn read_u32(b: &[u8]) -> Result<u32, String> {
    if b.len() < 4 {
        return Err(format!("u32 越界：长度 {}", b.len()));
    }
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn zlib_decompress(comp: &[u8], expected: usize) -> Result<Vec<u8>, String> {
    let dec = ZlibDecoder::new(comp);
    let mut out = Vec::with_capacity(expected.clamp(64, MAX_DECOMPRESSED_BYTES));
    let mut limited = dec.take(MAX_DECOMPRESSED_BYTES as u64 + 1);
    limited
        .read_to_end(&mut out)
        .map_err(|e| format!("zlib 解压失败：{e}"))?;
    if out.len() > MAX_DECOMPRESSED_BYTES {
        return Err("zlib 解压输出超过上限".into());
    }
    Ok(out)
}

/// 解码 Domino `MidiPortalSequence` 原始载荷。
pub fn decode_domino_clipboard(raw: &[u8]) -> Result<Vec<DominoNote>, String> {
    if raw.len() < 22 || &raw[..18] != PORTAL_MAGIC {
        return Err("不是 Domino PortalSequenceData 剪贴板数据".into());
    }
    let size = read_u32(&raw[18..22])? as usize;
    let body = zlib_decompress(&raw[22..], size)?;
    let top = parse_chunks(&body)?;
    let eb = top
        .iter()
        .find(|c| c.id == 0xEB && c.kind == 0x03)
        .ok_or_else(|| "缺少 eb(0xeb) 音符容器块".to_string())?;
    let inner = parse_chunks(eb.payload)?;

    let mut notes = Vec::new();
    for c in &inner {
        if c.id != 0xD1 || c.kind != 0x07 {
            continue;
        }
        let fields = parse_chunks(c.payload)?;
        let mut start = 0u32;
        let mut key = 0u8;
        let mut velocity = 100u8;
        let mut gate = 0u32;
        for f in fields {
            match (f.id, f.kind) {
                (0xE9, 0x03) => start = read_u32(f.payload)?,
                (0xD1, 0x07) if f.payload.len() == 1 => key = f.payload[0],
                (0xD2, 0x07) if f.payload.len() == 1 => velocity = f.payload[0].min(127),
                (0xD3, 0x07) => gate = read_u32(f.payload)?,
                _ => {}
            }
        }
        notes.push(DominoNote {
            start_tick: start,
            gate_ticks: gate,
            key: key.min(127),
            velocity,
        });
    }
    if notes.is_empty() {
        return Err("Domino 数据中未找到可解析的音符记录".into());
    }
    Ok(notes)
}

/// 把 Domino 音符转成 yinhe 剪贴板：tick 从 480 PPQ 重采样到目标 PPQ，
/// 所有音符落到 `target_track`。
pub fn to_notes_clipboard(
    notes: &[DominoNote],
    target_ppq: u32,
    target_track: u16,
) -> NotesClipboard {
    let ratio = target_ppq.max(1) as f64 / DOMINO_PPQ as f64;
    let materialized = notes
        .iter()
        .map(|n| {
            let start = (n.start_tick as f64 * ratio).round() as u32;
            let gate = (n.gate_ticks as f64 * ratio).round() as u32;
            (
                Note {
                    id: 0,
                    start_tick: start,
                    end_tick: start.saturating_add(gate),
                    velocity: n.velocity,
                    track: target_track,
                },
                n.key,
            )
        })
        .collect();
    NotesClipboard::from_materialized(materialized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::ZlibEncoder;
    use std::io::Write;

    fn chunk(id: u8, kind: u8, payload: &[u8]) -> Vec<u8> {
        let mut v = Vec::with_capacity(CHUNK_HDR + payload.len());
        v.push(id);
        v.push(kind);
        v.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        v.extend_from_slice(payload);
        v
    }

    fn note_record(start: u32, gate: u32, key: u8, vel: u8) -> Vec<u8> {
        let mut f = Vec::new();
        f.extend(chunk(0xE9, 0x03, &start.to_le_bytes()));
        f.extend(chunk(0xD1, 0x07, &[key]));
        f.extend(chunk(0xD2, 0x07, &[vel]));
        f.extend(chunk(0xD3, 0x07, &gate.to_le_bytes()));
        chunk(0xD1, 0x07, &f)
    }

    fn payload(notes: &[DominoNote]) -> Vec<u8> {
        let mut inner = Vec::new();
        for n in notes {
            inner.extend(note_record(n.start_tick, n.gate_ticks, n.key, n.velocity));
        }
        let body = chunk(0xEB, 0x03, &inner);
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::fast());
        enc.write_all(&body).unwrap();
        let comp = enc.finish().unwrap();
        let mut raw = Vec::new();
        raw.extend_from_slice(PORTAL_MAGIC);
        raw.extend_from_slice(&(body.len() as u32).to_le_bytes());
        raw.extend_from_slice(&comp);
        raw
    }

    #[test]
    fn decodes_notes_and_resamples_ppq() {
        let src = [
            DominoNote {
                start_tick: 0,
                gate_ticks: 480,
                key: 60,
                velocity: 100,
            },
            DominoNote {
                start_tick: 480,
                gate_ticks: 240,
                key: 67,
                velocity: 90,
            },
        ];
        let decoded = decode_domino_clipboard(&payload(&src)).expect("decode");
        assert_eq!(decoded, src);

        let cb = to_notes_clipboard(&decoded, 960, 3);
        let out = cb.collect();
        assert_eq!(out.len(), 2);
        assert_eq!((out[0].0.start_tick, out[0].0.end_tick), (0, 960));
        assert_eq!((out[1].0.start_tick, out[1].0.end_tick), (960, 1440));
        assert_eq!(out[0].1, 60);
        assert_eq!(out[1].1, 67);
        assert!(out.iter().all(|(n, _)| n.track == 3));
    }

    #[test]
    fn rejects_non_domino_and_corrupt_chunks() {
        assert!(decode_domino_clipboard(b"not domino").is_err());

        let mut raw = Vec::new();
        raw.extend_from_slice(PORTAL_MAGIC);
        raw.extend_from_slice(&u32::MAX.to_le_bytes());
        raw.extend_from_slice(&[0u8; 8]);
        assert!(decode_domino_clipboard(&raw).is_err());

        assert!(parse_chunks(&[0xEB, 0x03, 0xFF, 0xFF, 0xFF, 0xFF]).is_err());
    }
}
