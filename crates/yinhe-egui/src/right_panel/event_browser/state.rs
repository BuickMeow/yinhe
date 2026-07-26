//! 事件浏览器状态与选中项类型。
//!
//! `SelectedItem::Automation` 统一覆盖 CC / PitchBend / RPN / NRPN / Tempo，
//! 通过 `AutomationTarget` 区分具体类型，避免为每种自动化写单独变体。

use yinhe_types::AutomationTarget;

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
    Notes { track: u16 },
    ProgramChange { track: u16 },
    Automation { track: u16, target: AutomationTarget },
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub enum ArchiveKey {
    Conductor,
    Port(u8),
    Channel(u8, u8),
    Track(u16),
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
