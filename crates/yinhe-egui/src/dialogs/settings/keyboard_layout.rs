//! ANSI 104 物理键盘布局表（快捷键设置的键盘视图用）。
//!
//! 只描述「键在键盘上的位置与外观」，不含快捷键逻辑。
//!
//! 尺寸遵循 ANSI 104 的国际标准键位单位（1u = 普通字母键宽）：
//! Tab 1.5u、Caps 1.75u、Enter 2.25u、Backspace 2u、左右 Shift 2.25u/2.75u、
//! Ctrl/Alt/Cmd 各 1.25u、Space 6.25u。
//!
//! 布局按**区块**组织（主键区 / 导航区 / 方向键 / 小键盘），每个区块在行内
//! 用 `indent` 留出间隔。这样方向键不会跑到主键区右边、小键盘也有位置。
//!
//! 修饰键的**称呼随平台变化**（macOS 是 ⌘ Command / ⌥ Option，
//! Windows/Linux 是 Win / Alt），见 [`Modifier::label`]。

use eframe::egui::Key;

/// 纯修饰键（不参与绑定，只占位并按真实状态高亮）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Modifier {
    Shift,
    Ctrl,
    Alt,
    /// macOS 的 ⌘ / Windows 的 Win 键。
    Cmd,
}

impl Modifier {
    /// 该修饰键在当前输入状态下是否按下。
    pub(crate) fn active(self, mods: egui::Modifiers) -> bool {
        match self {
            Modifier::Shift => mods.shift,
            Modifier::Ctrl => mods.ctrl,
            Modifier::Alt => mods.alt,
            Modifier::Cmd => mods.command || mods.mac_cmd,
        }
    }

    /// 平台相关称呼。
    pub(crate) fn label(self) -> &'static str {
        let mac = cfg!(target_os = "macos");
        match self {
            Modifier::Shift => "Shift",
            Modifier::Ctrl => "Ctrl",
            Modifier::Alt => {
                if mac {
                    "Option"
                } else {
                    "Alt"
                }
            }
            Modifier::Cmd => {
                if mac {
                    "Command"
                } else {
                    "Win"
                }
            }
        }
    }
}

/// 一个物理按键。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct KeyCap {
    /// 键位对应的 egui 键；`None` = 纯修饰键 / 占位 / 小键盘装饰键。
    pub key: Option<Key>,
    /// 纯修饰键（不参与绑定）。
    pub modifier: Option<Modifier>,
    /// 键帽标签（修饰键留空，绘制时用 `Modifier::label()`）。
    pub label: &'static str,
    /// 相对宽度（1u = 普通键）。
    pub width: f32,
    /// 视觉上的按键高度倍数（小键盘的 `+` / `Enter` 是 2u 高）。
    pub height: f32,
}

impl KeyCap {
    const fn plain(key: Key, label: &'static str) -> Self {
        Self {
            key: Some(key),
            modifier: None,
            label,
            width: 1.0,
            height: 1.0,
        }
    }
    const fn wide(key: Key, label: &'static str, width: f32) -> Self {
        Self {
            key: Some(key),
            modifier: None,
            label,
            width,
            height: 1.0,
        }
    }
    const fn modifier(m: Modifier, width: f32) -> Self {
        Self {
            key: None,
            modifier: Some(m),
            label: "",
            width,
            height: 1.0,
        }
    }
    /// 空占位（布局对齐用）。
    const fn gap(width: f32) -> Self {
        Self {
            key: None,
            modifier: None,
            label: "",
            width,
            height: 1.0,
        }
    }
    /// 只显示、不可绑定的键（小键盘：egui 没有独立的 Numpad Key，
    /// 它们与主键区共用同一个 `egui::Key`）。
    const fn decorative(label: &'static str, width: f32, height: f32) -> Self {
        Self {
            key: None,
            modifier: None,
            label,
            width,
            height,
        }
    }
}

/// 行内一个区块（主键区 / 导航区 / 方向键 / 小键盘）。
#[derive(Clone, Copy, Debug)]
pub(crate) struct KeySegment {
    /// 该区块相对前一区块的额外间隔（单位 u）。
    pub gap_before: f32,
    pub caps: &'static [KeyCap],
}

const fn seg(gap_before: f32, caps: &'static [KeyCap]) -> KeySegment {
    KeySegment { gap_before, caps }
}

/// 键盘一行：可含多个区块。
#[derive(Clone, Copy, Debug)]
pub(crate) struct KeyRow {
    pub segments: &'static [KeySegment],
}

const fn row(segments: &'static [KeySegment]) -> KeyRow {
    KeyRow { segments }
}

/// ANSI 104 布局（美式全键盘）。
pub(crate) fn ansi_layout() -> &'static [KeyRow] {
    &ANSI
}

/// 每个区块之间的间隔（单位 u）。
const BLOCK_GAP: f32 = 0.6;

static ANSI: [KeyRow; 6] = [
    // ── Row 0：功能键行 + 打印键区 ──
    row(&[
        seg(
            0.0,
            &[
                KeyCap::plain(Key::Escape, "Esc"),
                KeyCap::gap(1.0),
                KeyCap::plain(Key::F1, "F1"),
                KeyCap::plain(Key::F2, "F2"),
                KeyCap::plain(Key::F3, "F3"),
                KeyCap::plain(Key::F4, "F4"),
                KeyCap::gap(0.5),
                KeyCap::plain(Key::F5, "F5"),
                KeyCap::plain(Key::F6, "F6"),
                KeyCap::plain(Key::F7, "F7"),
                KeyCap::plain(Key::F8, "F8"),
                KeyCap::gap(0.5),
                KeyCap::plain(Key::F9, "F9"),
                KeyCap::plain(Key::F10, "F10"),
                KeyCap::plain(Key::F11, "F11"),
                KeyCap::plain(Key::F12, "F12"),
            ],
        ),
        seg(
            BLOCK_GAP,
            &[
                KeyCap::plain(Key::PrintScreen, "PrtSc"),
                KeyCap::plain(Key::ScrollLock, "ScrLk"),
                KeyCap::plain(Key::Pause, "Pause"),
            ],
        ),
    ]),
    // ── Row 1：数字行 + 导航区 + 小键盘首行 ──
    row(&[
        seg(
            0.0,
            &[
                KeyCap::plain(Key::Backtick, "`"),
                KeyCap::plain(Key::Num1, "1"),
                KeyCap::plain(Key::Num2, "2"),
                KeyCap::plain(Key::Num3, "3"),
                KeyCap::plain(Key::Num4, "4"),
                KeyCap::plain(Key::Num5, "5"),
                KeyCap::plain(Key::Num6, "6"),
                KeyCap::plain(Key::Num7, "7"),
                KeyCap::plain(Key::Num8, "8"),
                KeyCap::plain(Key::Num9, "9"),
                KeyCap::plain(Key::Num0, "0"),
                KeyCap::plain(Key::Minus, "-"),
                KeyCap::plain(Key::Equals, "="),
                KeyCap::wide(Key::Backspace, "Backspace", 2.0),
            ],
        ),
        seg(
            BLOCK_GAP,
            &[
                KeyCap::plain(Key::Insert, "Ins"),
                KeyCap::plain(Key::Home, "Home"),
                KeyCap::plain(Key::PageUp, "PgUp"),
            ],
        ),
        seg(
            BLOCK_GAP,
            &[
                KeyCap::decorative("Num", 1.0, 1.0),
                KeyCap::decorative("/", 1.0, 1.0),
                KeyCap::decorative("*", 1.0, 1.0),
                KeyCap::decorative("-", 1.0, 1.0),
            ],
        ),
    ]),
    // ── Row 2：QWERTY + 导航区 + 小键盘 ──
    row(&[
        seg(
            0.0,
            &[
                KeyCap::wide(Key::Tab, "Tab", 1.5),
                KeyCap::plain(Key::Q, "Q"),
                KeyCap::plain(Key::W, "W"),
                KeyCap::plain(Key::E, "E"),
                KeyCap::plain(Key::R, "R"),
                KeyCap::plain(Key::T, "T"),
                KeyCap::plain(Key::Y, "Y"),
                KeyCap::plain(Key::U, "U"),
                KeyCap::plain(Key::I, "I"),
                KeyCap::plain(Key::O, "O"),
                KeyCap::plain(Key::P, "P"),
                KeyCap::plain(Key::OpenBracket, "["),
                KeyCap::plain(Key::CloseBracket, "]"),
                KeyCap::wide(Key::Backslash, "\\", 1.5),
            ],
        ),
        seg(
            BLOCK_GAP,
            &[
                KeyCap::plain(Key::Delete, "Del"),
                KeyCap::plain(Key::End, "End"),
                KeyCap::plain(Key::PageDown, "PgDn"),
            ],
        ),
        seg(
            BLOCK_GAP,
            &[
                KeyCap::decorative("7", 1.0, 1.0),
                KeyCap::decorative("8", 1.0, 1.0),
                KeyCap::decorative("9", 1.0, 1.0),
                KeyCap::decorative("+", 1.0, 2.0),
            ],
        ),
    ]),
    // ── Row 3：ASDF ──
    row(&[
        seg(
            0.0,
            &[
                KeyCap::decorative("Caps", 1.75, 1.0),
                KeyCap::plain(Key::A, "A"),
                KeyCap::plain(Key::S, "S"),
                KeyCap::plain(Key::D, "D"),
                KeyCap::plain(Key::F, "F"),
                KeyCap::plain(Key::G, "G"),
                KeyCap::plain(Key::H, "H"),
                KeyCap::plain(Key::J, "J"),
                KeyCap::plain(Key::K, "K"),
                KeyCap::plain(Key::L, "L"),
                KeyCap::plain(Key::Semicolon, ";"),
                KeyCap::plain(Key::Quote, "'"),
                KeyCap::wide(Key::Enter, "Enter", 2.25),
            ],
        ),
        seg(
            BLOCK_GAP,
            &[
                KeyCap::decorative("4", 1.0, 1.0),
                KeyCap::decorative("5", 1.0, 1.0),
                KeyCap::decorative("6", 1.0, 1.0),
            ],
        ),
    ]),
    // ── Row 4：左 Shift + ZXCV 行 + 右 Shift / 方向键 / 小键盘 ──
    row(&[
        seg(
            0.0,
            &[
                KeyCap::modifier(Modifier::Shift, 2.25),
                KeyCap::plain(Key::Z, "Z"),
                KeyCap::plain(Key::X, "X"),
                KeyCap::plain(Key::C, "C"),
                KeyCap::plain(Key::V, "V"),
                KeyCap::plain(Key::B, "B"),
                KeyCap::plain(Key::N, "N"),
                KeyCap::plain(Key::M, "M"),
                KeyCap::plain(Key::Comma, ","),
                KeyCap::plain(Key::Period, "."),
                KeyCap::plain(Key::Slash, "/"),
                KeyCap::modifier(Modifier::Shift, 2.75),
            ],
        ),
        // 方向键 ↑ 位于导航区列的中部（与 Row 3 的小键盘对齐）。
        seg(
            BLOCK_GAP,
            &[
                KeyCap::gap(1.0),
                KeyCap::plain(Key::ArrowUp, "↑"),
                KeyCap::gap(1.0),
            ],
        ),
        seg(
            BLOCK_GAP,
            &[
                KeyCap::decorative("1", 1.0, 1.0),
                KeyCap::decorative("2", 1.0, 1.0),
                KeyCap::decorative("3", 1.0, 1.0),
                KeyCap::decorative("Enter", 1.0, 2.0),
            ],
        ),
    ]),
    // ── Row 5：底行 + 方向键 ← ↓ → + 小键盘 ──
    row(&[
        seg(
            0.0,
            &[
                KeyCap::modifier(Modifier::Ctrl, 1.25),
                KeyCap::modifier(Modifier::Cmd, 1.25),
                KeyCap::modifier(Modifier::Alt, 1.25),
                KeyCap::wide(Key::Space, "Space", 6.25),
                KeyCap::modifier(Modifier::Alt, 1.25),
                KeyCap::modifier(Modifier::Cmd, 1.25),
                KeyCap::modifier(Modifier::Ctrl, 1.25),
            ],
        ),
        seg(
            BLOCK_GAP,
            &[
                KeyCap::plain(Key::ArrowLeft, "←"),
                KeyCap::plain(Key::ArrowDown, "↓"),
                KeyCap::plain(Key::ArrowRight, "→"),
            ],
        ),
        seg(
            BLOCK_GAP,
            &[
                KeyCap::decorative("0", 2.0, 1.0),
                KeyCap::decorative(".", 1.0, 1.0),
            ],
        ),
    ]),
];

/// `=` 键帽同时代表 `Plus`（Shift+`=`）：两个 Key 共用一个物理键位。
pub(crate) fn alias_keys(key: Key) -> &'static [Key] {
    match key {
        Key::Equals => &[Key::Plus],
        _ => &[],
    }
}

/// 布局里出现过的全部可绑定键（去重）。
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn layout_keys() -> Vec<Key> {
    let mut out: Vec<Key> = Vec::new();
    for row in ansi_layout() {
        for segment in row.segments {
            for cap in segment.caps {
                if let Some(key) = cap.key
                    && !out.contains(&key)
                {
                    out.push(key);
                }
            }
        }
    }
    out
}

/// 一行总宽度（单位 u，含区块间隔）。
pub(crate) fn row_units(row: &KeyRow) -> f32 {
    let mut total = 0.0;
    for (i, segment) in row.segments.iter().enumerate() {
        if i > 0 {
            total += segment.gap_before;
        }
        total += segment.caps.iter().map(|c| c.width).sum::<f32>();
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shortcuts::RECORDABLE_KEYS;

    /// 布局里的可绑定键必须都在 `RECORDABLE_KEYS` 里。
    #[test]
    fn layout_keys_are_all_recordable() {
        for key in layout_keys() {
            assert!(
                RECORDABLE_KEYS.contains(&key),
                "布局里有 {key:?}，但它不在 RECORDABLE_KEYS 里"
            );
        }
    }

    /// 反向：可录制的键都应该能在键盘上找到（别名键也算）。
    #[test]
    fn recordable_keys_are_all_on_layout() {
        let mut on_layout = layout_keys();
        for &key in &on_layout.clone() {
            on_layout.extend_from_slice(alias_keys(key));
        }
        for &key in RECORDABLE_KEYS {
            assert!(
                on_layout.contains(&key),
                "RECORDABLE_KEYS 里的 {key:?} 没有出现在布局上"
            );
        }
    }

    /// 同一个键不能出现两次（修饰键/装饰键不参与绑定）。
    #[test]
    fn layout_has_no_duplicate_bindable_keys() {
        let mut seen: Vec<Key> = Vec::new();
        for row in ansi_layout() {
            for segment in row.segments {
                for cap in segment.caps {
                    let Some(key) = cap.key else { continue };
                    assert!(!seen.contains(&key), "{key:?} 在布局里出现了两次");
                    seen.push(key);
                }
            }
        }
    }

    /// 修饰键称呼随平台变化：macOS 用 Command / Option。
    #[test]
    fn modifier_labels_are_platform_specific() {
        let mac = cfg!(target_os = "macos");
        assert_eq!(Modifier::Cmd.label(), if mac { "Command" } else { "Win" });
        assert_eq!(Modifier::Alt.label(), if mac { "Option" } else { "Alt" });
        assert_eq!(Modifier::Shift.label(), "Shift");
    }

    /// 国际标准键宽：Space 6.25u、Enter 2.25u、Tab 1.5u、Caps 1.75u。
    #[test]
    fn widths_follow_ansi_standard() {
        let find = |key: Key| {
            ansi_layout()
                .iter()
                .flat_map(|r| r.segments.iter())
                .flat_map(|s| s.caps.iter())
                .find(|c| c.key == Some(key))
                .map(|c| c.width)
        };
        assert!(
            (find(Key::Space).unwrap() - 6.25).abs() < 1e-6,
            "Space 6.25u"
        );
        assert!(
            (find(Key::Enter).unwrap() - 2.25).abs() < 1e-6,
            "Enter 2.25u"
        );
        assert!((find(Key::Tab).unwrap() - 1.5).abs() < 1e-6, "Tab 1.5u");
        let caps_w = ansi_layout()
            .iter()
            .flat_map(|r| r.segments.iter())
            .flat_map(|s| s.caps.iter())
            .find(|c| c.label == "Caps")
            .map(|c| c.width)
            .unwrap();
        assert!((caps_w - 1.75).abs() < 1e-6, "Caps 1.75u");
    }

    /// Z 行必须在 Space 行**之前**（user reported：Z 跑到了 Space 下面）。
    #[test]
    fn letter_row_comes_before_space_row() {
        let pos = |key: Key| {
            ansi_layout()
                .iter()
                .position(|r| {
                    r.segments
                        .iter()
                        .flat_map(|s| s.caps.iter())
                        .any(|c| c.key == Some(key))
                })
                .unwrap()
        };
        assert!(pos(Key::Z) < pos(Key::Space), "Z 行必须在底行（Space）之前");
    }

    /// 方向键必须与导航区同列（在同一区块里，不挂在主键区右侧）。
    #[test]
    fn arrow_keys_are_a_separate_block() {
        for row in ansi_layout() {
            let has_arrow = row
                .segments
                .iter()
                .any(|s| s.caps.iter().any(|c| c.key == Some(Key::ArrowUp)));
            if has_arrow {
                // 方向键所在区块不能同时包含字母键。
                let seg = row
                    .segments
                    .iter()
                    .find(|s| s.caps.iter().any(|c| c.key == Some(Key::ArrowUp)))
                    .unwrap();
                assert!(
                    !seg.caps.iter().any(|c| c.key == Some(Key::Q)),
                    "方向键不能和主键区混在一个区块里"
                );
            }
        }
    }
}
