//! Rack 输出拓扑与每轨 LC 的文档级操作。
//!
//! Rack 默认是单输出：整个 Rack 一条 LC，所有使用它的 Track 共享。
//! 内置音源可以切到 `PerTrack`：每条 MIDI Track 分配一个稳定 LC，
//! 引擎按轨拆分音频后分别进入各自的 LC。
//!
//! 外部插件（Rack 里有 `instruments`）不支持按轨拆分，切换会被拒绝。

use std::sync::Arc;

use yinhe_core::TrackKind;
use yinhe_types::RackOutputMode;

use crate::document::Document;

impl Document {
    /// 某源 MIDI 通道的 Rack 输出模式。
    pub fn rack_output_mode(&self, channel: u8) -> RackOutputMode {
        self.mixer
            .racks
            .get(channel as usize)
            .map(|rack| rack.output_mode)
            .unwrap_or_default()
    }

    /// 切换 Rack 输出模式，并按模式分配/清理每轨 LC。
    ///
    /// 返回是否有变化。`PerTrack` 只允许内置音源（Rack 无插件乐器）；
    /// 外部单输出插件无法按轨拆分，直接拒绝。
    pub fn set_rack_output_mode(&mut self, channel: u8, mode: RackOutputMode) -> bool {
        let Some(rack) = self.mixer.racks.get(channel as usize) else {
            return false;
        };
        if rack.output_mode == mode {
            return false;
        }
        if mode == RackOutputMode::PerTrack && !rack.instruments.is_empty() {
            return false;
        }

        self.mixer.racks[channel as usize].output_mode = mode;
        let model = Arc::make_mut(&mut self.data.model);
        for track in model.tracks.iter_mut() {
            if track.kind != TrackKind::Midi || track.global_channel() != channel {
                continue;
            }
            match mode {
                RackOutputMode::PerTrack => {
                    if track.lc.is_none() {
                        // 复制共享 LC 的混音参数，避免切换模式后听感突变。
                        let source = self.mixer.channels[channel as usize].clone();
                        let id = self.mixer.add_track_lc();
                        if let Some(node) = self.mixer.track_node_mut(id) {
                            node.strip = source.strip;
                            node.inserts = source.inserts.clone();
                            node.sends = source.sends.clone();
                            node.output = source.output;
                        }
                        Arc::make_mut(track).lc = Some(id);
                    }
                }
                RackOutputMode::Shared => {
                    let track = Arc::make_mut(track);
                    if let Some(id) = track.lc.take() {
                        self.mixer.remove_track_lc(id);
                    }
                }
            }
        }
        self.data.bump_revision();
        self.mark_mixer_dirty();
        true
    }

    /// 加载后修复 per-track Rack 的 LC：
    /// - `PerTrack` 但 Track 没有 `lc` → 分配；
    /// - Track 有 `lc` 但节点不存在 → 创建；
    /// - `Shared` 但 Track 还留着 `lc` → 清掉并删除节点。
    ///
    /// 返回是否有修复。
    pub fn ensure_rack_track_lcs(&mut self) -> bool {
        let model = Arc::make_mut(&mut self.data.model);
        let mut changed = false;
        for track in model.tracks.iter_mut() {
            if track.kind != TrackKind::Midi {
                continue;
            }
            let channel = track.global_channel();
            let mode = self
                .mixer
                .racks
                .get(channel as usize)
                .map(|rack| rack.output_mode)
                .unwrap_or_default();
            match (mode, track.lc) {
                (RackOutputMode::PerTrack, None) => {
                    let id = self.mixer.add_track_lc();
                    Arc::make_mut(track).lc = Some(id);
                    changed = true;
                }
                (RackOutputMode::PerTrack, Some(id)) => {
                    if self.mixer.track_node(id).is_none() {
                        self.mixer.add_track_lc_with_id(id);
                        changed = true;
                    }
                }
                (RackOutputMode::Shared, Some(id)) => {
                    Arc::make_mut(track).lc = None;
                    self.mixer.remove_track_lc(id);
                    changed = true;
                }
                (RackOutputMode::Shared, None) => {}
            }
        }
        if changed {
            self.data.bump_revision();
            self.mark_mixer_dirty();
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_track_mode_assigns_and_releases_lcs() {
        let mut doc = Document::empty();
        // Track 1 是 A01（索引 0 是 Master）。
        assert_eq!(doc.data.model.tracks[1].global_channel(), 0);
        assert!(doc.set_rack_output_mode(0, RackOutputMode::PerTrack));

        let lc = doc.data.model.tracks[1].lc.expect("per-track lc");
        assert!(doc.mixer.track_node(lc).is_some());
        assert_eq!(doc.rack_output_mode(0), RackOutputMode::PerTrack);

        assert!(doc.set_rack_output_mode(0, RackOutputMode::Shared));
        assert!(doc.data.model.tracks[1].lc.is_none());
        assert!(doc.mixer.track_node(lc).is_none());
    }

    #[test]
    fn plugin_rack_rejects_per_track_mode() {
        let mut doc = Document::empty();
        doc.mixer.racks[0]
            .instruments
            .push(yinhe_mixer::InsertRef::default());
        assert!(!doc.set_rack_output_mode(0, RackOutputMode::PerTrack));
        assert_eq!(doc.rack_output_mode(0), RackOutputMode::Shared);
        assert!(doc.data.model.tracks[1].lc.is_none());
    }

    #[test]
    fn ensure_repairs_missing_nodes_and_clears_shared_lcs() {
        let mut doc = Document::empty();
        doc.mixer.racks[0].output_mode = RackOutputMode::PerTrack;
        assert!(doc.data.model.tracks[1].lc.is_none());

        assert!(doc.ensure_rack_track_lcs());
        let lc = doc.data.model.tracks[1].lc.expect("assigned lc");
        assert!(doc.mixer.track_node(lc).is_some());

        // 手工删节点，加载修复应重建。
        doc.mixer.remove_track_lc(lc);
        assert!(doc.ensure_rack_track_lcs());
        assert!(doc.mixer.track_node(lc).is_some());
    }
}
