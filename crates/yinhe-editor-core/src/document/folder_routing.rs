//! 文件夹总线与 Track→Channel→Bus 路由推导。
//!
//! Studio One 语义：Track 只是时间线内容，声音进入它所属的 MixerChannel
//! （Rack），Channel 再决定主输出到 Master 还是某条 Bus。文件夹本身是
//! 轨道组织容器，但可以拥有自己的总线；文件夹内所有音轨的**整条源通道**
//! 一起送入该总线，嵌套文件夹的总线再送入父文件夹总线。
//!
//! 关键约束（A01 跨文件夹规则）：同一个源通道只能属于一个文件夹总线。
//! 若使用该通道的音轨被拆到文件夹内外，或拆到两个不同文件夹，该通道
//! 保持 Master 输出并在报告中列为 split channel，绝不只把一部分声音送入
//! 文件夹总线（引擎按通道混音，无法按轨拆分）。

use std::collections::HashMap;

use yinhe_types::MixerChannelId;

use super::Document;

/// 一次文件夹路由同步的结果。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FolderRoutingReport {
    /// 因跨文件夹/文件夹内外混用而无法送入文件夹总线的源通道。
    pub split_channels: Vec<MixerChannelId>,
    /// 本次同步是否增删了文件夹总线（需要标记混音参数已修改）。
    pub buses_changed: bool,
}

impl Document {
    /// 同步文件夹总线：确保每个文件夹有唯一总线，并推导 MIDI/音频通道与
    /// 总线的主输出。幂等；音轨结构变化、工程加载、撤销/重做后调用。
    pub fn sync_folder_routing(&mut self) -> FolderRoutingReport {
        let buses_changed = self.ensure_folder_buses();
        let split_channels = self.derive_folder_outputs();
        FolderRoutingReport {
            split_channels,
            buses_changed,
        }
    }

    /// 删除总线并同步重映射文件夹引用（MIX 删除总线按钮走这里）。
    pub fn remove_bus(&mut self, bus: u16) {
        if bus as usize >= self.mixer.buses.len() {
            return;
        }
        self.mixer.remove_bus(bus);
        let model = std::sync::Arc::make_mut(&mut self.data.model);
        for track in &mut model.tracks {
            let track = std::sync::Arc::make_mut(track);
            match track.folder_bus {
                Some(b) if b == bus => track.folder_bus = None,
                Some(b) if b > bus => track.folder_bus = Some(b - 1),
                _ => {}
            }
        }
        let _ = self.sync_folder_routing();
    }

    /// 确保每个 Folder 轨持有唯一且有效的文件夹总线。
    /// 返回是否真的增删了总线。
    fn ensure_folder_buses(&mut self) -> bool {
        let mut changed = false;

        // 1. 回收已无文件夹引用的文件夹总线（从高索引删，避免连锁重映射）。
        loop {
            let referenced: Vec<u16> = self
                .data
                .model
                .tracks
                .iter()
                .filter(|t| t.kind == yinhe_core::TrackKind::Folder)
                .filter_map(|t| t.folder_bus)
                .collect();
            let orphan = (0..self.mixer.buses.len()).rev().find(|&bus| {
                self.mixer.is_folder_bus(bus as u16) && !referenced.contains(&(bus as u16))
            });
            let Some(bus) = orphan else {
                break;
            };
            let bus = bus as u16;
            self.mixer.remove_bus(bus);
            let model = std::sync::Arc::make_mut(&mut self.data.model);
            for track in &mut model.tracks {
                let track = std::sync::Arc::make_mut(track);
                match track.folder_bus {
                    Some(b) if b == bus => track.folder_bus = None,
                    Some(b) if b > bus => track.folder_bus = Some(b - 1),
                    _ => {}
                }
            }
            changed = true;
        }

        // 2. 校验每个文件夹：越界、重复、缺失 → 重新分配。
        let mut used: Vec<u16> = Vec::new();
        for idx in 0..self.data.model.tracks.len() {
            if self.data.model.tracks[idx].kind != yinhe_core::TrackKind::Folder {
                continue;
            }
            let current = self.data.model.tracks[idx].folder_bus;
            let valid = current
                .filter(|&bus| (bus as usize) < self.mixer.buses.len())
                .filter(|bus| !used.contains(bus));
            let bus = match valid {
                Some(bus) => {
                    // 旧工程可能只有 folder_bus、没有标记：认领为文件夹总线。
                    if let Some(flag) = self.mixer.folder_buses.get_mut(bus as usize)
                        && !*flag
                    {
                        *flag = true;
                        changed = true;
                    }
                    Some(bus)
                }
                None => {
                    let allocated = self.mixer.add_folder_bus();
                    changed |= allocated.is_some();
                    allocated
                }
            };
            if let Some(bus) = bus {
                used.push(bus);
            }
            if self.data.model.tracks[idx].folder_bus != bus {
                let model = std::sync::Arc::make_mut(&mut self.data.model);
                if let Some(track) = model.tracks.get_mut(idx) {
                    std::sync::Arc::make_mut(track).folder_bus = bus;
                }
            }
        }
        changed
    }

    /// 推导通道/总线主输出，返回无法整通道归入同一文件夹的源通道。
    fn derive_folder_outputs(&mut self) -> Vec<MixerChannelId> {
        let derived = derive_routing(&self.data.model, self.mixer.buses.len());
        self.mixer.midi_outputs = derived.midi_outputs;
        self.mixer.audio_outputs = derived.audio_outputs;
        self.mixer.bus_outputs = derived.bus_outputs;
        derived.split_channels
    }

    /// 只读：当前结构下被拆到不同文件夹（或文件夹内外）的源通道。
    /// 调用前应保证 `sync_folder_routing` 至少跑过一次（文件夹总线已分配）。
    pub fn split_folder_channels(&self) -> Vec<MixerChannelId> {
        derive_routing(&self.data.model, self.mixer.buses.len()).split_channels
    }

    /// 只读：某源通道当前的主输出总线（None = Master）。
    pub fn channel_bus_output(&self, channel: MixerChannelId) -> Option<u16> {
        match channel {
            MixerChannelId::Midi(ch) => self.mixer.midi_outputs.get(ch as usize).copied().flatten(),
            MixerChannelId::Audio(ch) => {
                self.mixer.audio_outputs.get(ch as usize).copied().flatten()
            }
            MixerChannelId::Bus(_) | MixerChannelId::Master => None,
        }
    }
}

#[derive(Default)]
struct DerivedRouting {
    midi_outputs: Vec<Option<u16>>,
    audio_outputs: Vec<Option<u16>>,
    bus_outputs: Vec<Option<u16>>,
    split_channels: Vec<MixerChannelId>,
}

fn derive_routing(model: &yinhe_core::YinModel, bus_count: usize) -> DerivedRouting {
    let tracks = &model.tracks;

    // 文件夹总线 → 父文件夹总线。
    let mut bus_outputs = vec![None; bus_count];
    for (idx, track) in tracks.iter().enumerate() {
        if track.kind != yinhe_core::TrackKind::Folder {
            continue;
        }
        let Some(bus) = track.folder_bus.filter(|&b| (b as usize) < bus_count) else {
            continue;
        };
        let parent = enclosing_folder_bus(tracks, idx).filter(|&b| b != bus);
        bus_outputs[bus as usize] = parent;
    }

    // 源通道 → 每一条使用它的音轨所属文件夹总线。
    let mut channels: HashMap<MixerChannelId, Vec<Option<u16>>> = HashMap::new();
    for (idx, track) in tracks.iter().enumerate() {
        let Some(channel) = track.mixer_channel() else {
            continue;
        };
        if !channel.is_source() {
            continue; // Folder / Master 不是源通道
        }
        let bus = enclosing_folder_bus(tracks, idx);
        channels.entry(channel).or_default().push(bus);
    }

    let mut midi_outputs = vec![None; yinhe_mixer::CHANNEL_COUNT];
    let mut audio_outputs: Vec<Option<u16>> = Vec::new();
    let mut split_channels = Vec::new();
    for (channel, buses) in channels {
        let first = buses[0];
        let same = buses.iter().all(|&bus| bus == first);
        if !same {
            if buses.iter().any(Option::is_some) {
                // 有轨在文件夹、有轨不在（或分属不同文件夹）：
                // 整条通道保持 Master，避免只送出部分声音。
                split_channels.push(channel);
            }
            continue;
        }
        let Some(bus) = first else {
            continue;
        };
        match channel {
            MixerChannelId::Midi(ch) => {
                if let Some(slot) = midi_outputs.get_mut(ch as usize) {
                    *slot = Some(bus);
                }
            }
            MixerChannelId::Audio(ch) => {
                let idx = ch as usize;
                if audio_outputs.len() <= idx {
                    audio_outputs.resize(idx + 1, None);
                }
                audio_outputs[idx] = Some(bus);
            }
            MixerChannelId::Bus(_) | MixerChannelId::Master => {}
        }
    }
    split_channels.sort_by_key(channel_order);

    DerivedRouting {
        midi_outputs,
        audio_outputs,
        bus_outputs,
        split_channels,
    }
}

/// 最近的祖先文件夹总线（不含自身）。
fn enclosing_folder_bus(
    tracks: &[std::sync::Arc<yinhe_core::TrackData>],
    idx: usize,
) -> Option<u16> {
    let depth = tracks.get(idx)?.folder_depth;
    if depth == 0 {
        return None;
    }
    (0..idx)
        .rev()
        .find(|&j| {
            tracks[j].kind == yinhe_core::TrackKind::Folder && tracks[j].folder_depth < depth
        })
        .and_then(|j| tracks[j].folder_bus)
}

fn channel_order(channel: &MixerChannelId) -> (u8, u32) {
    match channel {
        MixerChannelId::Midi(ch) => (0, u32::from(*ch)),
        MixerChannelId::Audio(ch) => (1, u32::from(*ch)),
        MixerChannelId::Bus(ch) => (2, u32::from(*ch)),
        MixerChannelId::Master => (3, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_core::{TrackData, TrackKind};

    fn folder(name: &str, depth: u8) -> TrackData {
        let mut t = TrackData::new(0, 0);
        t.name = name.to_string();
        t.kind = TrackKind::Folder;
        t.folder_depth = depth;
        t
    }

    fn midi(name: &str, channel: u8, depth: u8) -> TrackData {
        let mut t = TrackData::new(0, channel);
        t.name = name.to_string();
        t.folder_depth = depth;
        t
    }

    /// 只有整条通道都在同一文件夹内时才送入文件夹总线。
    #[test]
    fn whole_channel_inside_folder_routes_to_bus() {
        let mut doc = Document::empty();
        doc.data.model = std::sync::Arc::new(yinhe_core::YinModel {
            tracks: vec![
                std::sync::Arc::new(midi("A01", 0, 0)),
                std::sync::Arc::new(folder("Folder", 0)),
                std::sync::Arc::new(midi("A02", 1, 1)),
            ],
            ..Default::default()
        });
        std::sync::Arc::make_mut(&mut doc.data.model).normalize_folder_structure();
        let report = doc.sync_folder_routing();
        assert!(report.split_channels.is_empty());
        let bus = doc.data.model.tracks[1].folder_bus.expect("folder bus");
        assert_eq!(doc.mixer.midi_outputs[1], Some(bus), "A02 整通道进总线");
        assert_eq!(doc.mixer.midi_outputs[0], None, "A01 在文件夹外");
        assert_eq!(doc.mixer.bus_outputs[bus as usize], None);
    }

    /// A01 一条轨在文件夹、一条不在：整通道保持 Master 并报告 split。
    #[test]
    fn shared_channel_split_across_folder_stays_on_master() {
        let mut doc = Document::empty();
        doc.data.model = std::sync::Arc::new(yinhe_core::YinModel {
            tracks: vec![
                std::sync::Arc::new(folder("Folder", 0)),
                std::sync::Arc::new(midi("A01 in", 0, 1)),
                std::sync::Arc::new(midi("A01 out", 0, 0)),
            ],
            ..Default::default()
        });
        std::sync::Arc::make_mut(&mut doc.data.model).normalize_folder_structure();
        let report = doc.sync_folder_routing();
        assert_eq!(report.split_channels, vec![MixerChannelId::Midi(0)]);
        assert_eq!(doc.mixer.midi_outputs[0], None);
    }

    /// 同一通道出现在两个文件夹：不拆分到任一总线。
    #[test]
    fn shared_channel_across_two_folders_stays_on_master() {
        let mut doc = Document::empty();
        doc.data.model = std::sync::Arc::new(yinhe_core::YinModel {
            tracks: vec![
                std::sync::Arc::new(folder("F1", 0)),
                std::sync::Arc::new(midi("A01 a", 0, 1)),
                std::sync::Arc::new(folder("F2", 0)),
                std::sync::Arc::new(midi("A01 b", 0, 1)),
            ],
            ..Default::default()
        });
        std::sync::Arc::make_mut(&mut doc.data.model).normalize_folder_structure();
        let report = doc.sync_folder_routing();
        assert_eq!(report.split_channels, vec![MixerChannelId::Midi(0)]);
        assert_eq!(doc.mixer.midi_outputs[0], None);
        let f1 = doc.data.model.tracks[0].folder_bus.unwrap();
        let f2 = doc.data.model.tracks[2].folder_bus.unwrap();
        assert_ne!(f1, f2, "两个文件夹各有独立总线");
    }

    /// 嵌套文件夹：内层总线输出到外层总线。
    #[test]
    fn nested_folder_bus_outputs_to_parent_bus() {
        let mut doc = Document::empty();
        doc.data.model = std::sync::Arc::new(yinhe_core::YinModel {
            tracks: vec![
                std::sync::Arc::new(folder("Outer", 0)),
                std::sync::Arc::new(folder("Inner", 1)),
                std::sync::Arc::new(midi("A02", 1, 2)),
            ],
            ..Default::default()
        });
        std::sync::Arc::make_mut(&mut doc.data.model).normalize_folder_structure();
        let report = doc.sync_folder_routing();
        assert!(report.split_channels.is_empty());
        let outer = doc.data.model.tracks[0].folder_bus.unwrap();
        let inner = doc.data.model.tracks[1].folder_bus.unwrap();
        assert_eq!(doc.mixer.midi_outputs[1], Some(inner));
        assert_eq!(doc.mixer.bus_outputs[inner as usize], Some(outer));
        assert_eq!(doc.mixer.bus_outputs[outer as usize], None);
    }

    /// Cmd+G 归组立即分配文件夹总线；undo 后孤儿总线被回收。
    #[test]
    fn grouping_allocates_and_undo_reclaims_folder_bus() {
        let mut doc = Document::empty();
        doc.data.model = std::sync::Arc::new(yinhe_core::YinModel {
            tracks: vec![
                std::sync::Arc::new(midi("A01", 0, 0)),
                std::sync::Arc::new(midi("A02", 1, 0)),
            ],
            ..Default::default()
        });
        std::sync::Arc::make_mut(&mut doc.data.model).normalize_folder_structure();
        let before = doc.mixer.buses.len();
        let action = doc.group_tracks(&[0, 1]).expect("group");
        assert_eq!(doc.mixer.buses.len(), before + 1, "归组即分配总线");
        let bus = doc.data.model.tracks[0].folder_bus.expect("folder bus");
        assert_eq!(doc.mixer.midi_outputs[0], Some(bus));
        assert_eq!(doc.mixer.midi_outputs[1], Some(bus));

        action.reversed().redo(&mut doc);
        assert_eq!(doc.mixer.buses.len(), before, "undo 后回收文件夹总线");
        assert_eq!(doc.mixer.midi_outputs[0], None);
    }

    /// 删除文件夹后回收其总线，并重映射更高索引的文件夹引用。
    #[test]
    fn orphan_folder_bus_is_reclaimed() {
        let mut doc = Document::empty();
        doc.data.model = std::sync::Arc::new(yinhe_core::YinModel {
            tracks: vec![
                std::sync::Arc::new(folder("F1", 0)),
                std::sync::Arc::new(folder("F2", 0)),
            ],
            ..Default::default()
        });
        std::sync::Arc::make_mut(&mut doc.data.model).normalize_folder_structure();
        doc.sync_folder_routing();
        let f1 = doc.data.model.tracks[0].folder_bus.unwrap();
        let f2 = doc.data.model.tracks[1].folder_bus.unwrap();
        let before = doc.mixer.buses.len();
        // 模拟删除 F1 后只剩 F2：F2.folder_bus 仍指向原索引。
        doc.data.model = std::sync::Arc::new(yinhe_core::YinModel {
            tracks: vec![std::sync::Arc::new(folder("F2", 0))],
            ..Default::default()
        });
        std::sync::Arc::make_mut(&mut doc.data.model).tracks[0] = std::sync::Arc::new({
            let mut t = folder("F2", 0);
            t.folder_bus = Some(f2);
            t
        });
        doc.sync_folder_routing();
        assert!(doc.mixer.buses.len() < before, "孤儿总线被回收");
        let new_f2 = doc.data.model.tracks[0].folder_bus.unwrap();
        assert_eq!(new_f2, f1.min(f2), "F2 引用前移到存活索引");
    }
}
