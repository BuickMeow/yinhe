//! 二进制编码工具：postcard 包装 + varint/zigzag + 带版本头的段编码。

use std::io::Cursor;

use crate::error::{YinError, invalid_data};

/// 编码带版本头的段：version u32 LE + zstd(payload)。
pub(crate) fn encode_versioned_section(
    version: u32,
    payload: &[u8],
    level: i32,
) -> Result<Vec<u8>, YinError> {
    let comp = zstd::encode_all(Cursor::new(payload), level.clamp(0, 22))?;
    let mut out = Vec::with_capacity(4 + comp.len());
    out.extend_from_slice(&version.to_le_bytes());
    out.extend_from_slice(&comp);
    Ok(out)
}

/// postcard 编码（varint，紧凑二进制）。
pub(crate) fn serialize_postcard<T: serde::Serialize>(v: &T) -> Result<Vec<u8>, YinError> {
    Ok(postcard::to_stdvec(v)?)
}

pub(crate) fn deserialize_postcard<T: serde::de::DeserializeOwned>(
    bytes: &[u8],
) -> Result<T, YinError> {
    Ok(postcard::from_bytes(bytes)?)
}

/// 写 varint（LEB128）。
pub(crate) fn push_varint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let b = (v & 0x7F) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            break;
        }
        out.push(b | 0x80);
    }
}

/// 读 varint（LEB128），游标前进。
pub(crate) fn read_varint(bytes: &[u8], pos: &mut usize) -> Result<u64, YinError> {
    let mut v: u64 = 0;
    let mut shift = 0u32;
    loop {
        let Some(&b) = bytes.get(*pos) else {
            return Err(YinError::Truncated {
                needed: 1,
                available: 0,
            });
        };
        *pos += 1;
        v |= ((b & 0x7F) as u64) << shift;
        if b & 0x80 == 0 {
            return Ok(v);
        }
        shift += 7;
        if shift >= 64 {
            return Err(invalid_data("varint too long"));
        }
    }
}

/// zigzag：有符号 → 无符号（小绝对值映射到小值）。
pub(crate) fn zigzag(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

/// zigzag 逆变换。
pub(crate) fn unzigzag(v: u64) -> i64 {
    ((v >> 1) as i64) ^ -((v & 1) as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_roundtrip() {
        let mut buf = Vec::new();
        for v in [
            0u64,
            1,
            127,
            128,
            300,
            16383,
            16384,
            u32::MAX as u64,
            u64::MAX,
        ] {
            buf.clear();
            push_varint(&mut buf, v);
            let mut pos = 0;
            assert_eq!(read_varint(&buf, &mut pos).unwrap(), v);
            assert_eq!(pos, buf.len(), "游标应停在编码末尾");
        }
    }

    #[test]
    fn zigzag_roundtrip() {
        for v in [
            0i64,
            1,
            -1,
            2,
            -2,
            i32::MAX as i64,
            i32::MIN as i64,
            i64::MAX,
            i64::MIN,
        ] {
            assert_eq!(unzigzag(zigzag(v)), v);
        }
    }

    #[test]
    fn read_varint_rejects_truncated_and_overlong() {
        let mut pos = 0;
        assert!(read_varint(&[], &mut pos).is_err());
        // 连续 continuation 超过 64 位。
        let buf = vec![0xFFu8; 11];
        let mut pos = 0;
        assert!(read_varint(&buf, &mut pos).is_err());
    }

    #[test]
    fn versioned_section_roundtrip() {
        let payload = b"hello versioned section";
        let sec = encode_versioned_section(7, payload, 3).unwrap();
        assert_eq!(&sec[..4], &7u32.to_le_bytes());
        let decoded = zstd::decode_all(Cursor::new(&sec[4..])).unwrap();
        assert_eq!(decoded, payload);
    }
}
