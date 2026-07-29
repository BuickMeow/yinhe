//! 事件浏览器状态与选中项类型。
//!
//! `SelectedItem::Automation` 统一覆盖 CC / PitchBend / RPN / NRPN / Tempo，
//! 通过 `AutomationTarget` 区分具体类型，避免为每种自动化写单独变体。

use yinhe_types::{AutomationTarget, SegmentShape};

/// 事件浏览器表格行点击时产生的跳转请求。
///
/// 音符/TimeSig 携带 `PulseKind`，App 据此启动闪烁动画；
/// automation 类只跳转不闪烁（`PulseKind = None`）。
#[derive(Clone, Debug)]
pub struct JumpRequest {
    pub tick: u32,
    /// 音符事件：`Some((track, key))`；其他事件：`None`。
    pub note: Option<(u16, u8)>,
    /// 闪烁高亮类型：`None` = 仅跳转不闪烁。
    pub pulse: Option<PulseKind>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PulseKind {
    /// 音符矩形闪烁（piano roll 内画白色描边矩形）。
    NoteRect,
    /// TimeSig 竖线闪烁（贯穿 piano roll 高度的白色竖线）。
    TimesigLine,
}

pub struct EventBrowserState {
    pub expanded_keys: std::collections::HashSet<ArchiveKey>,
    pub selected_item: Option<SelectedItem>,
    pub selected_track: Option<u16>,
    /// 事件列表当前页码（0-based）。切换 selected_item 时重置为 0。
    pub event_page: usize,
    pub(super) fingerprint: Option<u64>,
    pub(super) split_ratio: f32,
}

/// 事件浏览器中选中的条目。
///
/// `Automation` 统一覆盖 CC / PitchBend / RPN / NRPN / Tempo。
/// `track` 对 Tempo 无意义（用 0），其他类型为所属音轨索引。
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum SelectedItem {
    ProjectJson,
    MappingJson,
    TimeSig,
    /// 调号事件（全局，conductor 级）
    KeySig,
    /// 标记事件（全局，conductor 级）
    Markers,
    Notes { track: u16 },
    ProgramChange { track: u16 },
    Automation { track: u16, target: AutomationTarget },
    /// 歌词事件（per-track）
    Lyrics { track: u16 },
    /// 和弦事件（per-track）
    Chord { track: u16 },
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub enum ArchiveKey {
    Conductor,
    Port(u8),
    Channel(u8, u8),
    Track(u16),
}

/// 音符引用：足够定位一个音符的所有字段。
///
/// `id` 用于 `Arc::make_mut` 后的 retain，`start_tick` / `key` / `track` 用于
/// `pencil_drag_note` 寻址。`end_tick` / `velocity` 是当前值，便于 popup 实时显示。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoteRef {
    pub id: u32,
    pub start_tick: u32,
    pub end_tick: u32,
    pub key: u8,
    pub velocity: u8,
    pub track: u16,
}

/// 右键编辑请求：cell 上右键时写入 egui memory，由 `apply_edit_popups` 取出分派。
///
/// 一个全局 key `egui::Id::new((salt, "edit"))` 存 `EditRequest`，**不**用 `ui.id()`
/// （cell 是 child ui，`ui.id()` 与 popup 调用处不同）。
#[derive(Clone, Debug, PartialEq)]
pub enum EditRequest {
    /// Automation 的 tick 编辑（位置移动）
    AutoTick { tick: u32, value: f32 },
    /// Automation 的 value 编辑
    AutoValue { tick: u32, value: f32 },
    /// Automation 的 shape 编辑
    AutoShape { tick: u32, shape: SegmentShape },
    /// 音符 start_tick 编辑（保持 gate 不变，end_tick 跟随平移）
    NoteStartTick { note: NoteRef },
    /// 音符 end_tick 编辑（gate 随之变化）
    NoteEndTick { note: NoteRef },
    /// 音符 gate（长度）编辑（实际改 end_tick = start_tick + gate）
    NoteGate { note: NoteRef },
    /// 音符 key 编辑
    NoteKey { note: NoteRef },
    /// 音符 velocity 编辑
    NoteVelocity { note: NoteRef },
    /// TimeSig 的 tick 编辑（按 tick 寻址，避免 sort 后 idx 失效）
    TimeSigTick { tick: u32 },
    /// TimeSig 的 numerator 编辑
    TimeSigNumerator { tick: u32 },
    /// TimeSig 的 denominator 编辑（2 的幂次：2 = 4, 3 = 8）
    TimeSigDenominator { tick: u32 },
    /// KeySig 的 tick 编辑
    KeySigTick { tick: u32 },
    /// KeySig 的 sf 编辑（升降号数 -7..=7）
    KeySigSf { tick: u32 },
    /// KeySig 的 mi 编辑（0 = 大调，1 = 小调）
    KeySigMi { tick: u32 },
    /// 文本类事件（Marker/Lyrics/Chord）的 tick 编辑
    TextEventTick { kind: TextEventKind, tick: u32 },
    /// 文本类事件的 text 编辑
    TextEventText { kind: TextEventKind, tick: u32 },
}

/// 文本类事件种类：Marker（conductor 级）/ Lyrics/Chord（per-track）。
///
/// 用于 `EditRequest::TextEventTick` / `TextEventText` 区分事件归属，
/// 配合 `apply_text_popups` 分派到对应的 Document 方法。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextEventKind {
    Marker,
    Lyrics { track: u16 },
    Chord { track: u16 },
}

impl Default for EventBrowserState {
    fn default() -> Self {
        Self {
            expanded_keys: Default::default(),
            selected_item: None,
            selected_track: None,
            event_page: 0,
            fingerprint: None,
            split_ratio: 0.45,
        }
    }
}
