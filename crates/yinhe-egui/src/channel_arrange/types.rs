//! 通道 AR 的交互产物类型与几何辅助。

use eframe::egui;

use super::groups::{CaChannel, CaGroups};
use super::rows::{CaRow, CaRowLayout};

/// 通道 AR 拖动产物：(tick delta, 轨道 delta, alt 复制标志)。
pub(crate) type CaArRangeDelta = (i64, i32, bool);

/// 通道 AR 的 ghost 音符：(start_tick, end_tick, key, 行 id)。
pub(crate) type CaGhostNote = (u32, u32, u8, u16);

/// 屏幕局部 y（未含 rect.min.y）→ 行号。
pub(crate) fn row_at_local_y(layout: &CaRowLayout, scroll_y: f32, local_y: f32) -> Option<usize> {
    layout.hit_row(local_y + scroll_y)
}

/// 拖动锚定轨道：行 → 用哪个轨道计算垂直位移。
///
/// - 轨道行 / 自动化行 → 该轨道；
/// - 通道合并行 → 该通道首轨（整通道跟随）；
/// - Conductor / 越界 → None（垂直位移视为 0）。
pub(crate) fn anchor_track(layout: &CaRowLayout, groups: &CaGroups, row: usize) -> Option<u16> {
    match layout.row(row)? {
        CaRow::Track(t) => Some(t),
        CaRow::Automation { track, .. } => Some(track),
        CaRow::Channel(ch) => groups.group(ch).and_then(|g| g.tracks.first().copied()),
        CaRow::Conductor => None,
    }
}

/// 一行是否自动化子行（选框/橡皮擦让位给 AM 交互）。
pub(crate) fn is_am_row(layout: &CaRowLayout, row: usize) -> bool {
    matches!(layout.row(row), Some(CaRow::Automation { .. }))
}

/// 一帧行布局哈希（GPU 缓存键分量）：行种类 + 行高。
pub(crate) fn layout_hash(layout: &CaRowLayout) -> u64 {
    let mut h: u64 = 0;
    for i in 0..layout.total_rows() {
        let row_code: u64 = match layout.row(i) {
            Some(CaRow::Conductor) => 1,
            Some(CaRow::Channel(CaChannel::Midi(ch))) => 2 + u64::from(ch),
            Some(CaRow::Channel(CaChannel::Audio(ch))) => 1000 + u64::from(ch),
            Some(CaRow::Track(t)) => 10_000 + u64::from(t),
            Some(CaRow::Automation { track, sub }) => 100_000 + u64::from(track) * 16 + sub as u64,
            None => 0,
        };
        h = h.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(row_code);
        h = h
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .wrapping_add(u64::from(layout.height(i).to_bits()));
    }
    h
}

/// 每行颜色表（GPU `track_colors` 语义：index = 行 id）。
pub(crate) fn row_colors(
    layout: &CaRowLayout,
    groups: &CaGroups,
    track_colors: &[[f32; 4]],
    conductor_color: [f32; 4],
) -> Vec<[f32; 4]> {
    (0..layout.total_rows())
        .map(|i| match layout.row(i) {
            Some(CaRow::Conductor) => conductor_color,
            Some(CaRow::Channel(ch)) => groups
                .group(ch)
                .map(|g| g.color(track_colors))
                .unwrap_or(yinhe_core::DEFAULT_TRACK_COLOR),
            Some(CaRow::Track(t)) | Some(CaRow::Automation { track: t, .. }) => track_colors
                .get(t as usize)
                .copied()
                .unwrap_or(yinhe_core::DEFAULT_TRACK_COLOR),
            None => [0.5, 0.5, 0.5, 1.0],
        })
        .collect()
}

/// 选框在视口局部坐标下的 y 范围（`top..bottom`）。
///
/// 轨道集合 → 覆盖的行范围 → 行 y 前缀和；找不到任何行（如全被折叠隐藏）→ None。
pub(crate) fn sel_view_y(
    layout: &CaRowLayout,
    tracks: &[u16],
    scroll_y: f32,
) -> Option<(f32, f32)> {
    let (lo, hi) = layout.rows_covering_tracks(tracks)?;
    let top = layout.y_top(lo) - scroll_y;
    let bottom = layout.y_top(hi - 1) + layout.height(hi - 1) - scroll_y;
    Some((top, bottom))
}

/// 命中测试：点（视口局部坐标）是否在任一条通道 AR 选框内。
pub(crate) fn point_in_sel_rects(
    sels: &[yinhe_editor_core::edit_state::ChanSelRect],
    layout: &CaRowLayout,
    view: &yinhe_types::ArrangementView,
    local: egui::Pos2,
) -> bool {
    let scroll_y = view.base.scroll_y;
    sels.iter().any(|s| {
        let Some((top, bottom)) = sel_view_y(layout, &s.tracks, scroll_y) else {
            return false;
        };
        let x0 = view.tick_to_x(s.t_start);
        let x1 = view.tick_to_x(s.t_end);
        let rect = egui::Rect::from_min_max(
            egui::pos2(x0.min(x1), top.min(bottom)),
            egui::pos2(x0.max(x1), top.max(bottom)),
        );
        rect.contains(local)
    })
}

/// 通道 AR 的 `CaRowLayout` 作为音频片段交互的行几何。
impl crate::arrange::view_ui::audio_edit::AudioRowGeom for CaRowLayout {
    fn track_at_music_y(&self, music_y: f32) -> Option<u16> {
        let row = self.hit_row(music_y)?;
        match self.row(row) {
            Some(CaRow::Track(t)) => Some(t),
            _ => None,
        }
    }

    fn track_row_yh(&self, track: u16) -> Option<(f32, f32)> {
        let row = self.track_row_of(track)?;
        Some((self.y_top(row), self.height(row)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_core::{TrackData, YinModel};

    fn midi_track(port: u8, channel: u8) -> TrackData {
        TrackData::new(port, channel)
    }

    fn model_with(tracks: Vec<TrackData>) -> YinModel {
        let mut model = YinModel::new_empty_with_16_tracks();
        let mut all = vec![model.tracks[0].as_ref().clone()];
        all.extend(tracks);
        model.tracks = all.into_iter().map(std::sync::Arc::new).collect();
        model
    }

    /// 锚定轨道：轨道行/自动化行取自身；通道行取首轨；Conductor 为 None。
    #[test]
    fn anchor_track_resolves_rows() {
        let model = model_with(vec![midi_track(0, 0), midi_track(0, 0)]);
        let groups = CaGroups::from_model(&model, Some(0));
        let layout = CaRowLayout::build(true, &groups, |_| true, |_| 1, 40.0);
        // 行：0=Conductor, 1=A01(窄) , 2=Track1, 3=Auto(1,0), 4=Track2, 5=Auto(2,0)
        assert_eq!(anchor_track(&layout, &groups, 0), None);
        assert_eq!(anchor_track(&layout, &groups, 1), Some(1));
        assert_eq!(anchor_track(&layout, &groups, 2), Some(1));
        assert_eq!(anchor_track(&layout, &groups, 3), Some(1));
        assert_eq!(anchor_track(&layout, &groups, 4), Some(2));
        assert!(is_am_row(&layout, 3));
        assert!(!is_am_row(&layout, 2));
    }

    /// 行颜色表：Conductor 主题色、通道首轨色、轨道行自身色。
    #[test]
    fn row_colors_by_row_kind() {
        let model = model_with(vec![midi_track(0, 0), midi_track(0, 0)]);
        let groups = CaGroups::from_model(&model, Some(0));
        let layout = CaRowLayout::build(true, &groups, |_| false, |_| 0, 40.0);
        let colors = vec![
            [1.0, 0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0, 1.0],
            [0.0, 0.0, 1.0, 1.0],
        ];
        let conductor = [0.9, 0.9, 0.9, 1.0];
        let rows = row_colors(&layout, &groups, &colors, conductor);
        assert_eq!(rows[0], conductor);
        // 通道行 = 首轨（轨道 1）色。
        assert_eq!(rows[1], [0.0, 1.0, 0.0, 1.0]);
    }

    /// 选框 y 范围覆盖所选轨道之间的全部行（含自动化行）。
    #[test]
    fn sel_view_y_covers_rows_between_tracks() {
        let model = model_with(vec![midi_track(0, 0), midi_track(0, 0)]);
        let groups = CaGroups::from_model(&model, Some(0));
        let layout = CaRowLayout::build(true, &groups, |_| true, |_| 1, 40.0);
        // 行 2=Track1(y60,h40), 3=Auto(1,0)(y100), 4=Track2(y140) → 覆盖 60..180。
        let (top, bottom) = sel_view_y(&layout, &[1, 2], 0.0).unwrap();
        assert_eq!((top, bottom), (60.0, 180.0));
        // scroll 平移进入局部坐标。
        let (top, bottom) = sel_view_y(&layout, &[1], 10.0).unwrap();
        assert_eq!((top, bottom), (50.0, 90.0));
        assert_eq!(sel_view_y(&layout, &[9], 0.0), None);
    }

    /// 点选框命中：tick × 行覆盖范围内为 true。
    #[test]
    fn point_in_sel_rects_hits_covered_area() {
        use yinhe_editor_core::edit_state::ChanSelRect;
        let model = model_with(vec![midi_track(0, 0), midi_track(0, 0)]);
        let groups = CaGroups::from_model(&model, Some(0));
        let layout = CaRowLayout::build(true, &groups, |_| true, |_| 1, 40.0);
        let mut view = yinhe_types::ArrangementView::default();
        view.base.pixels_per_tick = 1.0;
        view.base.scroll_y = 0.0;
        let sels = vec![ChanSelRect {
            t_start: 100.0,
            t_end: 200.0,
            tracks: vec![1, 2],
        }];
        assert!(point_in_sel_rects(
            &sels,
            &layout,
            &view,
            egui::pos2(150.0, 80.0)
        ));
        assert!(!point_in_sel_rects(
            &sels,
            &layout,
            &view,
            egui::pos2(150.0, 20.0)
        ));
        assert!(!point_in_sel_rects(
            &sels,
            &layout,
            &view,
            egui::pos2(50.0, 80.0)
        ));
    }

    /// 布局哈希随展开状态/行高变化。
    #[test]
    fn layout_hash_changes_with_layout() {
        let model = model_with(vec![midi_track(0, 0), midi_track(0, 2)]);
        let groups = CaGroups::from_model(&model, Some(0));
        let a = CaRowLayout::build(true, &groups, |_| false, |_| 0, 40.0);
        let b = CaRowLayout::build(true, &groups, |ch| ch == CaChannel::Midi(0), |_| 0, 40.0);
        let c = CaRowLayout::build(true, &groups, |_| false, |_| 0, 48.0);
        assert_ne!(layout_hash(&a), layout_hash(&b));
        assert_ne!(layout_hash(&a), layout_hash(&c));
        assert_eq!(layout_hash(&a), layout_hash(&a));
    }
}
