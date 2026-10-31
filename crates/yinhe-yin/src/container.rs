//! Container framing: header + length-prefixed sections.
//!
//! 固定 3 段（project.json / mapping.json / data）+ 2 个可选段
//! （mixer / audio，老文件没有，读取器按尾部剩余字节惰性识别）。

use crate::error::YinError;
use crate::{MAGIC, VERSION};

/// Byte sections of a .yin file (without the outer header).
pub(crate) struct Sections {
    pub project_json: Vec<u8>,
    pub mapping_json: Vec<u8>,
    pub data: Vec<u8>,
    /// 可选第 4 段：混音台参数（postcard + zstd）。
    /// 老文件没有这一段 → None；老读取器不检查尾部，读新文件时自动忽略。
    pub mixer: Option<Vec<u8>>,
    /// 可选第 5 段：音频素材（内嵌原始文件字节，postcard + zstd）。
    /// 老文件没有这一段 → None；老读取器读完 mixer 段即停，忽略本段。
    pub audio: Option<Vec<u8>>,
}

/// Pack header + sections into the final byte buffer.
pub(crate) fn pack(sections: Sections) -> Vec<u8> {
    // 段序由位置隐含（无段 id）：有音频段而 mixer 段为 None 时，必须写一个
    // 长度为 0 的 mixer 占位，否则读取侧会把音频段当作 mixer 段。
    // 空 mixer 段解码为 None，不丢任何设置。
    let mixer_len = sections.mixer.as_ref().map_or(0, |m| 4 + m.len());
    let audio_len = sections.audio.as_ref().map_or(0, |a| 4 + a.len());
    let mixer_placeholder = if sections.audio.is_some() && sections.mixer.is_none() {
        4
    } else {
        0
    };
    let total = 4
        + 2
        + 4
        + sections.project_json.len()
        + 4
        + sections.mapping_json.len()
        + 4
        + sections.data.len()
        + mixer_len
        + mixer_placeholder
        + audio_len;
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&(sections.project_json.len() as u32).to_le_bytes());
    out.extend_from_slice(&sections.project_json);
    out.extend_from_slice(&(sections.mapping_json.len() as u32).to_le_bytes());
    out.extend_from_slice(&sections.mapping_json);
    out.extend_from_slice(&(sections.data.len() as u32).to_le_bytes());
    out.extend_from_slice(&sections.data);
    if let Some(mixer) = &sections.mixer {
        out.extend_from_slice(&(mixer.len() as u32).to_le_bytes());
        out.extend_from_slice(mixer);
    }
    if let Some(audio) = &sections.audio {
        if sections.mixer.is_none() {
            out.extend_from_slice(&0u32.to_le_bytes());
        }
        out.extend_from_slice(&(audio.len() as u32).to_le_bytes());
        out.extend_from_slice(audio);
    }
    out
}

/// Parse header + extract the three sections from a .yin byte buffer.
pub(crate) fn unpack(bytes: &[u8]) -> Result<Sections, YinError> {
    let mut cur = Cursor::new(bytes);
    let magic = cur.take(4)?;
    if magic != MAGIC {
        return Err(YinError::BadMagic);
    }
    let version = cur.read_u16()?;
    if version != VERSION {
        return Err(YinError::BadVersion(version));
    }
    let project_len = cur.read_u32()? as usize;
    let project_json = cur.take(project_len)?.to_vec();
    let mapping_len = cur.read_u32()? as usize;
    let mapping_json = cur.take(mapping_len)?.to_vec();
    let data_len = cur.read_u32()? as usize;
    let data = cur.take(data_len)?.to_vec();
    // 可选第 4 段：尾部还有字节 → 混音台参数段；没有 → 老文件。
    //
    // 可选段不能阻断工程本体：旧版本/中断保存/传输被截断时，尾部可能只剩
    // 半个 mixer/audio 段。旧实现这里直接 `?`，会让整个 9.69M 音符工程
    // 报 "Truncated" 而打不开。混音段损坏时放弃后续音频段；音频段损坏
    // 只记警告，音符/工程照常加载。
    let mut optional_damaged = false;
    let mixer = if cur.pos < bytes.len() {
        match cur
            .read_u32()
            .and_then(|len| cur.take(len as usize).map(|b| b.to_vec()))
        {
            Ok(v) => Some(v),
            Err(e) => {
                tracing::warn!("混音段损坏，忽略：{e}");
                optional_damaged = true;
                None
            }
        }
    } else {
        None
    };
    // 可选第 5 段：混音段之后还有字节 → 音频素材段。
    let audio = if !optional_damaged && cur.pos < bytes.len() {
        match cur
            .read_u32()
            .and_then(|len| cur.take(len as usize).map(|b| b.to_vec()))
        {
            Ok(v) => Some(v),
            Err(e) => {
                tracing::warn!("音频段损坏/被截断，忽略：{e}");
                None
            }
        }
    } else {
        None
    };
    Ok(Sections {
        project_json,
        mapping_json,
        data,
        mixer,
        audio,
    })
}

struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], YinError> {
        if self.pos + n > self.buf.len() {
            return Err(YinError::Truncated {
                needed: n,
                available: self.buf.len() - self.pos,
            });
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn read_u16(&mut self) -> Result<u16, YinError> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn read_u32(&mut self) -> Result<u32, YinError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_bytes(mixer: Option<Vec<u8>>) -> Vec<u8> {
        pack(Sections {
            project_json: b"{}".to_vec(),
            mapping_json: b"{}".to_vec(),
            data: b"note-data".to_vec(),
            mixer,
            audio: None,
        })
    }

    #[test]
    fn truncated_optional_audio_does_not_break_data() {
        let mut bytes = base_bytes(Some(vec![1, 2, 3]));
        // 模拟保存/传输被截断：音频段长度巨大，但后面没有对应 payload。
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());
        let sections = unpack(&bytes).expect("工程本体应可读");
        assert_eq!(sections.data, b"note-data");
        assert_eq!(sections.mixer.as_deref(), Some(&[1, 2, 3][..]));
        assert!(sections.audio.is_none());
    }

    #[test]
    fn truncated_optional_mixer_does_not_break_data() {
        let mut bytes = base_bytes(None);
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());
        let sections = unpack(&bytes).expect("工程本体应可读");
        assert_eq!(sections.data, b"note-data");
        assert!(sections.mixer.is_none());
        assert!(sections.audio.is_none());
    }
}
