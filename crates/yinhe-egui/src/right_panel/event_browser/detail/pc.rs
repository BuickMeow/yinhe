//! Program Change 事件详情。

use super::*;

// ── Program Change ──

pub(super) fn show_pc_detail(
    ui: &mut egui::Ui,
    doc: &mut Document,
    bar_lookup: &BarLookup,
    state: &mut EventBrowserState,
    track: u16,
) -> Option<JumpRequest> {
    with_pc_cache(doc, track, |doc, events| {
        let (page_start, _) = event_table_section(
            ui,
            state,
            "eb_pc",
            t!("event_browser.title.program_change").as_ref(),
            events,
            &[
                ("#", 40.0),
                (t!("event_browser.header.tick").as_ref(), 70.0),
                (t!("event_browser.header.position").as_ref(), 80.0),
                (t!("event_browser.header.program").as_ref(), 50.0),
            ],
            |e| e.tick,
            |i, row, click_key, state, e, page_ticks, page_start| {
                cell_row_header(
                    row,
                    state,
                    "eb_pc_edit",
                    i,
                    page_start,
                    e.tick,
                    page_ticks,
                    click_key,
                );
                cell_editable(
                    row,
                    "eb_pc_edit",
                    i,
                    format!("{}", e.tick),
                    EditRequest::PcTick { tick: e.tick },
                    click_key,
                );
                cell_position(
                    row,
                    bar_lookup,
                    "eb_pc_edit",
                    i,
                    e.tick,
                    |nt| EditRequest::PcTick { tick: nt },
                    click_key,
                );
                cell_editable(
                    row,
                    "eb_pc_edit",
                    i,
                    format!("{}", e.program),
                    EditRequest::PcProgram { tick: e.tick },
                    click_key,
                );
            },
        );
        apply_pc_popups(ui, doc, "eb_pc_edit", track, bar_lookup);
        apply_pc_ops(ui, doc, state, "eb_pc_edit", track);
        // PC：切到所在 track，仅跳转不闪烁
        take_row_click(ui, "eb_pc").map(|i| JumpRequest {
            tick: events[page_start + i].tick,
            note: Some((track, 0)),
        })
    })
}
