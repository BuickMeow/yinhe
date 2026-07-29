//! Conductor-track event editing: TimeSig / KeySig / Marker.
//!
//! 与 `automation_edit` 不同，conductor 的拍号/调号/标记事件数量极少
//! （通常 < 10），因此用全量 before/after 快照而非 per-event delta，
//! 简化 undo 逻辑。
//!
//! popup 层自己用 `record_*_before` / `finalize_*_undo` 管理 undo 快照，
//! 这里只负责修改数据，不返回快照。

use std::sync::Arc;

use super::Document;

impl Document {
    /// 按 `old_tick` 找到 `conductor.time_sig` 事件并修改其字段。
    ///
    /// 修改 tick 后重新排序，保持 `time_sig` 按 tick 升序。
    /// TempoMap 依赖 time_sig，会同步重建。
    /// 未找到对应 tick 的事件时静默返回。
    pub fn set_time_sig_event(
        &mut self,
        old_tick: u32,
        new_tick: u32,
        new_numerator: u8,
        new_denominator: u8,
    ) {
        let model = Arc::make_mut(&mut self.data.model);
        let conductor = Arc::make_mut(&mut model.conductor);
        let Some(idx) = conductor.time_sig.iter().position(|e| e.tick == old_tick) else { return };
        {
            let event = &mut conductor.time_sig[idx];
            event.tick = new_tick;
            event.numerator = new_numerator;
            event.denominator = new_denominator;
        }
        conductor.time_sig.sort_by_key(|e| e.tick);
        // conductor 借用已释放（NLL），可重建 tempo_map
        model.rebuild_tempo_map();
        self.data.bump_revision();
    }

    /// 按 `old_tick` 找到 `conductor.key_sig` 事件并修改其字段。
    /// 未找到对应 tick 的事件时静默返回。
    pub fn set_keysig_event(
        &mut self,
        old_tick: u32,
        new_tick: u32,
        new_sf: i8,
        new_mi: u8,
    ) {
        let model = Arc::make_mut(&mut self.data.model);
        let conductor = Arc::make_mut(&mut model.conductor);
        let Some(idx) = conductor.key_sig.iter().position(|e| e.tick == old_tick) else { return };
        {
            let event = &mut conductor.key_sig[idx];
            event.tick = new_tick;
            event.sf = new_sf;
            event.mi = new_mi;
        }
        conductor.key_sig.sort_by_key(|e| e.tick);
        self.data.bump_revision();
    }

    /// 按 `old_tick` 找到 `conductor.markers` 事件并修改其字段。
    /// 未找到对应 tick 的事件时静默返回。
    pub fn set_marker_event(
        &mut self,
        old_tick: u32,
        new_tick: u32,
        new_text: String,
    ) {
        let model = Arc::make_mut(&mut self.data.model);
        let conductor = Arc::make_mut(&mut model.conductor);
        let Some(idx) = conductor.markers.iter().position(|e| e.tick == old_tick) else { return };
        {
            let event = &mut conductor.markers[idx];
            event.tick = new_tick;
            event.text = new_text;
        }
        conductor.markers.sort_by_key(|e| e.tick);
        self.data.bump_revision();
    }
}
