use std::collections::{HashMap, HashSet};

use yinhe_types::MAX_KEY;

use crate::config::ProjectSfConfig;
use crate::document::TrackOverride;
use crate::history::PendingEdits;
use crate::playback::PlaybackState;
use crate::quantize::QuantizePreset;

/// Selection rectangle state. Single source of truth for the visual selection boxes.
/// Replaces scattered egui persisted data (sel_rect_persist, sel_drag_origin, last_delta).
///
/// 支持多选框：shift+框选时不清空已有选框，而是 append。
/// 拖拽时所有选框一起偏移。
#[derive(Clone, Default)]
pub struct SelRectState {
    /// Committed selection rects: (t_start, t_end, key_lo, key_hi).
    /// 多选框时按 shift+框选顺序累加。
    pub rects: Vec<(f64, f64, u8, u8)>,
    /// 与`rects`平行的标记：该选框是否为空区域框选自动生成的垂直选框
    /// （普通 Select 工具框选无音符区域时自动切换为全键 0..MAX_KEY）。
    /// 这类选框拖动时锁定上下移动（保持全键语义）；用户手动框选出的
    /// 全键选框不受影响，仍可上下移动。
    pub auto_vertical: Vec<bool>,
    /// Saved rects at drag start; never modified during drag.
    drag_origins: Vec<(f64, f64, u8, u8)>,
    /// Current drag delta in (tick, key) units.
    drag_delta: Option<(i64, i32)>,
    /// Saved rects at resize start; never modified during resize.
    resize_origins: Vec<(f64, f64, u8, u8)>,
    /// Active resize side (Left/Right); None when not resizing.
    resize_side: Option<ResizeSide>,
    /// Current resize delta in tick units.
    resize_dt: Option<i64>,
}

/// Which edge of the selection rect is being dragged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeSide {
    Left,
    Right,
}

impl SelRectState {
    fn offset_rect(rect: (f64, f64, u8, u8), dt: i64, dk: i32) -> (f64, f64, u8, u8) {
        let (t0, t1, kl, kh) = rect;
        (
            t0 + dt as f64,
            t1 + dt as f64,
            (kl as i32 + dk).clamp(0, MAX_KEY as i32) as u8,
            (kh as i32 + dk).clamp(0, MAX_KEY as i32) as u8,
        )
    }

    /// Apply a resize delta to a single rect. Left side changes t_start,
    /// Right side changes t_end. Ensures t_end > t_start (min width 1 tick).
    fn resize_rect(rect: (f64, f64, u8, u8), side: ResizeSide, dt: i64) -> (f64, f64, u8, u8) {
        let (t0, t1, kl, kh) = rect;
        match side {
            ResizeSide::Left => {
                let new_t0 = (t0 + dt as f64).max(0.0).min(t1 - 1.0);
                (new_t0, t1, kl, kh)
            }
            ResizeSide::Right => {
                let new_t1 = (t1 + dt as f64).max(t0 + 1.0);
                (t0, new_t1, kl, kh)
            }
        }
    }

    /// Returns the effective selection rects:
    /// - During drag: drag_origins + drag_delta
    /// - During resize: resize_origins + resize_side + resize_dt
    /// - Otherwise: rects
    pub fn effective_rects(&self) -> Vec<(f64, f64, u8, u8)> {
        if let Some((dt, dk)) = self.drag_delta {
            self.drag_origins
                .iter()
                .map(|&r| Self::offset_rect(r, dt, dk))
                .collect()
        } else if let (Some(side), Some(dt)) = (self.resize_side, self.resize_dt) {
            self.resize_origins
                .iter()
                .map(|&r| Self::resize_rect(r, side, dt))
                .collect()
        } else {
            self.rects.clone()
        }
    }

    /// 是否正在 resize。
    pub fn is_resizing(&self) -> bool {
        self.resize_side.is_some()
    }

    /// 是否没有任何选框。
    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }

    /// 清空所有选框。
    pub fn clear(&mut self) {
        self.rects.clear();
        self.auto_vertical.clear();
    }

    /// 添加一个选框。`auto_vertical=true` 表示空区域框选自动生成的垂直选框（拖动时锁定上下移动）。
    pub fn push_rect(&mut self, rect: (f64, f64, u8, u8), auto_vertical: bool) {
        self.rects.push(rect);
        self.auto_vertical.push(auto_vertical);
    }

    /// 选区中是否包含自动生成的垂直选框（拖动时锁定上下移动）。
    pub fn has_auto_vertical(&self) -> bool {
        self.auto_vertical.iter().any(|&b| b)
    }

    /// Begin dragging: save current rects as origins, clear delta.
    pub fn start_drag(&mut self) {
        self.drag_origins = self.rects.clone();
        self.drag_delta = None;
    }

    /// Update drag delta.
    pub fn update_drag(&mut self, dt: i64, dk: i32) {
        self.drag_delta = Some((dt, dk));
    }

    /// End drag: commit origins + delta to rects, clear drag state.
    pub fn end_drag(&mut self) {
        if let Some((dt, dk)) = self.drag_delta {
            self.rects = self
                .drag_origins
                .iter()
                .map(|&r| Self::offset_rect(r, dt, dk))
                .collect();
        }
        self.drag_origins.clear();
        self.drag_delta = None;
    }

    /// Cancel drag without committing.
    pub fn cancel_drag(&mut self) {
        self.drag_origins.clear();
        self.drag_delta = None;
    }

    /// Begin resizing: save current rects as origins, clear resize delta.
    pub fn start_resize(&mut self, side: ResizeSide) {
        self.resize_origins = self.rects.clone();
        self.resize_side = Some(side);
        self.resize_dt = None;
    }

    /// Update resize delta.
    pub fn update_resize(&mut self, dt: i64) {
        self.resize_dt = Some(dt);
    }

    /// End resize: commit origins + dt to rects, clear resize state.
    pub fn end_resize(&mut self) {
        if let (Some(side), Some(dt)) = (self.resize_side, self.resize_dt) {
            self.rects = self
                .resize_origins
                .iter()
                .map(|&r| Self::resize_rect(r, side, dt))
                .collect();
        }
        self.resize_origins.clear();
        self.resize_side = None;
        self.resize_dt = None;
    }

    /// Cancel resize without committing.
    pub fn cancel_resize(&mut self) {
        self.resize_origins.clear();
        self.resize_side = None;
        self.resize_dt = None;
    }
}

/// Transient editing state. Not persisted to disk.
/// Selection (`selected` and `sel_rect`) is captured in undo snapshots; most
/// other fields are not. Preserved across document switches (zoom/scroll live
/// in App, not here).
pub struct EditState {
    pub selected: yinhe_core::Selection,
    pub track_selected: HashSet<u16>,
    pub cursor_tick: Option<f64>,
    pub quantize_arrange: QuantizePreset,
    pub quantize_pianoroll: QuantizePreset,
    /// 「允许新重叠音符」运行时副本（全局持久化在 AudioSettings，
    /// 新建文档/切换开关时由 app 同步进来；undo 快照不含它）。
    pub allow_overlapping_notes: bool,
    /// 重叠关闭时的移动策略（全局持久化在 AudioSettings）。
    pub overlap_blocked_behavior: crate::audio_settings::OverlapBlockedBehavior,
    pub playback: PlaybackState,
    pub track_overrides: Vec<TrackOverride>,
    pub track_visible: Vec<bool>,
    /// PR 专用显示开关（PR 控制栏右侧勾选）。与 track_visible 分离：
    /// AR 显隐用 track_visible，PR 栏不影响 AR。
    pub track_pianoroll_visible: Vec<bool>,
    pub controller_panels: Vec<yinhe_types::AutomationPanelView>,
    pub show_controller_panels: bool,
    pub soundfont_selected_port: u8,
    pub project_sf: ProjectSfConfig,
    pub pending_edits: PendingEdits,
    /// Per-track display colors (RGBA, computed once at load time).
    pub track_colors_cache: Vec<[f32; 4]>,
    /// Cached track metadata (recomputed from midi + track_names).
    pub track_info_cache: Vec<yinhe_core::TrackInfo>,
    /// Cached first ProgramChange per channel.
    pub pc_map_cache: HashMap<u8, u8>,
    /// Index of the conductor track, if detected.
    pub conductor_track_idx: Option<u16>,
    /// 仅安卓端使用；桌面端已改用 main_track()/write_track() 派生，
    /// 不再读取本字段。track_ops.rs 中对它的维护保留（供安卓端使用）。
    pub editing_track: Option<u16>,
    /// Selection rectangle state.
    pub sel_rect: SelRectState,
    /// AR 选框（钢琴卷帘/自动化的选框在 `sel_rect`/`anchor_sel_rects`）。
    /// 与 sel_rect 一样参与 undo 快照，随文档切换。
    pub arr_sel_rect: Vec<(f64, f64, usize, usize)>,
    /// 各音轨最近一次 velocity 修改（(start_tick, velocity)）。
    /// 新建音符默认力度取此值（同轨多音符修改时记时间最晚的），无记录回退 100。
    /// UI 状态，不参与 undo 快照，不持久化。
    pub recent_velocity: Vec<Option<(u32, u8)>>,
    /// 各音轨最近一次 gate 修改（(start_tick, gate)）。
    /// 新建音符默认长度取此值（同轨多音符修改时记时间最晚的），无记录回退 `fallback`（通常为量化间隔）。
    /// UI 状态，不参与 undo 快照，不持久化。
    pub recent_gate: Vec<Option<(u32, u32)>>,
    /// AR：各音轨自动化 lane 是否展开（按音轨位置，与 track_visible 同语义）。
    pub arr_am_expanded: Vec<bool>,
    /// AR：每条展开的自动化 lane 的视图状态（锚点选框等），
    /// key = (音轨索引, lane target)。base/panel_height/y_offset 每帧由
    /// AR 视图同步覆盖，只有 anchor_sel_rects 等持久状态有效。
    pub arr_am_views:
        HashMap<(u16, yinhe_types::AutomationTarget), yinhe_types::AutomationPanelView>,
    /// AR：被选中的 AM lane（(音轨索引, target)）。选中 AM lane 时
    /// 高亮其子行；卷帘（PR）显示该轨主音轨的音符。
    /// 与 track_selected 互斥（点主行清空本集合，点子行清空 track_selected）。
    pub arr_am_selected: HashSet<(u16, yinhe_types::AutomationTarget)>,
    /// AR：每条自动化 lane 的 M/S 试听状态（Mute/Solo 某个效果来试听）。
    /// 纯试听状态：不进 undo、不持久化；发给音频引擎做自动化旁通。
    pub arr_am_ms: HashMap<(u16, yinhe_types::AutomationTarget), yinhe_types::AmMsState>,
}

impl Default for EditState {
    fn default() -> Self {
        Self {
            selected: yinhe_core::Selection::default(),
            track_selected: HashSet::new(),
            cursor_tick: Some(0.0),
            quantize_arrange: QuantizePreset::Fraction(1, 4),
            quantize_pianoroll: QuantizePreset::Fraction(1, 16),
            allow_overlapping_notes: true,
            overlap_blocked_behavior: crate::audio_settings::OverlapBlockedBehavior::default(),
            playback: PlaybackState::default(),
            track_overrides: vec![TrackOverride::default()],
            track_visible: Vec::new(),
            track_pianoroll_visible: Vec::new(),
            controller_panels: vec![yinhe_types::AutomationPanelView::default()],
            show_controller_panels: true,
            soundfont_selected_port: 0,
            project_sf: ProjectSfConfig::default(),
            pending_edits: PendingEdits::default(),
            track_colors_cache: Vec::new(),
            track_info_cache: Vec::new(),
            pc_map_cache: HashMap::new(),
            conductor_track_idx: None,
            editing_track: None,
            sel_rect: SelRectState::default(),
            arr_sel_rect: Vec::new(),
            recent_velocity: Vec::new(),
            recent_gate: Vec::new(),
            arr_am_expanded: Vec::new(),
            arr_am_views: HashMap::new(),
            arr_am_selected: HashSet::new(),
            arr_am_ms: HashMap::new(),
        }
    }
}

impl EditState {
    /// 主音轨 = 所有选中音轨中索引最小者。
    /// 「编辑目标轨道」概念已删除（桌面端），写入目标由选中集合派生；
    /// Conductor 被选中时主音轨 = Conductor（Tempo 编辑用）。
    pub fn main_track(&self) -> Option<u16> {
        self.track_selected.iter().min().copied()
    }

    /// 写入目标轨（铅笔/双击/录音/步进/自动化）= 主音轨；
    /// 无选中时回退到第一个非 Conductor 轨道，绝不回退到 Conductor
    /// （极端情况：全部轨道都是 Conductor 时返回 None）。
    pub fn write_track(&self) -> Option<u16> {
        self.main_track().or_else(|| {
            (0..self.track_visible.len() as u16).find(|&i| Some(i) != self.conductor_track_idx)
        })
    }

    /// 新建音符的默认力度：该音轨最近一次 velocity 修改值，无记录时 100。
    pub fn default_velocity(&self, track: u16) -> u8 {
        self.recent_velocity
            .get(track as usize)
            .and_then(|v| *v)
            .map(|(_, v)| v)
            .unwrap_or(100)
    }

    /// 记录一次 velocity 修改。同一音轨多次修改时只保留 start_tick 最晚的
    /// （一笔批量修改多个音符 = 记录时间最晚那个音符的力度）。
    pub fn remember_velocity(&mut self, track: u16, start_tick: u32, velocity: u8) {
        let i = track as usize;
        if self.recent_velocity.len() <= i {
            self.recent_velocity.resize(i + 1, None);
        }
        let slot = &mut self.recent_velocity[i];
        if slot.is_none_or(|(t, _)| start_tick >= t) {
            *slot = Some((start_tick, velocity));
        }
    }

    /// 新建音符的默认长度：该音轨最近一次 gate 修改值，无记录时 `fallback`（调用方通常传量化间隔）。
    pub fn default_gate(&self, track: u16, fallback: u32) -> u32 {
        self.recent_gate
            .get(track as usize)
            .and_then(|v| *v)
            .map(|(_, g)| g)
            .unwrap_or(fallback)
    }

    /// 记录一次 gate 修改。同一音轨多次修改时只保留 start_tick 最晚的
    /// （一笔批量修改多个音符 = 记录时间最晚那个音符的长度）。
    pub fn remember_gate(&mut self, track: u16, start_tick: u32, gate: u32) {
        let i = track as usize;
        if self.recent_gate.len() <= i {
            self.recent_gate.resize(i + 1, None);
        }
        let slot = &mut self.recent_gate[i];
        if slot.is_none_or(|(t, _)| start_tick >= t) {
            *slot = Some((start_tick, gate));
        }
    }

    /// 音轨结构变化（增/删/移轨，含 undo/redo）后重映射所有 `(track_idx, target)` 键
    /// （`arr_am_ms` / `arr_am_views` / `arr_am_selected`）：防止 M/S 试听、lane 选中
    /// 状态残留指向错误的轨道/lane。`remap(track) -> None` 表示该轨已删除（键丢弃）。
    pub fn remap_am_track_keys(&mut self, remap: impl Fn(u16) -> Option<u16> + Copy) {
        let remap_entry =
            |(t, target): (u16, yinhe_types::AutomationTarget)| remap(t).map(|nt| (nt, target));
        self.arr_am_ms = std::mem::take(&mut self.arr_am_ms)
            .into_iter()
            .filter_map(|(k, v)| remap_entry(k).map(|nk| (nk, v)))
            .collect();
        self.arr_am_views = std::mem::take(&mut self.arr_am_views)
            .into_iter()
            .filter_map(|(k, v)| remap_entry(k).map(|nk| (nk, v)))
            .collect();
        self.arr_am_selected = std::mem::take(&mut self.arr_am_selected)
            .into_iter()
            .filter_map(remap_entry)
            .collect();
    }

    /// 音符选框整体 tick 平移（selected + sel_rect + arr_sel_rect）。
    /// 用于 tick 加减/复制的非拖拽编辑（拖拽类由 UI 拖拽状态机负责，不要调用）。
    pub fn offset_sel_ticks(&mut self, dt: i64) {
        self.selected.offset_ticks(dt);
        for r in &mut self.sel_rect.rects {
            r.0 += dt as f64;
            r.1 += dt as f64;
        }
        for r in &mut self.arr_sel_rect {
            r.0 += dt as f64;
            r.1 += dt as f64;
        }
    }

    /// 音符选框整体 key 平移（selected + sel_rect；AR 选框无 key 概念）。
    pub fn offset_sel_keys(&mut self, dk: i32) {
        self.selected.offset(0, dk);
        // 先算好平移后是否仍为全键（0..MAX_KEY），再同步解除失效的自动垂直标记：
        // 自动垂直选框移出全键范围后不再有"全键"语义，拖动锁定随之解除。
        let still_vertical: Vec<bool> = self
            .sel_rect
            .rects
            .iter()
            .zip(&self.sel_rect.auto_vertical)
            .map(|(r, &auto)| {
                let kl = (r.2 as i32 + dk).clamp(0, MAX_KEY as i32) as u8;
                let kh = (r.3 as i32 + dk).clamp(0, MAX_KEY as i32) as u8;
                auto && kl == 0 && kh == MAX_KEY
            })
            .collect();
        for r in &mut self.sel_rect.rects {
            r.2 = (r.2 as i32 + dk).clamp(0, MAX_KEY as i32) as u8;
            r.3 = (r.3 as i32 + dk).clamp(0, MAX_KEY as i32) as u8;
        }
        self.sel_rect.auto_vertical = still_vertical;
    }

    /// 音符选框 tick 终点统一平移（gate 加减用，起点不动）。保证 te > ts。
    pub fn offset_sel_te(&mut self, dt: i64) {
        for r in &mut self.sel_rect.rects {
            r.1 = (r.1 + dt as f64).max(r.0 + 1.0);
        }
        for r in &mut self.arr_sel_rect {
            r.1 = (r.1 + dt as f64).max(r.0 + 1.0);
        }
        for r in &mut self.selected.rects {
            let new_te = (r.1 as i64 + dt).max(r.0 as i64 + 1) as u32;
            r.1 = new_te;
        }
    }

    /// 音符选框 tick 范围相对 `t0` 等比缩放（变速用，key/track 不动）。
    pub fn scale_sel_ticks(&mut self, t0: u64, factor: f64) {
        let scale = |v: u64| -> u64 {
            let s = (t0 as f64 + (v as f64 - t0 as f64) * factor)
                .round()
                .max(t0 as f64);
            if s > u32::MAX as f64 {
                u32::MAX as u64
            } else {
                s as u64
            }
        };
        let scale_rect = |ts: &mut u64, te: &mut u64| {
            let nts = scale(*ts);
            let nte = scale(*te).max(nts + 1);
            *ts = nts;
            *te = nte;
        };
        for r in &mut self.selected.rects {
            let mut ts = r.0 as u64;
            let mut te = r.1 as u64;
            scale_rect(&mut ts, &mut te);
            r.0 = ts as u32;
            r.1 = te as u32;
        }
        for r in &mut self.sel_rect.rects {
            let mut ts = r.0 as u64;
            let mut te = r.1 as u64;
            scale_rect(&mut ts, &mut te);
            r.0 = ts as f64;
            r.1 = te as f64;
        }
        for r in &mut self.arr_sel_rect {
            let mut ts = r.0 as u64;
            let mut te = r.1 as u64;
            scale_rect(&mut ts, &mut te);
            r.0 = ts as f64;
            r.1 = te as f64;
        }
    }

    /// AM 选框（指定面板）tick 范围平移。
    pub fn offset_anchor_ticks(&mut self, panel_idx: usize, dt: i64) {
        if let Some(panel) = self.controller_panels.get_mut(panel_idx) {
            for r in &mut panel.anchor_sel_rects {
                r.tick_start += dt as f64;
                r.tick_end += dt as f64;
            }
        }
    }

    /// AM 选框（指定面板）value 范围平移（value_range 为 None 的垂直全选跳过）。
    pub fn offset_anchor_values(&mut self, panel_idx: usize, dv: f32) {
        if let Some(panel) = self.controller_panels.get_mut(panel_idx) {
            for r in &mut panel.anchor_sel_rects {
                if let Some((lo, hi)) = &mut r.value_range {
                    *lo += dv;
                    *hi += dv;
                }
            }
        }
    }

    /// AM 选框（指定面板）tick 范围相对 `t0` 等比缩放（value_range 不动）。
    pub fn scale_anchor_ticks(&mut self, panel_idx: usize, t0: f64, factor: f64) {
        if let Some(panel) = self.controller_panels.get_mut(panel_idx) {
            for r in &mut panel.anchor_sel_rects {
                let ts = r.tick_start.min(r.tick_end);
                let te = r.tick_start.max(r.tick_end);
                let nts = (t0 + (ts - t0) * factor).round();
                let nte = (t0 + (te - t0) * factor).round().max(nts + 1.0);
                r.tick_start = nts;
                r.tick_end = nte;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_velocity_falls_back_to_100() {
        let state = EditState::default();
        assert_eq!(state.default_velocity(0), 100);
        assert_eq!(state.default_velocity(5), 100);
    }

    #[test]
    fn remember_velocity_keeps_latest_tick_per_track() {
        let mut state = EditState::default();
        state.remember_velocity(1, 100, 75);
        assert_eq!(state.default_velocity(1), 75);
        // 更晚的音符覆盖
        state.remember_velocity(1, 300, 60);
        assert_eq!(state.default_velocity(1), 60);
        // 更早的音符不覆盖
        state.remember_velocity(1, 50, 90);
        assert_eq!(state.default_velocity(1), 60);
        // 其他音轨不受影响
        assert_eq!(state.default_velocity(0), 100);
        state.remember_velocity(2, 0, 10);
        assert_eq!(state.default_velocity(2), 10);
        assert_eq!(state.default_velocity(1), 60);
    }

    #[test]
    fn remember_velocity_same_tick_keeps_latest() {
        let mut state = EditState::default();
        state.remember_velocity(0, 100, 70);
        state.remember_velocity(0, 100, 80);
        assert_eq!(state.default_velocity(0), 80);
    }

    #[test]
    fn default_gate_falls_back_to_interval() {
        let state = EditState::default();
        assert_eq!(state.default_gate(0, 120), 120);
        assert_eq!(state.default_gate(5, 480), 480);
    }

    #[test]
    fn remember_gate_keeps_latest_tick_per_track() {
        let mut state = EditState::default();
        state.remember_gate(1, 100, 75);
        assert_eq!(state.default_gate(1, 120), 75);
        // 更晚的音符覆盖
        state.remember_gate(1, 300, 60);
        assert_eq!(state.default_gate(1, 120), 60);
        // 更早的音符不覆盖
        state.remember_gate(1, 50, 90);
        assert_eq!(state.default_gate(1, 120), 60);
        // 其他音轨不受影响
        assert_eq!(state.default_gate(0, 120), 120);
        state.remember_gate(2, 0, 10);
        assert_eq!(state.default_gate(2, 120), 10);
        assert_eq!(state.default_gate(1, 120), 60);
    }

    #[test]
    fn remember_gate_same_tick_keeps_latest() {
        let mut state = EditState::default();
        state.remember_gate(0, 100, 70);
        state.remember_gate(0, 100, 80);
        assert_eq!(state.default_gate(0, 120), 80);
    }
}
