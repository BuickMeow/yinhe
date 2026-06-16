/// LEB128-style unsigned varint writer.
pub fn write_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

/// LEB128-style unsigned varint reader. Returns None on truncation.
pub fn read_varint(buf: &[u8], cursor: &mut usize) -> Option<u64> {
    let mut result: u64 = 0;
    let mut shift: u32 = 0;
    loop {
        if *cursor >= buf.len() {
            return None;
        }
        let b = buf[*cursor];
        *cursor += 1;
        result |= ((b & 0x7F) as u64) << shift;
        if b & 0x80 == 0 {
            return Some(result);
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
}

/// Zigzag-encode a signed integer into an unsigned integer for varint encoding.
pub fn zigzag_encode(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

/// Zigzag-decode an unsigned integer back to a signed integer.
pub fn zigzag_decode(v: u64) -> i64 {
    let v = v as i64;
    (v >> 1) ^ -(v & 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_varint_roundtrip() {
        let values = [0u64, 1, 127, 128, 255, 256, 16383, 16384, 0x3FFF_FFFF, u64::MAX];
        for &v in &values {
            let mut buf = Vec::new();
            write_varint(&mut buf, v);
            let mut cursor = 0;
            let decoded = read_varint(&buf, &mut cursor).unwrap();
            assert_eq!(decoded, v, "varint roundtrip failed for {}", v);
            assert_eq!(cursor, buf.len(), "cursor should consume all bytes for {}", v);
        }
    }

    #[test]
    fn write_varint_invalid_empty() {
        let mut cursor = 0;
        let result = read_varint(&[], &mut cursor);
        assert!(result.is_none());
    }

    #[test]
    fn zigzag_roundtrip() {
        let values: Vec<i64> = vec![0, 1, -1, 127, -128, i16::MAX as i64, i16::MIN as i64, 0, -1];
        for &v in &values {
            let encoded = zigzag_encode(v);
            let decoded = zigzag_decode(encoded);
            assert_eq!(decoded, v, "zigzag roundtrip failed for {}", v);
        }
    }

    #[test]
    fn zigzag_encoding_table() {
        assert_eq!(zigzag_encode(0), 0);
        assert_eq!(zigzag_encode(-1), 1);
        assert_eq!(zigzag_encode(1), 2);
        assert_eq!(zigzag_encode(-2), 3);
        assert_eq!(zigzag_encode(2), 4);
    }
}
