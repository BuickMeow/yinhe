//! LC 冻结状态编辑与失效处理。
//!
//! 冻结的 `FrozenAudio` 记录创建时的工程 revision 与混音 revision；任一变化
//! 即视为失效。失效的冻结在推送混音参数前排掉，播放回退到实时输出，避免
//! 用户改了音符/insert 却仍听到旧冻结音频。

use yinhe_mixer::{FrozenAudio, MixerNode};
use yinhe_types::MixerNodeId;

use crate::history::UndoAction;

use super::Document;

impl Document {
    fn mixer_nodes_mut(&mut self) -> impl Iterator<Item = &mut MixerNode> {
        self.mixer.nodes_mut()
    }

    /// 设置/清除一个节点的冻结音频。
    ///
    /// `after` 里的 revision 由这里统一盖章（调用方不需要知道当前 revision）；
    /// 如果与当前状态相同则不产生 undo。冻结属于混音段，标记 dirty 但**不**
    /// 推进 `data.revision`（冻结本身不改变工程内容）。
    pub fn set_node_freeze(
        &mut self,
        node: MixerNodeId,
        after: Option<FrozenAudio>,
    ) -> Option<UndoAction> {
        let before = self.mixer.node(node)?.freeze.clone();
        if before.is_none() && after.is_none() {
            return None;
        }
        // 先推进 mixer revision，再把冻结锚到推进后的值；否则冻结会立刻失效。
        self.mark_mixer_dirty();
        let mut after = after;
        if let Some(freeze) = after.as_mut() {
            freeze.source_revision = self.data.revision;
        }
        if before == after {
            return None;
        }
        self.mixer.node_mut(node)?.freeze = after.clone();
        Some(UndoAction::Freeze {
            node,
            before,
            after,
        })
    }

    /// 加载工程后调用：把已有冻结的 revision 重新锚定到当前会话。
    ///
    /// `ProjectData::revision` 是会话内计数器，不落盘；存档里的冻结在保存
    /// 时是有效的，重开后不能因为 revision 变了就判失效。
    pub fn adopt_loaded_freezes(&mut self) -> bool {
        let revision = self.data.revision;
        let mut changed = false;
        for node in self.mixer_nodes_mut() {
            let Some(freeze) = node.freeze.as_mut() else {
                continue;
            };
            if freeze.source_revision != revision {
                freeze.source_revision = revision;
                changed = true;
            }
        }
        changed
    }

    /// 清除 revision 已不匹配的冻结，返回是否有变化。
    pub fn prune_stale_freezes(&mut self) -> bool {
        let revision = self.data.revision;
        let mut changed = false;
        for node in self.mixer_nodes_mut() {
            let stale = node
                .freeze
                .as_ref()
                .is_some_and(|freeze| freeze.source_revision != revision);
            if stale {
                node.freeze = None;
                changed = true;
            }
        }
        if changed {
            self.mark_mixer_dirty();
        }
        changed
    }

    /// 是否存在已失效的冻结（用于音符编辑后决定是否重推混音参数）。
    pub fn has_stale_freezes(&self) -> bool {
        let revision = self.data.revision;
        self.mixer
            .nodes()
            .filter_map(|node| node.freeze.as_ref())
            .any(|freeze| freeze.source_revision != revision)
    }

    /// 某节点的冻结是否已失效（UI 显示用）。没有冻结返回 false。
    pub fn freeze_is_stale(&self, node: MixerNodeId) -> bool {
        self.mixer
            .node(node)
            .and_then(|n| n.freeze.as_ref())
            .is_some_and(|freeze| freeze.source_revision != self.data.revision)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_mixer::FrozenAudio;

    fn frozen() -> FrozenAudio {
        FrozenAudio {
            source: "frozen".into(),
            source_revision: 0,
            sample_rate: 48_000,
            frames: 16,
        }
    }

    #[test]
    fn set_freeze_stamps_revisions_and_invalidates_on_edit() {
        let mut doc = Document::empty();
        let node = MixerNodeId::LogicalChannel(yinhe_types::LogicalChannelId(0));
        let action = doc.set_node_freeze(node, Some(frozen())).expect("freeze");
        assert!(!doc.freeze_is_stale(node));

        doc.data.bump_revision();
        assert!(doc.freeze_is_stale(node));
        assert!(doc.has_stale_freezes());

        action.reversed().redo(&mut doc);
        assert!(doc.mixer.node(node).expect("node").freeze.is_none());
    }

    #[test]
    fn loaded_freezes_are_reanchored() {
        let mut doc = Document::empty();
        let node = MixerNodeId::LogicalChannel(yinhe_types::LogicalChannelId(0));
        doc.mixer.node_mut(node).expect("node").freeze = Some(FrozenAudio {
            source: "saved".into(),
            source_revision: 1,
            sample_rate: 48_000,
            frames: 8,
        });
        assert!(doc.adopt_loaded_freezes());
        assert!(!doc.freeze_is_stale(node));
    }

    #[test]
    fn engine_copy_drops_stale_freeze() {
        let mut doc = Document::empty();
        let node = MixerNodeId::LogicalChannel(yinhe_types::LogicalChannelId(0));
        doc.set_node_freeze(node, Some(frozen())).expect("freeze");
        let mut params = doc.mixer.clone();
        params.strip_stale_freezes(doc.data.revision);
        assert!(params.node(node).expect("node").freeze.is_some());

        doc.data.bump_revision();
        let mut params = doc.mixer.clone();
        params.strip_stale_freezes(doc.data.revision);
        assert!(params.node(node).expect("node").freeze.is_none());
    }
}
