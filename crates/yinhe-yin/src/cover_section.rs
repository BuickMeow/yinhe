//! `.yin` 可选封面段：原始图片字节（不 zstd，JPEG/PNG 已压缩）。
//!
//! 布局：`version u8 = 1` + mime + full + thumb，长度均为 u32 LE。

use std::sync::Arc;

use yinhe_core::CoverArt;

fn read_u32(bytes: &[u8], pos: &mut usize) -> Option<u32> {
    let end = pos.checked_add(4)?;
    let v = u32::from_le_bytes(bytes.get(*pos..end)?.try_into().ok()?);
    *pos = end;
    Some(v)
}

fn read_bytes<'a>(bytes: &'a [u8], pos: &mut usize, len: usize) -> Option<&'a [u8]> {
    let end = pos.checked_add(len)?;
    let out = bytes.get(*pos..end)?;
    *pos = end;
    Some(out)
}

pub fn encode_cover_section(cover: &CoverArt) -> Vec<u8> {
    let mime = cover.mime.as_bytes();
    let mut out =
        Vec::with_capacity(1 + 4 + mime.len() + 4 + cover.data.len() + 4 + cover.thumb.len());
    out.push(1);
    out.extend_from_slice(&(mime.len() as u32).to_le_bytes());
    out.extend_from_slice(mime);
    out.extend_from_slice(&(cover.data.len() as u32).to_le_bytes());
    out.extend_from_slice(&cover.data);
    out.extend_from_slice(&(cover.thumb.len() as u32).to_le_bytes());
    out.extend_from_slice(&cover.thumb);
    out
}

pub fn decode_cover_section(bytes: &[u8]) -> Option<CoverArt> {
    let mut pos = 0usize;
    let version = *bytes.first()?;
    pos += 1;
    if version != 1 {
        return None;
    }
    let mime_len = read_u32(bytes, &mut pos)? as usize;
    let mime = String::from_utf8_lossy(read_bytes(bytes, &mut pos, mime_len)?).into_owned();
    let data_len = read_u32(bytes, &mut pos)? as usize;
    let data = read_bytes(bytes, &mut pos, data_len)?.to_vec();
    let thumb_len = read_u32(bytes, &mut pos)? as usize;
    let thumb = read_bytes(bytes, &mut pos, thumb_len)?.to_vec();
    Some(CoverArt {
        mime,
        data: Arc::new(data),
        thumb: Arc::new(thumb),
    })
}
