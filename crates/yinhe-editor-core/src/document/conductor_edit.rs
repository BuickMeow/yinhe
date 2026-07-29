//! Conductor-track event editing: TimeSig.
//!
//! 与 `automation_edit` 不同，conductor 的拍号事件数量极少（通常 < 10），
//! 因此用全量 before/after 快照而非 per-event delta，简化 undo 逻辑。

use std::sync::Arc;

use yinhe_types::TimeSigEvent;

use super::Document;

impl Document {
    /// 按 `old_tick` 找到 `conductor.time_sig` 事件并修改其字段。
    ///
    /// 修改 tick 后重新排序，保持 `time_sig` 按 tick 升序。
    /// 返回 `(before, after)` 全量快照供调用方 push undo。
    /// TempoMap 依赖 time_sig，会同步重建。
    pub fn set_time_sig_event(
        &mut self,
        old_tick: u32,
        new_tick: u32,
        new_numerator: u8,
        new_denominator: u8,
    ) -> Option<(Vec<TimeSigEvent>, Vec<TimeSigEvent>)> {
        let model = Arc::make_mut(&mut self.data.model);
        let conductor = Arc::make_mut(&mut model.conductor);
        let idx = conductor.time_sig.iter().position(|e| e.tick == old_tick)?;
        let before = conductor.time_sig.clone();
        {
            let event = &mut conductor.time_sig[idx];
            event.tick = new_tick;
            event.numerator = new_numerator;
            event.denominator = new_denominator;
        }
        conductor.time_sig.sort_by_key(|e| e.tick);
        let after = conductor.time_sig.clone();
        // conductor 借用已释放（NLL），可重建 tempo_map
        model.rebuild_tempo_map();
        self.data.bump_revision();
        Some((before, after))
    }
}
