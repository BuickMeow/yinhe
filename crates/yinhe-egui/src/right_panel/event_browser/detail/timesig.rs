//! 拍号事件详情。

use super::*;

// ── TimeSig ──

pub(super) fn show_timesig_detail(
    ui: &mut egui::Ui,
    doc: &mut Document,
    bar_lookup: &BarLookup,
    state: &mut EventBrowserState,
) -> Option<JumpRequest> {
    // 先 clone 出 owned 数据，避免不可变借用阻塞后续 &mut doc 编辑
    let mut sorted: Vec<yinhe_types::TimeSigEvent> = doc.data.model.conductor.time_sig.clone();
    sorted.sort_by_key(|e| e.tick);
    let (page_start, _) = event_table_section(
        ui,
        state,
        "eb_ts",
        t!("event_browser.title.timesig").as_ref(),
        &sorted,
        &[
            ("#", 40.0),
            (t!("event_browser.header.tick").as_ref(), 70.0),
            (t!("event_browser.header.position").as_ref(), 80.0),
            (t!("event_browser.header.numerator").as_ref(), 50.0),
            (t!("event_browser.header.denominator").as_ref(), 50.0),
        ],
        |e| e.tick,
        |i, row, click_key, state, e, page_ticks, page_start| {
            let denom = 1u32 << e.denominator as u32;
            cell_row_header(
                row,
                state,
                "eb_ts_edit",
                i,
                page_start,
                e.tick,
                page_ticks,
                click_key,
            );
            cell_editable(
                row,
                "eb_ts_edit",
                i,
                format!("{}", e.tick),
                EditRequest::TimeSigTick { tick: e.tick },
                click_key,
            );
            cell_position(
                row,
                bar_lookup,
                "eb_ts_edit",
                i,
                e.tick,
                |nt| EditRequest::TimeSigTick { tick: nt },
                click_key,
            );
            cell_editable(
                row,
                "eb_ts_edit",
                i,
                format!("{}", e.numerator),
                EditRequest::TimeSigNumerator { tick: e.tick },
                click_key,
            );
            cell_editable(
                row,
                "eb_ts_edit",
                i,
                format!("{}", denom),
                EditRequest::TimeSigDenominator { tick: e.tick },
                click_key,
            );
        },
    );
    apply_timesig_popups(ui, doc, "eb_ts_edit", bar_lookup);
    apply_timesig_ops(ui, doc, state, "eb_ts_edit");
    take_row_click(ui, "eb_ts").map(|i| JumpRequest {
        tick: sorted[page_start + i].tick,
        note: None,
    })
}
