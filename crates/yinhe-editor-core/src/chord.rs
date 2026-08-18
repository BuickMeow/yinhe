//! 和弦识别（纯逻辑）：把“当前按下的琴键集合”识别为和弦名，供 PR 视图的和弦指示器使用。
//!
//! 防乱按规则：不同音级超过 7 个、或按键跨度超过 24 半音（两个八度）时不识别，
//! 避免双手乱拍被误识别成莫名其妙的和弦。匹配不上任何和弦时也返回 None（宁缺毋滥）。

use yinhe_types::{NOTE_NAMES, key_name};

/// 音级集合转 bitmask：bit i 置位表示音级 i 出现。
const fn pc_mask(pcs: &[u8]) -> u16 {
    let mut mask = 0u16;
    let mut i = 0;
    while i < pcs.len() {
        mask |= 1 << pcs[i];
        i += 1;
    }
    mask
}

/// 和弦表：相对根音的音级 bitmask（精确相等才匹配）到后缀名，全部为 ASCII。
/// 按音数从多到少排列（虽然精确匹配不会误判，但保持优先级清晰）。
const CHORDS: &[(u16, &str)] = &[
    // 五音和弦
    (pc_mask(&[0, 2, 4, 7, 10]), "9"),
    (pc_mask(&[0, 2, 4, 7, 11]), "maj9"),
    (pc_mask(&[0, 2, 3, 7, 10]), "m9"),
    // 四音和弦
    (pc_mask(&[0, 4, 7, 10]), "7"),
    (pc_mask(&[0, 4, 7, 11]), "maj7"),
    (pc_mask(&[0, 3, 7, 10]), "m7"),
    (pc_mask(&[0, 3, 7, 11]), "m(maj7)"),
    (pc_mask(&[0, 3, 6, 9]), "dim7"),
    (pc_mask(&[0, 3, 6, 10]), "m7b5"),
    (pc_mask(&[0, 4, 8, 10]), "aug7"),
    (pc_mask(&[0, 4, 7, 9]), "6"),
    (pc_mask(&[0, 3, 7, 9]), "m6"),
    (pc_mask(&[0, 2, 4, 7]), "add9"),
    (pc_mask(&[0, 2, 3, 7]), "m(add9)"),
    // 三音和弦（大三和弦后缀为空，只显示根音名）
    (pc_mask(&[0, 4, 7]), ""),
    (pc_mask(&[0, 3, 7]), "m"),
    (pc_mask(&[0, 3, 6]), "dim"),
    (pc_mask(&[0, 4, 8]), "aug"),
    (pc_mask(&[0, 2, 7]), "sus2"),
    (pc_mask(&[0, 5, 7]), "sus4"),
    // 二音（power chord）
    (pc_mask(&[0, 7]), "5"),
];

/// 识别按下的琴键集合对应的和弦名。
///
/// - 空输入返回 None
/// - 单个键返回 "C5" 式音名
/// - 两个及以上键做精确和弦匹配；根音不等于低音时用 slash 记法（如 "C/E"）
/// - 匹配不上、音级超过 7 个、或跨度超过 24 半音时返回 None
pub fn recognize(keys: &[u8]) -> Option<String> {
    if keys.is_empty() {
        return None;
    }
    if keys.len() == 1 {
        return Some(key_name(keys[0]));
    }
    let mut mask = 0u16;
    let mut low = u8::MAX;
    let mut high = u8::MIN;
    for &key in keys {
        mask |= 1 << (key % 12);
        low = low.min(key);
        high = high.max(key);
    }
    // 防乱按：音级太多或跨度太大都不识别。
    if mask.count_ones() > 7 || high - low > 24 {
        return None;
    }
    let bass_pc = low % 12;
    // 候选根音：低音优先，其余出现的音级按升序。
    let roots = std::iter::once(bass_pc)
        .chain((0..12u8).filter(|&pc| pc != bass_pc && mask & (1 << pc) != 0));
    for root in roots {
        // 以 root 为根音，把所有出现的音级折算成相对音级。
        let mut rel = 0u16;
        for pc in 0..12u8 {
            if mask & (1 << pc) != 0 {
                rel |= 1 << ((pc + 12 - root) % 12);
            }
        }
        if let Some(&(_, suffix)) = CHORDS.iter().find(|&&(m, _)| m == rel) {
            let name = format!("{}{}", NOTE_NAMES[root as usize], suffix);
            if root == bass_pc {
                return Some(name);
            }
            // 转位：slash 记法标出实际低音。
            return Some(format!("{}/{}", name, NOTE_NAMES[bass_pc as usize]));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::recognize;

    #[test]
    fn single_key_shows_note_name() {
        assert_eq!(recognize(&[60]), Some("C5".to_string()));
    }

    #[test]
    fn major_triad() {
        assert_eq!(recognize(&[60, 64, 67]), Some("C".to_string()));
    }

    #[test]
    fn minor_triad() {
        assert_eq!(recognize(&[60, 63, 67]), Some("Cm".to_string()));
    }

    #[test]
    fn dominant_seventh() {
        assert_eq!(recognize(&[60, 64, 67, 70]), Some("C7".to_string()));
    }

    #[test]
    fn inversion_slash_notation() {
        // C/E：低音是 E。
        assert_eq!(recognize(&[64, 67, 72]), Some("C/E".to_string()));
    }

    #[test]
    fn sus4() {
        assert_eq!(recognize(&[60, 65, 67]), Some("Csus4".to_string()));
    }

    #[test]
    fn dim7() {
        assert_eq!(recognize(&[60, 63, 66, 69]), Some("Cdim7".to_string()));
    }

    #[test]
    fn add9() {
        assert_eq!(recognize(&[60, 62, 64, 67]), Some("Cadd9".to_string()));
    }

    #[test]
    fn ninth() {
        assert_eq!(recognize(&[60, 62, 64, 67, 70]), Some("C9".to_string()));
    }

    #[test]
    fn too_many_pitch_classes_rejected() {
        // 8 个不同音级，属于乱按，不识别。
        assert_eq!(recognize(&[60, 61, 62, 63, 64, 65, 66, 67]), None);
    }

    #[test]
    fn wide_span_rejected() {
        // 跨度 25 半音 > 24，不识别。
        assert_eq!(recognize(&[60, 85]), None);
    }

    #[test]
    fn unrecognized_returns_none() {
        // {0,1} 不在和弦表里。
        assert_eq!(recognize(&[60, 61]), None);
    }

    #[test]
    fn empty_returns_none() {
        assert_eq!(recognize(&[]), None);
    }
}
