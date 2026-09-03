use std::collections::{HashMap, HashSet};

use crate::config::ProjectSfConfig;
use crate::document::TrackOverride;
use crate::history::PendingEdits;
use crate::playback::PlaybackState;
use crate::quantize::QuantizePreset;

pub mod sel_rect;
pub mod selection_ops;
pub mod velocity_gate;

pub use sel_rect::{ResizeSide, SelRectState};

/// 瞬态编辑状态（不落盘）
/// Selection 参与 undo 快照，其余多为 UI 状态
pub struct EditState {
    pub selected: yinhe_core::Selection,
    pub track_selected: HashSet<u16>,
    pub cursor_tick: Option<f64>,
    pub quantize_arrange: QuantizePreset,
    pub quantize_pianoroll: QuantizePreset,
    pub allow_overlapping_notes: bool,
    pub overlap_blocked_behavior: crate::audio_settings::OverlapBlockedBehavior,
    pub playback: PlaybackState,
    pub track_overrides: Vec<TrackOverride>,
    pub track_visible: Vec<bool>,
    pub track_pianoroll_visible: Vec<bool>,
    pub controller_panels: Vec<yinhe_types::AutomationPanelView>,
    pub show_controller_panels: bool,
    pub soundfont_selected_port: u8,
    pub project_sf: ProjectSfConfig,
    pub pending_edits: PendingEdits,
    pub track_colors_cache: Vec<[f32; 4]>,
    pub track_info_cache: Vec<yinhe_core::TrackInfo>,
    pub pc_map_cache: HashMap<u8, u8>,
    pub conductor_track_idx: Option<u16>,
    pub editing_track: Option<u16>,
    pub sel_rect: SelRectState,
    pub arr_sel_rect: Vec<(f64, f64, usize, usize)>,
    pub recent_velocity: Vec<Option<(u32, u8)>>,
    pub recent_gate: Vec<Option<(u32, u32)>>,
    pub arr_am_expanded: Vec<bool>,
    pub arr_am_views:
        HashMap<(u16, yinhe_types::AutomationTarget), yinhe_types::AutomationPanelView>,
    pub arr_am_selected: HashSet<(u16, yinhe_types::AutomationTarget)>,
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
    /// 主音轨 = 选中音轨中索引最小者
    pub fn main_track(&self) -> Option<u16> {
        self.track_selected.iter().min().copied()
    }

    /// 写入目标轨 = 主音轨，无选中时回退到首个非 Conductor 轨
    pub fn write_track(&self) -> Option<u16> {
        self.main_track().or_else(|| {
            (0..self.track_visible.len() as u16).find(|&i| Some(i) != self.conductor_track_idx)
        })
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
        state.remember_velocity(1, 300, 60);
        assert_eq!(state.default_velocity(1), 60);
        state.remember_velocity(1, 50, 90);
        assert_eq!(state.default_velocity(1), 60);
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
        state.remember_gate(1, 300, 60);
        assert_eq!(state.default_gate(1, 120), 60);
        state.remember_gate(1, 50, 90);
        assert_eq!(state.default_gate(1, 120), 60);
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
