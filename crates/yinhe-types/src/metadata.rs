//! Score-level metadata event types: key signature, marker, lyrics, chord.
//!
//! 这些事件类型与 `TimeSigEvent` 同级，用于承载 SMF meta 事件中的
//! 调号 (FF 59)、标记 (FF 06/07)、歌词 (FF 05) 以及和弦（非 SMF 标准，
//! 用 Text meta 或自定义格式存储）。

use serde::{Deserialize, Serialize};

/// 音阶类型（业界标准音阶）。
///
/// yinhe 内部使用比 MIDI FF 59 更丰富的音阶表示。
/// 导出 MIDI 时，非大小调音阶会被替换为最接近的大/小调（见 `to_midi_sf_mi`）。
///
/// 半音间隔从根音开始，例如大调 `[2,2,1,2,2,2,1]` 对应 C-D-E-F-G-A-B-C。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ScaleType {
    Major,
    NaturalMinor,
    HarmonicMinor,
    MelodicMinor,
    Dorian,
    Phrygian,
    Lydian,
    Mixolydian,
    Locrian,
    MajorPentatonic,
    MinorPentatonic,
    Blues,
    WholeTone,
    // ── 追加变体（保持上方索引不变以兼容已存工程；UI 顺序由 `ALL` 控制）──
    HarmonicMajor,
    DoubleHarmonicMajor,
    Altered,
    Ionian,
    Aeolian,
    Diminished,
    CombinationDiminished,
    Augmented,
    MajorBlues,
    RagaBhairav,
    RagaGamanasrama,
    RagaTodi,
    Spanish,
    Gypsy,
    Arabian,
    Egyptian,
    Hawaiian,
    Pelog,
    Miyakobushi,
    Ryukyu,
}

impl ScaleType {
    /// 返回从根音开始的半音间隔。
    ///
    /// 八度内半音数和必须为 12（`intervals_sum_to_octave` 测试保证）。
    /// 异域音阶（Raga/Pelog/…）采用业界常用的西方十二平均律近似。
    pub const fn intervals(&self) -> &'static [u8] {
        match self {
            Self::Major => &[2, 2, 1, 2, 2, 2, 1],
            Self::NaturalMinor => &[2, 1, 2, 2, 1, 2, 2],
            Self::HarmonicMinor => &[2, 1, 2, 2, 1, 3, 1],
            Self::MelodicMinor => &[2, 1, 2, 2, 2, 2, 1],
            Self::Dorian => &[2, 1, 2, 2, 2, 1, 2],
            Self::Phrygian => &[1, 2, 2, 2, 1, 2, 2],
            Self::Lydian => &[2, 2, 2, 1, 2, 2, 1],
            Self::Mixolydian => &[2, 2, 1, 2, 2, 1, 2],
            Self::Locrian => &[1, 2, 2, 1, 2, 2, 2],
            Self::MajorPentatonic => &[2, 2, 3, 2, 3],
            Self::MinorPentatonic => &[3, 2, 2, 3, 2],
            Self::Blues => &[3, 2, 1, 1, 3, 2],
            Self::WholeTone => &[2, 2, 2, 2, 2, 2],
            Self::HarmonicMajor => &[2, 2, 1, 2, 1, 3, 1],
            Self::DoubleHarmonicMajor => &[1, 3, 1, 2, 1, 3, 1],
            Self::Altered => &[1, 2, 1, 2, 2, 2, 2],
            Self::Ionian => &[2, 2, 1, 2, 2, 2, 1],
            Self::Aeolian => &[2, 1, 2, 2, 1, 2, 2],
            Self::Diminished => &[2, 1, 2, 1, 2, 1, 2, 1],
            Self::CombinationDiminished => &[1, 2, 1, 2, 1, 2, 1, 2],
            Self::Augmented => &[3, 1, 3, 1, 3, 1],
            Self::MajorBlues => &[2, 1, 1, 3, 2, 3],
            Self::RagaBhairav => &[1, 3, 1, 2, 1, 3, 1],
            Self::RagaGamanasrama => &[1, 3, 2, 1, 1, 3, 1],
            Self::RagaTodi => &[1, 2, 2, 2, 1, 2, 2],
            Self::Spanish => &[1, 3, 1, 2, 1, 2, 2],
            Self::Gypsy => &[2, 1, 3, 1, 1, 3, 1],
            Self::Arabian => &[2, 2, 1, 1, 2, 2, 2],
            Self::Egyptian => &[2, 3, 2, 3, 2],
            Self::Hawaiian => &[2, 1, 2, 2, 2, 2, 1],
            Self::Pelog => &[1, 2, 2, 2, 2, 1, 2],
            Self::Miyakobushi => &[1, 4, 1, 4, 2],
            Self::Ryukyu => &[4, 1, 2, 1, 4],
        }
    }

    /// 返回该音阶在指定根音下的调内音 pitch class 集合（12 位 bitmask，bit i = pc i 在调内）。
    ///
    /// 例：C 大调 → 0b101010110101（C/D/E/F/G/A/B 在调内，C#/D#/F#/G#/A# 不在）。
    pub fn pitch_classes(&self, root: u8) -> u16 {
        let root = (root % 12) as u16;
        let mut mask = 1u16 << root;
        let mut pc = root;
        for &interval in self.intervals() {
            pc = (pc + interval as u16) % 12;
            mask |= 1u16 << pc;
        }
        mask
    }

    /// 所有音阶变体（UI 列表顺序，对齐 Domino 音阶表）。与 `english_name` 对应。
    pub const ALL: &[ScaleType] = &[
        Self::Major,
        Self::NaturalMinor,
        Self::HarmonicMinor,
        Self::MelodicMinor,
        Self::HarmonicMajor,
        Self::DoubleHarmonicMajor,
        Self::Altered,
        Self::Ionian,
        Self::Dorian,
        Self::Phrygian,
        Self::Lydian,
        Self::Mixolydian,
        Self::Aeolian,
        Self::Locrian,
        Self::Diminished,
        Self::CombinationDiminished,
        Self::Augmented,
        Self::WholeTone,
        Self::MajorPentatonic,
        Self::MinorPentatonic,
        Self::MajorBlues,
        Self::Blues,
        Self::RagaBhairav,
        Self::RagaGamanasrama,
        Self::RagaTodi,
        Self::Spanish,
        Self::Gypsy,
        Self::Arabian,
        Self::Egyptian,
        Self::Hawaiian,
        Self::Pelog,
        Self::Miyakobushi,
        Self::Ryukyu,
    ];

    /// 英文名（时间码调式显示用，如 `C Natural Major`）。
    pub const fn english_name(&self) -> &'static str {
        match self {
            Self::Major => "Natural Major",
            Self::NaturalMinor => "Natural Minor",
            Self::HarmonicMinor => "Harmonic Minor",
            Self::MelodicMinor => "Melodic Minor",
            Self::HarmonicMajor => "Harmonic Major",
            Self::DoubleHarmonicMajor => "Double Harmonic Major",
            Self::Altered => "Altered",
            Self::Ionian => "Ionian",
            Self::Dorian => "Dorian",
            Self::Phrygian => "Phrygian",
            Self::Lydian => "Lydian",
            Self::Mixolydian => "Mixolydian",
            Self::Aeolian => "Aeolian",
            Self::Locrian => "Locrian",
            Self::Diminished => "Diminish",
            Self::CombinationDiminished => "Combination Diminish",
            Self::Augmented => "Augment",
            Self::WholeTone => "Wholetone",
            Self::MajorPentatonic => "Major Pentatonic",
            Self::MinorPentatonic => "Minor Pentatonic",
            Self::MajorBlues => "Major Blues",
            Self::Blues => "Minor Blues",
            Self::RagaBhairav => "Raga Bhairav",
            Self::RagaGamanasrama => "Raga Gamanasrama",
            Self::RagaTodi => "Raga Todi",
            Self::Spanish => "Spanish",
            Self::Gypsy => "Gypsy",
            Self::Arabian => "Arabian",
            Self::Egyptian => "Egyptian",
            Self::Hawaiian => "Hawaiian",
            Self::Pelog => "Pelog",
            Self::Miyakobushi => "Miyakobushi",
            Self::Ryukyu => "Ryukyu",
        }
    }

    pub const fn display_name(&self) -> &'static str {
        match self {
            Self::Major => "自然大调",
            Self::NaturalMinor => "自然小调",
            Self::HarmonicMinor => "和声小调",
            Self::MelodicMinor => "旋律小调",
            Self::HarmonicMajor => "和声大调",
            Self::DoubleHarmonicMajor => "双和声大调",
            Self::Altered => "变化音阶",
            Self::Ionian => "伊奥尼亚",
            Self::Dorian => "多利亚",
            Self::Phrygian => "弗里几亚",
            Self::Lydian => "利底亚",
            Self::Mixolydian => "混合利底亚",
            Self::Aeolian => "爱奥利亚",
            Self::Locrian => "洛克里亚",
            Self::Diminished => "减音阶",
            Self::CombinationDiminished => "组合减音阶",
            Self::Augmented => "增音阶",
            Self::WholeTone => "全音阶",
            Self::MajorPentatonic => "大调五声",
            Self::MinorPentatonic => "小调五声",
            Self::MajorBlues => "大调布鲁斯",
            Self::Blues => "小调布鲁斯",
            Self::RagaBhairav => "拜拉夫拉格",
            Self::RagaGamanasrama => "伽摩那室利拉格",
            Self::RagaTodi => "托迪拉格",
            Self::Spanish => "西班牙音阶",
            Self::Gypsy => "吉普赛音阶",
            Self::Arabian => "阿拉伯音阶",
            Self::Egyptian => "埃及音阶",
            Self::Hawaiian => "夏威夷音阶",
            Self::Pelog => "佩洛格",
            Self::Miyakobushi => "都节音阶",
            Self::Ryukyu => "琉球音阶",
        }
    }

    /// 转换为 MIDI FF 59 的 (sf, mi)。
    ///
    /// 大调 → (大调根音对应的升降号数, 0)
    /// 自然小调 → (关系大调的升降号数, 1)
    /// 其他音阶 → 替换为最接近的大/小调：含大三度的归大调、含小三度的归小调。
    pub fn to_midi_sf_mi(&self, root: u8) -> (i8, u8) {
        let root = root % 12;
        match self {
            Self::Major
            | Self::HarmonicMajor
            | Self::DoubleHarmonicMajor
            | Self::Ionian
            | Self::Lydian
            | Self::Mixolydian
            | Self::MajorPentatonic
            | Self::MajorBlues
            | Self::WholeTone
            | Self::Augmented
            | Self::Arabian
            | Self::RagaBhairav
            | Self::RagaGamanasrama
            | Self::Ryukyu => (major_root_to_sf(root), 0),
            Self::NaturalMinor
            | Self::HarmonicMinor
            | Self::MelodicMinor
            | Self::Aeolian
            | Self::Dorian
            | Self::Phrygian
            | Self::Locrian
            | Self::Altered
            | Self::Diminished
            | Self::CombinationDiminished
            | Self::MinorPentatonic
            | Self::Blues
            | Self::RagaTodi
            | Self::Spanish
            | Self::Gypsy
            | Self::Egyptian
            | Self::Hawaiian
            | Self::Pelog
            | Self::Miyakobushi => (major_root_to_sf((root + 3) % 12), 1),
        }
    }
}

/// 大调根音 (0=C, 1=C#/Db, ...) → 五线谱升降号数 sf (-7..=7)。
///
/// 对同音异名的根音（如 C#/Db）选绝对值较小的 sf，避免极端调号。
const fn major_root_to_sf(root: u8) -> i8 {
    const TABLE: [i8; 12] = [
        0,  // C
        -5, // Db（C# 选 -5 降号而非 +7 升号）
        2,  // D
        -3, // Eb
        4,  // E
        -1, // F
        -6, // Gb（F# 选 -6 降号而非 +6 升号）
        1,  // G
        -4, // Ab
        3,  // A
        -2, // Bb
        5,  // B
    ];
    TABLE[(root % 12) as usize]
}

/// MIDI FF 59 (sf, mi) → (root, scale)。
///
/// mi=0: 大调，root = sf 对应的大调主音。
/// mi=1: 小调，root = sf 对应的关系小调主音（大调主音下方小三度）。
pub fn from_midi_sf_mi(sf: i8, mi: u8) -> (u8, ScaleType) {
    let major_root = sf_to_major_root(sf);
    match mi {
        1 => ((major_root + 9) % 12, ScaleType::NaturalMinor), // 关系小调 = 大调根音 + 9 (mod 12)
        _ => (major_root, ScaleType::Major),
    }
}

/// sf (-7..=7) → 大调根音 pitch class (0=C)。
const fn sf_to_major_root(sf: i8) -> u8 {
    // 升号侧：C G D A E B F# C#
    // 降号侧：F Bb Eb Ab Db Gb Cb
    match sf {
        0 => 0,   // C
        1 => 7,   // G
        2 => 2,   // D
        3 => 9,   // A
        4 => 4,   // E
        5 => 11,  // B
        6 => 6,   // F#
        7 => 1,   // C#
        -1 => 5,  // F
        -2 => 10, // Bb
        -3 => 3,  // Eb
        -4 => 8,  // Ab
        -5 => 1,  // Db (同 C#)
        -6 => 6,  // Gb (同 F#)
        -7 => 11, // Cb (同 B)
        _ => 0,   // 越界回退 C
    }
}

/// 调号事件。
///
/// yinhe 内部用 `(root, scale)` 表示调号，比 MIDI FF 59 的 `(sf, mi)` 更丰富：
/// `root` = 根音 pitch class (0=C, 1=C#/Db, ..., 11=B)；
/// `scale` = 音阶类型（大调/小调/多利亚/...）。
///
/// 导出 MIDI 时，非大小调音阶由 `ScaleType::to_midi_sf_mi` 替换为最接近的大/小调。
///
/// serde 兼容性：postcard/bincode 不支持 `deserialize_any`（untagged 枚举依赖它），
/// 所以这里直接用派生实现，与派生 `Serialize` 的线格式一一对应。
/// 早期版本曾用 `#[serde(untagged)]` 迁移旧 `{sf, mi}` 格式，
/// 但该迁移对二进制格式从来无法工作，已删除。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KeySigEvent {
    pub tick: u32,
    pub root: u8,
    pub scale: ScaleType,
}

/// 标记事件（MIDI Meta FF 06 Marker / FF 07 CueMarker）。
///
/// SMF 标准中 Marker 和 CueMarker 都是文本标记，区别在于 DAW 的显示方式。
/// 这里统一存储，不区分 kind——Marker 一般用于段落标记（如 "Verse"、"Chorus"）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MarkerEvent {
    pub tick: u32,
    pub text: String,
}

/// 歌词事件（MIDI Meta FF 05 Lyrics）。
///
/// SMF 标准中歌词是 per-track 的，每个 syllable 一个事件。
/// 这里存储原始文本片段，不做 syllable 拆分。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LyricsEvent {
    pub tick: u32,
    pub text: String,
}

/// 和弦事件（非 SMF 标准）。
///
/// SMF 没有专门的和弦 meta event。部分 DAW 用 Text (FF 01) 以特定格式
/// 表示和弦（如 "Cmaj7"、"Am7b5"）。这里用 `text` 字段存储和弦名称，
/// 解析/写出时走 Text meta。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChordEvent {
    pub tick: u32,
    pub text: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每个音阶的相邻音程之和必须为一个八度（12 半音），且每段间隔 > 0。
    /// 防止新增/编辑异域音阶时写错音程（列表 / 预览 / 调内音集合都会错）。
    #[test]
    fn intervals_sum_to_octave() {
        for &scale in ScaleType::ALL {
            let sum: u32 = scale.intervals().iter().map(|&x| x as u32).sum();
            assert_eq!(sum, 12, "{:?} 音程和应为 12，实际 {sum}", scale);
            assert!(
                scale.intervals().iter().all(|&x| x > 0),
                "{:?} 存在 0 间隔",
                scale
            );
            assert!(
                scale.intervals().len() >= 5,
                "{:?} 音数过少（{}）",
                scale,
                scale.intervals().len()
            );
        }
    }

    /// C 大调应包含 C D E F G A B（pc 0,2,4,5,7,9,11），不含黑键。
    #[test]
    fn test_major_scale_pitch_classes() {
        let mask = ScaleType::Major.pitch_classes(0);
        // C D E F G A B
        for pc in [0u8, 2, 4, 5, 7, 9, 11] {
            assert!(mask & (1u16 << pc) != 0, "pc {} should be in C major", pc);
        }
        // C# D# F# G# A#
        for pc in [1u8, 3, 6, 8, 10] {
            assert!(
                mask & (1u16 << pc) == 0,
                "pc {} should NOT be in C major",
                pc
            );
        }
    }

    /// 根音必须始终在调内。
    #[test]
    fn test_root_always_in_scale() {
        for &scale in ScaleType::ALL {
            for root in 0u8..12 {
                let mask = scale.pitch_classes(root);
                assert!(
                    mask & (1u16 << root) != 0,
                    "root {} not in scale {:?} pitch_classes",
                    root,
                    scale
                );
            }
        }
    }

    /// 全音阶只有 6 个音（间隔全为 2）。
    #[test]
    fn test_whole_tone_has_six_notes() {
        let mask = ScaleType::WholeTone.pitch_classes(0);
        assert_eq!(mask.count_ones(), 6);
    }

    /// 五声音阶只有 5 个音。
    #[test]
    fn test_pentatonic_has_five_notes() {
        let mask = ScaleType::MajorPentatonic.pitch_classes(0);
        assert_eq!(mask.count_ones(), 5);
        let mask = ScaleType::MinorPentatonic.pitch_classes(0);
        assert_eq!(mask.count_ones(), 5);
    }

    /// 七音音阶应有 7 个音。
    #[test]
    fn test_seven_note_scales() {
        for &scale in &[
            ScaleType::Major,
            ScaleType::NaturalMinor,
            ScaleType::HarmonicMinor,
            ScaleType::MelodicMinor,
            ScaleType::Dorian,
            ScaleType::Phrygian,
            ScaleType::Lydian,
            ScaleType::Mixolydian,
            ScaleType::Locrian,
        ] {
            let mask = scale.pitch_classes(0);
            assert_eq!(mask.count_ones(), 7, "{:?} should have 7 notes", scale);
        }
    }

    /// MIDI 往返：大调 (sf, 0) → (root, Major) → (sf', 0)。
    /// root pitch class 必须一致；sf 在等音异名情况下可能变化
    /// （如 Cb major sf=-7 → root=11 → B major sf=5）。
    #[test]
    fn test_midi_roundtrip_major() {
        for sf in -7..=7i8 {
            let major_root = sf_to_major_root(sf);
            let (root, scale) = from_midi_sf_mi(sf, 0);
            assert_eq!(scale, ScaleType::Major);
            assert_eq!(root, major_root);
            let (sf2, mi2) = scale.to_midi_sf_mi(root);
            assert_eq!(mi2, 0);
            // root 必须能往返，sf 在等音异名下可能不同
            assert_eq!(sf_to_major_root(sf2), major_root);
        }
    }

    /// MIDI 往返：小调 (sf, 1) → (root, NaturalMinor) → (sf', 1)。
    /// root pitch class 必须一致；sf 在等音异名情况下可能变化。
    #[test]
    fn test_midi_roundtrip_minor() {
        for sf in -7..=7i8 {
            let (root, scale) = from_midi_sf_mi(sf, 1);
            assert_eq!(scale, ScaleType::NaturalMinor);
            let (sf2, mi2) = scale.to_midi_sf_mi(root);
            assert_eq!(mi2, 1);
            // root 必须能往返
            let (root2, _) = from_midi_sf_mi(sf2, 1);
            assert_eq!(root2, root);
        }
    }

    /// 新格式 {root, scale} 正常反序列化。
    #[test]
    fn test_new_keysig_deserialize() {
        let json = r#"{"tick":1920,"root":9,"scale":"Dorian"}"#;
        let ev: KeySigEvent = serde_json::from_str(json).unwrap();
        assert_eq!(ev.tick, 1920);
        assert_eq!(ev.root, 9);
        assert_eq!(ev.scale, ScaleType::Dorian);
    }

    /// 非大小调音阶导出时替换为最接近的大/小调。
    #[test]
    fn test_non_major_minor_export_fallback() {
        // 布鲁斯 → 小调
        let (_sf, mi) = ScaleType::Blues.to_midi_sf_mi(0);
        assert_eq!(mi, 1);
        // 利底亚 → 大调
        let (_sf, mi) = ScaleType::Lydian.to_midi_sf_mi(0);
        assert_eq!(mi, 0);
    }
}
