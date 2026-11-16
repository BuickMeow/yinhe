//! Rack 输出拓扑与每轨 LC 的文档级操作。
//!
//! Rack 默认是单输出：整个 Rack 一条 LC，所有使用它的 Track 共享。
//! 内置音源可以切到 `PerTrack`：每条 MIDI Track 分配一个稳定 LC，
//! 引擎按轨拆分音频后分别进入各自的 LC。
//!
//! 外部单输出插件（Rack 里有 `instruments`）不支持按轨拆分；
//! 多输出插件使用 `PerOutput`，每个输出一条 LC，Track 显式映射到输出。

use std::sync::Arc;

use yinhe_core::TrackKind;
use yinhe_types::{RackOutputMode, RackOutputTopology};

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
        if mode == RackOutputMode::PerOutput {
            // PerOutput 只由 set_rack_outputs 按检测到的拓扑设置。
            return false;
        }
        // 关闭 PerOutput 时释放按输出分配的 LC。
        let old_output_lcs = std::mem::take(&mut self.mixer.racks[channel as usize].output_lcs);
        for lc in old_output_lcs {
            self.mixer.remove_track_lc(lc);
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
                    track.rack_output = None;
                    if let Some(id) = track.lc.take() {
                        self.mixer.remove_track_lc(id);
                    }
                }
                RackOutputMode::PerOutput => {}
            }
        }
        self.data.bump_revision();
        self.mark_mixer_dirty();
        true
    }

    /// 写入插件检测到的输出拓扑；多输出时分配每个输出一条 LC。
    ///
    /// 返回是否有变化。`PerOutput` 下每条 Track 的 `rack_output` 选择输出，
    /// `lc` 指向该输出的共享 LC。单输出/无输出回到 `Shared`。
    pub fn set_rack_outputs(&mut self, channel: u8, topology: RackOutputTopology) -> bool {
        let Some(rack) = self.mixer.racks.get(channel as usize) else {
            return false;
        };
        let count = topology.outputs.len();
        let old_count = rack.output_lcs.len();
        if rack.outputs == topology && (count <= 1 || old_count == count) {
            return false;
        }
        // 旧输出 LC 全部释放；稍后按新拓扑重建。
        let old_lcs = std::mem::take(&mut self.mixer.racks[channel as usize].output_lcs);
        for lc in old_lcs {
            self.mixer.remove_track_lc(lc);
        }
        self.mixer.racks[channel as usize].outputs = topology.clone();
        if count <= 1 {
            self.mixer.racks[channel as usize].output_mode = RackOutputMode::Shared;
            let model = Arc::make_mut(&mut self.data.model);
            for track in model.tracks.iter_mut() {
                if track.kind == TrackKind::Midi && track.global_channel() == channel {
                    let track = Arc::make_mut(track);
                    track.rack_output = None;
                    track.lc = None;
                }
            }
        } else {
            self.mixer.racks[channel as usize].output_mode = RackOutputMode::PerOutput;
            let source = self.mixer.channels[channel as usize].clone();
            let mut lcs = Vec::with_capacity(count);
            for _ in 0..count {
                let id = self.mixer.add_track_lc();
                if let Some(node) = self.mixer.track_node_mut(id) {
                    node.strip = source.strip;
                    node.inserts = source.inserts.clone();
                    node.sends = source.sends.clone();
                    node.output = source.output;
                }
                lcs.push(id);
            }
            let model = Arc::make_mut(&mut self.data.model);
            for track in model.tracks.iter_mut() {
                if track.kind != TrackKind::Midi || track.global_channel() != channel {
                    continue;
                }
                let out = track.rack_output.unwrap_or(0).min(count as u16 - 1);
                let track = Arc::make_mut(track);
                track.rack_output = Some(out);
                track.lc = Some(lcs[out as usize]);
            }
            self.mixer.racks[channel as usize].output_lcs = lcs;
        }
        self.data.bump_revision();
        self.mark_mixer_dirty();
        true
    }

    /// 修改某轨在多输出 Rack 中的输出映射（0 起）。
    pub fn set_track_rack_output(&mut self, track_idx: usize, output: u16) -> bool {
        let model = Arc::make_mut(&mut self.data.model);
        let Some(track) = model.tracks.get(track_idx) else {
            return false;
        };
        if track.kind != TrackKind::Midi {
            return false;
        }
        let channel = track.global_channel();
        let Some(rack) = self.mixer.racks.get(channel as usize) else {
            return false;
        };
        let Some(lc) = rack.output_lcs.get(output as usize).copied() else {
            return false;
        };
        if self.mixer.track_node(lc).is_none() {
            self.mixer.add_track_lc_with_id(lc);
        }
        let track = Arc::make_mut(&mut model.tracks[track_idx]);
        if track.rack_output == Some(output) && track.lc == Some(lc) {
            return false;
        }
        track.rack_output = Some(output);
        track.lc = Some(lc);
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
            match mode {
                RackOutputMode::PerOutput => {
                    let count = self.mixer.racks[channel as usize]
                        .output_lcs
                        .len()
                        .max(self.mixer.racks[channel as usize].outputs.outputs.len());
                    if count == 0 {
                        continue;
                    }
                    while self.mixer.racks[channel as usize].output_lcs.len() < count {
                        let id = self.mixer.add_track_lc();
                        self.mixer.racks[channel as usize].output_lcs.push(id);
                        changed = true;
                    }
                    let out = track.rack_output.unwrap_or(0).min(count as u16 - 1) as usize;
                    let id = self.mixer.racks[channel as usize].output_lcs[out];
                    if self.mixer.track_node(id).is_none() {
                        self.mixer.add_track_lc_with_id(id);
                        changed = true;
                    }
                    if track.lc != Some(id) || track.rack_output != Some(out as u16) {
                        let track = Arc::make_mut(track);
                        track.lc = Some(id);
                        track.rack_output = Some(out as u16);
                        changed = true;
                    }
                }
                RackOutputMode::PerTrack => match track.lc {
                    None => {
                        let id = self.mixer.add_track_lc();
                        Arc::make_mut(track).lc = Some(id);
                        changed = true;
                    }
                    Some(id) => {
                        if self.mixer.track_node(id).is_none() {
                            self.mixer.add_track_lc_with_id(id);
                            changed = true;
                        }
                    }
                },
                RackOutputMode::Shared => {
                    let track = Arc::make_mut(track);
                    if track.lc.take().is_some() || track.rack_output.take().is_some() {
                        changed = true;
                    }
                }
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
    fn multi_output_topology_allocates_output_lcs_and_maps_tracks() {
        use yinhe_types::{RackOutput, RackOutputTopology};
        let mut doc = Document::empty();
        let topology = RackOutputTopology {
            outputs: vec![
                RackOutput {
                    name: "Out 1".into(),
                    channels: 2,
                },
                RackOutput {
                    name: "Out 2".into(),
                    channels: 2,
                },
            ],
            dynamic: true,
        };
        assert!(doc.set_rack_outputs(0, topology));
        let lcs = doc.mixer.racks[0].output_lcs.clone();
        assert_eq!(lcs.len(), 2);
        assert_eq!(doc.rack_output_mode(0), RackOutputMode::PerOutput);
        assert_eq!(doc.data.model.tracks[1].lc, Some(lcs[0]));
        assert_eq!(doc.data.model.tracks[1].rack_output, Some(0));

        assert!(doc.set_track_rack_output(1, 1));
        assert_eq!(doc.data.model.tracks[1].lc, Some(lcs[1]));
        assert_eq!(doc.data.model.tracks[1].rack_output, Some(1));

        assert!(doc.set_rack_output_mode(0, RackOutputMode::Shared));
        assert!(doc.data.model.tracks[1].lc.is_none());
        assert!(doc.mixer.track_node(lcs[0]).is_none());
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
