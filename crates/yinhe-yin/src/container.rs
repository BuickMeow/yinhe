//! Container framing: header + 3 length-prefixed sections.

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
}

/// Pack header + sections into the final byte buffer.
pub(crate) fn pack(sections: Sections) -> Vec<u8> {
    let mixer_len = sections.mixer.as_ref().map_or(0, |m| 4 + m.len());
    let total = 4
        + 2
        + 4
        + sections.project_json.len()
        + 4
        + sections.mapping_json.len()
        + 4
        + sections.data.len()
        + mixer_len;
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
    let mixer = if cur.pos < bytes.len() {
        let mixer_len = cur.read_u32()? as usize;
        Some(cur.take(mixer_len)?.to_vec())
    } else {
        None
    };
    Ok(Sections {
        project_json,
        mapping_json,
        data,
        mixer,
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
