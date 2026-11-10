//! 走带循环：开关、默认区间、选区取 Range、向音频引擎同步。

use yinhe_editor_core::document::Document;

use crate::app::App;

impl App {
    /// 以光标所在小节为起点、四小节为长度的默认循环区间。
    fn default_loop_range(doc: &Document) -> (u32, u32) {
        let bar = doc.data.model.tempo_map.bar_divide().max(1.0) as u32;
        let cursor = doc.edit.cursor_tick.unwrap_or(0.0).max(0.0) as u32;
        let start = cursor / bar * bar;
        (start, start.saturating_add(bar.saturating_mul(4)))
    }

    /// 当前框选的 tick 范围（PR / AR / 通道 AR 任一来源取并集）。
    fn selection_tick_range(doc: &Document) -> Option<(u32, u32)> {
        let mut lo = u32::MAX;
        let mut hi = 0u32;
        let mut take = |a: f64, b: f64| {
            let a = a.max(0.0).min(u32::MAX as f64) as u32;
            let b = b.max(0.0).min(u32::MAX as f64) as u32;
            if b > a {
                lo = lo.min(a);
                hi = hi.max(b);
            }
        };

        for &(t0, t1, _, _, _, _) in &doc.edit.selected.rects {
            take(t0 as f64, t1 as f64);
        }
        for &(t0, t1, _, _) in &doc.edit.sel_rect.effective_rects() {
            take(t0, t1);
        }
        for &(t0, t1, _, _) in &doc.edit.arr_sel_rect {
            take(t0, t1);
        }
        for r in &doc.edit.chan_ar_sel {
            take(r.t_start, r.t_end);
        }

        (hi > lo).then_some((lo, hi))
    }

    fn sync_loop_rows(doc: &mut Document) {
        let enabled = doc.edit.playback.loop_enabled();
        doc.edit.event_rows_ar.sync_loop(enabled);
        doc.edit.event_rows_pr.sync_loop(enabled);
    }

    /// 播放菜单/走带栏/快捷键：切换循环开关。
    pub(crate) fn toggle_loop(&mut self) {
        let enabled = self
            .workspace
            .active_doc
            .and_then(|idx| self.workspace.documents.get(idx))
            .is_some_and(|d| d.edit.playback.loop_enabled());
        self.set_loop_enabled(!enabled);
    }

    /// 直接设置循环开关。开启时若区间无效，以光标所在小节生成四小节默认区间。
    pub(crate) fn set_loop_enabled(&mut self, enabled: bool) {
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let Some(doc) = self.workspace.documents.get_mut(idx) else {
            return;
        };
        if enabled {
            let (s, e) = doc.edit.playback.loop_range();
            if e <= s {
                let (s, e) = Self::default_loop_range(doc);
                let _ = doc.edit.playback.set_loop_range(s, e);
            }
        }
        let _ = doc.edit.playback.set_loop_enabled(enabled);
        Self::sync_loop_rows(doc);
    }

    /// 播放菜单「循环选区」：把循环区间设为当前框选范围并开启。
    pub(crate) fn loop_to_selection(&mut self) {
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let Some(doc) = self.workspace.documents.get_mut(idx) else {
            return;
        };
        let Some((s, e)) = Self::selection_tick_range(doc) else {
            return;
        };
        if doc.edit.playback.set_loop_range(s, e) {
            let _ = doc.edit.playback.set_loop_enabled(true);
        }
        Self::sync_loop_rows(doc);
    }

    /// 每帧把当前文档的 loop 状态推给音频引擎（只在变化时发送）。
    pub(crate) fn sync_loop_to_engine(&mut self) {
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let Some(audio) = self.audio_state.handle.as_ref() else {
            return;
        };
        let Some(doc) = self.workspace.documents.get(idx) else {
            return;
        };
        let enabled = doc.edit.playback.loop_enabled();
        let (start, end) = doc.edit.playback.loop_range();
        let state = (enabled, start, end);
        if self.audio_state.loop_sent == Some(state) {
            return;
        }
        audio.handle.send(yinhe_audio::AudioCommand::SetLoop {
            enabled,
            start_tick: start,
            end_tick: end,
        });
        self.audio_state.loop_sent = Some(state);
    }
}
