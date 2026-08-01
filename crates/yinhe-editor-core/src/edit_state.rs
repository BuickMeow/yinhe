use std::collections::{HashMap, HashSet};

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
            (kl as i32 + dk).clamp(0, 127) as u8,
            (kh as i32 + dk).clamp(0, 127) as u8,
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
    pub playback: PlaybackState,
    pub track_overrides: Vec<TrackOverride>,
    pub track_visible: Vec<bool>,
    pub track_pianoroll_visible: Vec<bool>,
    pub track_pianoroll_visible_snapshot: Option<Vec<bool>>,
    pub controller_panels: Vec<yinhe_types::AutomationPanelView>,
    pub show_controller_panels: bool,
    pub soundfont_selected_port: u8,
    pub project_sf: ProjectSfConfig,
    pub pending_edits: PendingEdits,
    /// Per-track display colors (computed once at load time).
    pub track_colors_cache: Vec<[f32; 3]>,
    /// Cached track metadata (recomputed from midi + track_names).
    pub track_info_cache: Vec<yinhe_core::TrackInfo>,
    /// Cached first ProgramChange per channel.
    pub pc_map_cache: HashMap<u8, u8>,
    /// Index of the conductor track, if detected.
    pub conductor_track_idx: Option<u16>,
    /// 当前被铅笔工具编辑的轨道（双击 track 设置）。
    /// 同时只能有一个；可见且被选择时才允许编辑。
    /// Conductor 也可被设为 editing_track（仅用于 Tempo automation）。
    pub editing_track: Option<u16>,
    /// Selection rectangle state.
    pub sel_rect: SelRectState,
    /// AR 选框（钢琴卷帘/自动化的选框在 `sel_rect`/`anchor_sel_rects`）。
    /// 与 sel_rect 一样参与 undo 快照，随文档切换。
    pub arr_sel_rect: Vec<(f64, f64, usize, usize)>,
}

impl Default for EditState {
    fn default() -> Self {
        Self {
            selected: yinhe_core::Selection::default(),
            track_selected: HashSet::new(),
            cursor_tick: Some(0.0),
            quantize_arrange: QuantizePreset::Fraction(1, 4),
            quantize_pianoroll: QuantizePreset::Fraction(1, 16),
            playback: PlaybackState::default(),
            track_overrides: vec![TrackOverride::default()],
            track_visible: Vec::new(),
            track_pianoroll_visible: Vec::new(),
            track_pianoroll_visible_snapshot: None,
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
        }
    }
}

impl EditState {
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
        for r in &mut self.sel_rect.rects {
            r.2 = (r.2 as i32 + dk).clamp(0, 127) as u8;
            r.3 = (r.3 as i32 + dk).clamp(0, 127) as u8;
        }
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
