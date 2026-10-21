//! UI 无关的 PR 可见音符缓存。
//!
//! 目标：把“可见范围 + 预取窗口 + key 分桶 + LOD + hidden/selection”收成一份
//! 逻辑，供 Flutter / egui 两个壳复用。这里不依赖任何 UI 线程或窗口系统，
//! 只负责回答三件事：
//!
//! 1. 当前应该覆盖哪一段 tick / key（`plan_window`）；
//! 2. 哪些 key 的实例缓存过期，需要重建；
//! 3. 拼好的扁平 `NoteInstance` 列表和上传缓存键。
//!
//! 关键性质：实例里只有 tick/key/track/velocity，**没有滚动像素**。因此
//! 只要 tick 窗口和 key 范围不变，滚动/缩放只改 uniform 即可，不需要重传
//! 音符层。窗口在屏幕外预取半个视口，避免每滚动一像素就重建。

use std::collections::HashSet;

use yinhe_core::Selection;
use yinhe_types::{KEY_COUNT, NoteSource, PianoRollView};

use crate::layer_cache_key;
use crate::pianoroll::{
    SUMMARY_BLOCK_TICKS, build_key_notes_in_range, build_key_summary, select_summary_level,
};
use crate::vertex::NoteInstance;

/// 预取窗口（tick 已量化到视口跨度，滚动在窗口内不会改变它）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoteWindow {
    pub tick_start: u32,
    pub tick_end: u32,
    pub key_lo: u8,
    pub key_hi: u8,
}

/// 规划当前可见窗口。
///
/// `tick` 窗口按“一个可见跨度”对齐，并在两侧各预取半屏；`key` 范围直接用
/// 视图的可见 key 范围（`visible_cross_range` 本身已含 1 键 padding）。
/// 窗口变化只发生在滚动跨过一个可见跨度时，而不是每一像素。
pub fn plan_window(view: &PianoRollView, width: f32, height: f32) -> NoteWindow {
    let (lo, hi) = view.visible_main_range(view.main_axis_len(width, height));
    let span = (hi - lo).max(1.0);
    let block = span;
    let pad = span * 0.5;
    let start = ((lo - pad) / block).floor() * block;
    let end = ((hi + pad) / block).ceil() * block;
    let tick_start = start.max(0.0).min(f64::from(u32::MAX)) as u32;
    let tick_end = end
        .max(f64::from(tick_start) + 1.0)
        .min(f64::from(u32::MAX)) as u32;
    let (key_lo, key_hi) = view.visible_cross_range(view.cross_axis_len(width, height));
    NoteWindow {
        tick_start,
        tick_end,
        key_lo,
        key_hi,
    }
}

/// 一次缓存更新所需的输入。
pub struct NoteCacheParams<'a> {
    pub midi: &'a dyn NoteSource,
    pub view: &'a PianoRollView,
    pub width: f32,
    pub height: f32,
    pub track_visible: &'a [bool],
    pub track_selected: &'a HashSet<u16>,
    pub hidden_notes: &'a HashSet<(u16, u32, u8)>,
    /// `hidden_notes` 的预计算 hash；避免每帧重扫大选区隐藏集合。
    pub hidden_hash: u64,
    /// `None` = 不做选中位/选中排序。
    pub selection: Option<&'a Selection>,
    pub note_revisions: &'a [u64; KEY_COUNT],
    /// 模型身份：换文档时必须变化，避免新旧模型 revision 恰好相同导致复用旧实例。
    /// Flutter 传模型 Arc 地址；桌面可传 Arc 地址或文档 epoch。
    pub source_tag: u64,
}

struct KeyBucket {
    instances: Vec<NoteInstance>,
    tick_start: u32,
    tick_end: u32,
    note_revision: u64,
    track_visible_hash: u64,
    hidden_hash: u64,
    selection_hash: u64,
    track_selected_hash: u64,
    summary_block: Option<u32>,
}

/// 每个 key 的可见实例缓存 + 扁平输出。
pub struct NoteLayerCache {
    buckets: [Option<KeyBucket>; KEY_COUNT],
    flat: Vec<NoteInstance>,
    /// 上传缓存键：只随窗口/依赖/可见 key 的 revision 变化，不随滚动像素变化。
    cache_key: u64,
    last_window: Option<NoteWindow>,
    last_summary: Option<u32>,
    source_tag: u64,
    /// 区分 PR/AR 等不同消费方，避免切换视图时 layer 缓存键碰撞。
    layer_tag: u64,
}

impl NoteLayerCache {
    pub fn new(layer_tag: u64) -> Self {
        Self {
            buckets: std::array::from_fn(|_| None),
            flat: Vec::new(),
            cache_key: 0,
            last_window: None,
            last_summary: None,
            source_tag: 0,
            layer_tag,
        }
    }

    /// 当前扁平实例（仅在 `update` 返回 true 或首次调用后有内容）。
    pub fn instances(&self) -> &[NoteInstance] {
        &self.flat
    }

    /// 上传缓存键。相同键表示 GPU 上的层内容无需重传。
    pub fn cache_key(&self) -> u64 {
        self.cache_key
    }

    pub fn window(&self) -> Option<NoteWindow> {
        self.last_window
    }

    /// 更新缓存。返回 true 表示窗口/依赖变化，调用方应重传扁平层。
    pub fn update(&mut self, p: &NoteCacheParams<'_>) -> bool {
        if self.source_tag != p.source_tag {
            self.buckets = std::array::from_fn(|_| None);
            self.flat.clear();
            self.last_window = None;
            self.last_summary = None;
            self.cache_key = 0;
            self.source_tag = p.source_tag;
        }
        let window = plan_window(p.view, p.width, p.height);
        let summary_level = select_summary_level(p.view.base.pixels_per_tick);
        let summary_block = summary_level.map(|i| SUMMARY_BLOCK_TICKS[i]);
        let tv_hash = crate::hash_bools(p.track_visible);
        let hidden_hash = p.hidden_hash;
        // 摘要层不带选中位，也不做选中轨道排序；selection/track_selected
        // 变化只需要改 uniform，不应让整层摘要重建。
        let selection_hash = if summary_block.is_none() {
            p.selection.map(Selection::state_hash).unwrap_or(0)
        } else {
            0
        };
        let track_selected_hash = if summary_block.is_none() {
            hash_track_set(p.track_selected)
        } else {
            0
        };

        let mut rebuilt = false;
        for key in window.key_lo..=window.key_hi {
            let revision = p.note_revisions.get(key as usize).copied().unwrap_or(0);
            let up_to_date = self.buckets[key as usize].as_ref().is_some_and(|b| {
                b.tick_start == window.tick_start
                    && b.tick_end == window.tick_end
                    && b.note_revision == revision
                    && b.track_visible_hash == tv_hash
                    && b.hidden_hash == hidden_hash
                    && b.selection_hash == selection_hash
                    && b.track_selected_hash == track_selected_hash
                    && b.summary_block == summary_block
            });
            if up_to_date {
                continue;
            }

            let mut instances = build_key_notes_in_range(
                p.midi,
                key,
                window.tick_start,
                window.tick_end,
                p.hidden_notes,
                p.track_visible,
                if summary_block.is_none() {
                    p.selection
                } else {
                    None
                },
                if summary_block.is_none() {
                    Some(p.track_selected)
                } else {
                    None
                },
            );
            if let Some(block) = summary_block {
                instances = build_key_summary(key, &instances, block);
            }
            self.buckets[key as usize] = Some(KeyBucket {
                instances,
                tick_start: window.tick_start,
                tick_end: window.tick_end,
                note_revision: revision,
                track_visible_hash: tv_hash,
                hidden_hash,
                selection_hash,
                track_selected_hash,
                summary_block,
            });
            rebuilt = true;
        }

        let layout_changed = self.last_window != Some(window) || self.last_summary != summary_block;
        if rebuilt || layout_changed {
            self.flat.clear();
            for key in window.key_lo..=window.key_hi {
                if let Some(bucket) = &self.buckets[key as usize] {
                    self.flat.extend_from_slice(&bucket.instances);
                }
            }
            self.last_window = Some(window);
            self.last_summary = summary_block;
        }

        let key_count = usize::from(window.key_hi.saturating_sub(window.key_lo)) + 1;
        let mut parts = Vec::with_capacity(16 + key_count);
        parts.push(self.layer_tag);
        parts.push(p.source_tag);
        parts.push(u64::from(p.view.orientation as u8));
        parts.push(window.tick_start as u64);
        parts.push(window.tick_end as u64);
        parts.push(u64::from(window.key_lo));
        parts.push(u64::from(window.key_hi));
        parts.push(summary_block.map_or(0, u64::from));
        parts.push(tv_hash);
        parts.push(hidden_hash);
        parts.push(selection_hash);
        parts.push(track_selected_hash);
        for key in window.key_lo..=window.key_hi {
            parts.push(p.note_revisions.get(key as usize).copied().unwrap_or(0));
        }
        self.cache_key = layer_cache_key(&parts).max(1);
        rebuilt || layout_changed
    }
}

impl Default for NoteLayerCache {
    fn default() -> Self {
        Self::new(0)
    }
}

/// 与顺序无关的轨道集合 hash。
fn hash_track_set(set: &HashSet<u16>) -> u64 {
    set.iter().fold(0u64, |h, &track| {
        h.wrapping_add((u64::from(track) + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_key_notes_in_range;
    use yinhe_test_helpers::make_midi;
    use yinhe_types::TimelineViewBase;

    fn make_view() -> PianoRollView {
        PianoRollView {
            base: TimelineViewBase {
                pixels_per_tick: 1.0,
                scroll_x: 0.0,
                scroll_y: 1180.0,
                left_panel_width: 0.0,
                dirty: true,
                track_panel_row_height: 40.0,
                track_panel_scroll_y: 0.0,
                follow_target: None,
                follow_anim_start: 0.0,
                follow_anim_elapsed: 0.0,
            },
            key_height: 10.0,
            ..Default::default()
        }
    }

    fn make_model() -> yinhe_test_helpers::MockMidi {
        make_midi(vec![
            (5, 100, 200, 0, 100),
            (5, 500, 600, 0, 90),
            (5, 1500, 1600, 0, 80),
            (6, 1200, 1300, 1, 70),
        ])
    }

    fn empty_visible() -> Vec<bool> {
        vec![true, true]
    }

    fn update_cache(
        cache: &mut NoteLayerCache,
        model: &yinhe_test_helpers::MockMidi,
        view: &PianoRollView,
        visible: &[bool],
        selection: &Selection,
        tracks: &HashSet<u16>,
        hidden: &HashSet<(u16, u32, u8)>,
        revisions: &[u64; KEY_COUNT],
    ) -> bool {
        cache.update(&NoteCacheParams {
            midi: model,
            view,
            width: 1000.0,
            height: 100.0,
            track_visible: visible,
            track_selected: tracks,
            hidden_notes: hidden,
            hidden_hash: crate::hash_hidden(hidden),
            selection: Some(selection),
            note_revisions: revisions,
            source_tag: 7,
        })
    }

    #[test]
    fn window_is_stable_within_one_screen_and_moves_after_it() {
        let mut view = make_view();
        let a = plan_window(&view, 1000.0, 100.0);
        view.base.scroll_x = 200.0;
        let b = plan_window(&view, 1000.0, 100.0);
        assert_eq!(a, b, "同一跨度内滚动不应改变窗口");
        view.base.scroll_x = 900.0;
        let c = plan_window(&view, 1000.0, 100.0);
        assert_ne!(a, c, "跨过窗口边界后应重建窗口");
    }

    #[test]
    fn cache_skips_rebuild_inside_padding() {
        let model = make_model();
        let mut view = make_view();
        let mut cache = NoteLayerCache::new(1);
        let visible = empty_visible();
        let selection = Selection::default();
        let selected_tracks = HashSet::new();
        let hidden = HashSet::new();
        let revisions = [0u64; KEY_COUNT];

        assert!(update_cache(
            &mut cache,
            &model,
            &view,
            &visible,
            &selection,
            &selected_tracks,
            &hidden,
            &revisions
        ));
        assert_eq!(cache.instances().len(), 4, "窗口内应包含 4 个音符");
        let key = cache.cache_key();

        view.base.scroll_x = 200.0;
        assert!(
            !update_cache(
                &mut cache,
                &model,
                &view,
                &visible,
                &selection,
                &selected_tracks,
                &hidden,
                &revisions
            ),
            "padding 内滚动不应重建"
        );
        assert_eq!(cache.cache_key(), key);

        view.base.scroll_x = 900.0;
        assert!(
            update_cache(
                &mut cache,
                &model,
                &view,
                &visible,
                &selection,
                &selected_tracks,
                &hidden,
                &revisions
            ),
            "跨窗口后应重建"
        );
    }

    #[test]
    fn cache_rebuilds_only_changed_key_revision() {
        let model = make_model();
        let view = make_view();
        let mut cache = NoteLayerCache::new(1);
        let visible = empty_visible();
        let selection = Selection::default();
        let selected_tracks = HashSet::new();
        let hidden = HashSet::new();
        let mut revisions = [0u64; KEY_COUNT];

        assert!(update_cache(
            &mut cache,
            &model,
            &view,
            &visible,
            &selection,
            &selected_tracks,
            &hidden,
            &revisions
        ));
        assert!(!update_cache(
            &mut cache,
            &model,
            &view,
            &visible,
            &selection,
            &selected_tracks,
            &hidden,
            &revisions
        ));
        revisions[5] += 1;
        assert!(
            update_cache(
                &mut cache,
                &model,
                &view,
                &visible,
                &selection,
                &selected_tracks,
                &hidden,
                &revisions
            ),
            "key 5 的 revision 变化应触发重建"
        );
        assert!(!update_cache(
            &mut cache,
            &model,
            &view,
            &visible,
            &selection,
            &selected_tracks,
            &hidden,
            &revisions
        ));
    }

    #[test]
    fn hidden_note_is_removed_from_flat() {
        let model = make_model();
        let view = make_view();
        let mut cache = NoteLayerCache::new(1);
        let visible = empty_visible();
        let selection = Selection::default();
        let selected_tracks = HashSet::new();
        let revisions = [0u64; KEY_COUNT];
        let mut hidden = HashSet::new();

        assert!(update_cache(
            &mut cache,
            &model,
            &view,
            &visible,
            &selection,
            &selected_tracks,
            &hidden,
            &revisions
        ));
        assert_eq!(cache.instances().len(), 4);
        hidden.insert((0, 100, 5));
        assert!(update_cache(
            &mut cache,
            &model,
            &view,
            &visible,
            &selection,
            &selected_tracks,
            &hidden,
            &revisions
        ));
        assert_eq!(cache.instances().len(), 3);
    }

    #[test]
    fn key_range_is_restricted_to_bucket() {
        let model = make_model();
        let visible = empty_visible();
        let hidden = HashSet::new();
        let selected: HashSet<u16> = HashSet::new();
        let notes =
            build_key_notes_in_range(&model, 5, 0, 2000, &hidden, &visible, None, Some(&selected));
        assert_eq!(notes.len(), 3);
    }
}
