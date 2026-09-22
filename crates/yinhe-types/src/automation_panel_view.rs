use crate::{AutomationEvent, AutomationLane, AutomationTarget, NoteBitset, TimelineViewBase};

/// Default panel height in pixels.
pub const DEFAULT_PANEL_HEIGHT: f32 = 80.0;
/// Minimum panel height when dragging.
pub const MIN_PANEL_HEIGHT: f32 = 40.0;
/// Maximum panel height when dragging.
pub const MAX_PANEL_HEIGHT: f32 = 200.0;

/// 持久化的锚点选框（音乐坐标）。
///
/// 框选完成后存储在 `AutomationPanelView::anchor_sel_rect` 中，用于：
/// 1. 持续显示选框（视觉反馈，类似 PR/AR 的 sel_rect）
/// 2. 点击选框内时触发拖拽（而非新框选）
/// 3. 选中状态由锚点是否在此范围内决定
///
/// 存储音乐坐标而非屏幕坐标，这样滚动/缩放后选框位置仍然正确。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AnchorSelRect {
    /// 选框的 tick 范围
    pub tick_start: f64,
    pub tick_end: f64,
    /// 选框的 value 范围。
    /// `None` = 垂直全选（y 范围用整个面板高度，SelectVertical 工具）
    pub value_range: Option<(f32, f32)>,
}

impl AnchorSelRect {
    /// 判断锚点 (tick, value) 是否在选框范围内。
    pub fn contains(&self, tick: u32, value: f32) -> bool {
        let ts = self.tick_start.min(self.tick_end);
        let te = self.tick_start.max(self.tick_end);
        let tick_in = (tick as f64) >= ts && (tick as f64) <= te;
        let value_in = match self.value_range {
            None => true,
            Some((vmin, vmax)) => value >= vmin && value <= vmax,
        };
        tick_in && value_in
    }
}

/// View state for a single automation panel in the controller area below the pianoroll.
#[derive(Clone, Debug)]
pub struct AutomationPanelView {
    /// Shared horizontal timeline state (scroll_x, pixels_per_tick, left_panel_width).
    /// These fields are synced from the pianoroll view each frame.
    pub base: TimelineViewBase,
    /// Current panel height in pixels.
    pub panel_height: f32,
    /// The automation target currently displayed in this panel.
    /// When `show_velocity` is true, this field is ignored for data rendering.
    /// `AutomationTarget::Tempo` 时显示 conductor.tempo lane。
    pub selected_target: AutomationTarget,
    /// When true, render velocity bars from note data instead of automation lanes.
    pub show_velocity: bool,
    /// Cached index into `MidiFile.automation_lanes` for fast lookup.
    pub lane_index: usize,
    /// Whether the panel content needs to be rebuilt.
    pub dirty: bool,
    /// 垂直缩放系数。1.0 = 满量程（0~max_val）映射到面板高度。
    /// > 1.0 = 放大（只显示部分值范围），< 1.0 = 缩小（显示更宽范围）。
    pub value_zoom: f32,
    /// 垂直滚动偏移（值空间单位，如 CC 的 0~127）。
    /// 面板顶部对应的值 = `value_scroll`。
    pub value_scroll: f32,
    /// 面板内容在宿主纹理中的 y 偏移（像素）。
    /// PR 的独立面板为 0；AR 的自动化 lane 画在共享走带纹理里，
    /// 用此字段把曲线平移到所属子行的顶部。
    pub y_offset: f32,
    /// 持久化的锚点选框列表（音乐坐标）。支持多选框：shift+框选时累加。
    /// 选中状态由锚点是否在任一选框范围内决定（类似 PR/AR 的 sel_rect）。
    /// 框选完成后追加，点击选框外或清空选区时清空全部。
    pub anchor_sel_rects: Vec<AnchorSelRect>,
    /// 锚点成员集（按 `AutomationEvent.id`）。框选/点选提交时物化。
    /// `Some` = 成员态：命中以位图为准，`anchor_sel_rects` 退化为扫描范围/
    /// 选框显示，拖动/复制到落点不会吸收其他锚点；`None` = 矩形态。
    pub anchor_members: Option<NoteBitset>,
    /// 已物化的 `anchor_sel_rects` 前缀长度（语义同 `Selection::materialized_rects`）。
    pub materialized_anchor_rects: usize,
}

impl Default for AutomationPanelView {
    fn default() -> Self {
        Self {
            base: TimelineViewBase {
                pixels_per_tick: 0.15,
                scroll_x: 0.0,
                scroll_y: 0.0,
                left_panel_width: 60.0,
                dirty: true,
                track_panel_row_height: 40.0,
                track_panel_scroll_y: 0.0,
                follow_target: None,
                follow_anim_start: 0.0,
                follow_anim_elapsed: 0.0,
            },
            panel_height: DEFAULT_PANEL_HEIGHT,
            selected_target: AutomationTarget::CC { controller: 7 },
            show_velocity: true,
            lane_index: 0,
            dirty: true,
            value_zoom: 1.0,
            value_scroll: 0.0,
            y_offset: 0.0,
            anchor_sel_rects: Vec::new(),
            anchor_members: None,
            materialized_anchor_rects: 0,
        }
    }
}

impl AutomationPanelView {
    /// Sync horizontal scroll state from the pianoroll view.
    pub fn sync_from_pianoroll(
        &mut self,
        scroll_x: f32,
        pixels_per_tick: f32,
        left_panel_width: f32,
    ) {
        if self.base.scroll_x != scroll_x
            || self.base.pixels_per_tick != pixels_per_tick
            || self.base.left_panel_width != left_panel_width
        {
            self.base.scroll_x = scroll_x;
            self.base.pixels_per_tick = pixels_per_tick;
            self.base.left_panel_width = left_panel_width;
            self.dirty = true;
        }
    }

    /// Convenience: keyboard / left-panel width.
    #[inline]
    pub fn left_panel_width(&self) -> f32 {
        self.base.left_panel_width
    }

    /// Hash of all fields that affect GPU rendering output.
    /// Used as cache key for GPU layers (Layer 0 grid only depends on geometry).
    pub fn render_hash(&self) -> u64 {
        crate::hash::hash_f32s(&[
            self.base.pixels_per_tick,
            self.base.scroll_x,
            self.base.left_panel_width,
            self.panel_height,
            self.value_zoom,
            self.value_scroll,
            self.y_offset,
        ])
    }

    /// 将自动化值转换为宿主纹理 Y 坐标（像素，含 y_offset）。
    /// `max_val` = 当前 target 的最大值（如 CC 的 127）。
    #[inline]
    pub fn value_to_y(&self, value: f32, max_val: f32) -> f32 {
        let visible_range = max_val / self.value_zoom;
        if visible_range <= 0.0 {
            return self.y_offset;
        }
        let h = self.panel_height;
        self.y_offset + h - ((value - self.value_scroll) / visible_range) * h
    }

    /// 将宿主纹理 Y 坐标（像素，含 y_offset）转换回自动化值。
    /// `max_val` = 当前 target 的最大值。
    #[inline]
    pub fn y_to_value(&self, y: f32, max_val: f32) -> f32 {
        let visible_range = max_val / self.value_zoom;
        if visible_range <= 0.0 {
            return 0.0;
        }
        let h = self.panel_height;
        self.value_scroll + (1.0 - (y - self.y_offset) / h) * visible_range
    }

    /// 根据 max_val 限制 value_scroll 的范围，防止滚出有效区间。
    pub fn clamp_value_scroll(&mut self, max_val: f32) {
        let visible_range = max_val / self.value_zoom;
        let max_scroll = (max_val - visible_range).max(0.0);
        self.value_scroll = self.value_scroll.clamp(0.0, max_scroll);
    }

    /// 是否处于锚点成员态（框选/点选物化后）。
    pub fn has_anchor_members(&self) -> bool {
        self.anchor_members.is_some()
    }

    /// 锚点成员数量（矩形态返回 `None`）。
    pub fn anchor_member_count(&self) -> Option<u64> {
        self.anchor_members.as_ref().map(NoteBitset::count)
    }

    /// 把尚未物化的选框（`anchor_sel_rects[materialized_anchor_rects..]`）
    /// 采样为该 lane 的锚点成员（AM 框选/点选提交时调用）。
    /// id=0（未分配）的锚点跳过；属性筛选不在此应用（保持动态谓词）。
    pub fn materialize_anchor_pending(&mut self, lane: &AutomationLane) {
        if self.materialized_anchor_rects >= self.anchor_sel_rects.len() {
            return;
        }
        let pending: Vec<AnchorSelRect> =
            self.anchor_sel_rects[self.materialized_anchor_rects..].to_vec();
        let bits = self.anchor_members.get_or_insert_with(NoteBitset::default);
        for rect in pending {
            // 先按 tick 范围二分缩小扫描窗口（选框是闭区间，右端 +1 变半开）。
            let ts = rect.tick_start.min(rect.tick_end).max(0.0);
            let te = rect.tick_start.max(rect.tick_end).max(0.0);
            let lo = ts as u32;
            let hi = (te.ceil() as u64 + 1).min(u32::MAX as u64) as u32;
            for evt in lane.events_in_range(lo, hi) {
                if evt.id != 0 && rect.contains(evt.tick, evt.value) {
                    bits.insert(evt.id);
                }
            }
        }
        self.materialized_anchor_rects = self.anchor_sel_rects.len();
    }

    /// 锚点是否被选中：成员态查位图，矩形态查选框几何。
    pub fn accepts_anchor(&self, ev: &AutomationEvent) -> bool {
        match &self.anchor_members {
            Some(bits) => bits.contains(ev.id),
            None => self
                .anchor_sel_rects
                .iter()
                .any(|r| r.contains(ev.tick, ev.value)),
        }
    }

    /// 用给定 id 重建锚点成员集（复制/粘贴后让选择跟随副本）。
    pub fn set_anchor_members(&mut self, ids: impl IntoIterator<Item = u32>) {
        let mut bits = NoteBitset::default();
        for id in ids {
            if id != 0 {
                bits.insert(id);
            }
        }
        self.anchor_members = Some(bits);
        self.materialized_anchor_rects = self.anchor_sel_rects.len();
    }

    /// 清空锚点选择（选框 + 成员）。
    pub fn clear_anchor_selection(&mut self) {
        self.anchor_sel_rects.clear();
        self.anchor_members = None;
        self.materialized_anchor_rects = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_values() {
        let view = AutomationPanelView::default();
        assert_eq!(view.panel_height, DEFAULT_PANEL_HEIGHT);
        assert!(view.show_velocity);
        assert_eq!(view.lane_index, 0);
        assert!(view.dirty);
        assert_eq!(view.base.pixels_per_tick, 0.15);
        assert_eq!(view.base.scroll_x, 0.0);
        assert_eq!(view.base.left_panel_width, 60.0);
    }

    #[test]
    fn test_sync_from_pianoroll_updates_values() {
        let mut view = AutomationPanelView {
            dirty: false,
            ..Default::default()
        };

        view.sync_from_pianoroll(100.0, 0.5, 80.0);

        assert_eq!(view.base.scroll_x, 100.0);
        assert_eq!(view.base.pixels_per_tick, 0.5);
        assert_eq!(view.base.left_panel_width, 80.0);
        assert!(view.dirty);
    }

    #[test]
    fn test_sync_from_pianoroll_no_change_skips_dirty() {
        let mut view = AutomationPanelView {
            dirty: false,
            ..Default::default()
        };

        view.sync_from_pianoroll(0.0, 0.15, 60.0);

        assert!(
            !view.dirty,
            "dirty should remain false when values unchanged"
        );
    }

    #[test]
    fn test_sync_from_pianoroll_partial_update_triggers_dirty() {
        let mut view = AutomationPanelView {
            dirty: false,
            ..Default::default()
        };

        // Only change scroll_x
        view.sync_from_pianoroll(50.0, 0.15, 60.0);

        assert!(view.dirty);
        assert_eq!(view.base.scroll_x, 50.0);
        assert_eq!(view.base.pixels_per_tick, 0.15);
        assert_eq!(view.base.left_panel_width, 60.0);
    }

    #[test]
    fn test_left_panel_width_returns_base_value() {
        let view = AutomationPanelView::default();
        assert_eq!(view.left_panel_width(), view.base.left_panel_width);
    }

    #[test]
    fn test_panel_height_constants() {
        assert_eq!(DEFAULT_PANEL_HEIGHT, 80.0);
        assert_eq!(MIN_PANEL_HEIGHT, 40.0);
        assert_eq!(MAX_PANEL_HEIGHT, 200.0);
    }
}
