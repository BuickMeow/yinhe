//! Automation 详情（CC / PitchBend / RPN / NRPN / Tempo）。

use super::*;

// ── Automation（统一 CC/PB/RPN/NRPN/Tempo） ──

pub(super) fn show_automation_detail(
    ui: &mut egui::Ui,
    doc: &mut Document,
    bar_lookup: &BarLookup,
    state: &mut EventBrowserState,
    track: u16,
    target: &AutomationTarget,
) -> Option<JumpRequest> {
    // Automation：仅跳转不闪烁；Tempo 不切 track（note=None）
    let note = if matches!(target, AutomationTarget::Tempo) {
        None
    } else {
        Some((track, 0))
    };
    with_automation_cache(doc, track, target, |doc, events, lane_idx| {
        let (page_start, _) = event_table_section(
            ui,
            state,
            "eb_auto",
            &target.display_name(),
            events,
            &[
                ("#", 40.0),
                (t!("event_browser.header.tick").as_ref(), 70.0),
                (t!("event_browser.header.position").as_ref(), 80.0),
                (t!("event_browser.header.value").as_ref(), 60.0),
                ("X1", 50.0),
                ("Y1", 50.0),
                ("X2", 50.0),
                ("Y2", 50.0),
                (t!("event_browser.header.shape").as_ref(), 90.0),
            ],
            |e| e.tick,
            |i, row, click_key, state, e, page_ticks, page_start| {
                cell_row_header(
                    row,
                    state,
                    "eb_auto_edit",
                    i,
                    page_start,
                    e.tick,
                    page_ticks,
                    click_key,
                );
                cell_editable(
                    row,
                    "eb_auto_edit",
                    i,
                    format!("{}", e.tick),
                    EditRequest::AutoTick {
                        tick: e.tick,
                        value: e.value,
                    },
                    click_key,
                );
                cell_position(
                    row,
                    bar_lookup,
                    "eb_auto_edit",
                    i,
                    e.tick,
                    |nt| EditRequest::AutoTick {
                        tick: nt,
                        value: e.value,
                    },
                    click_key,
                );
                cell_editable(
                    row,
                    "eb_auto_edit",
                    i,
                    crate::piano_view::automation_panel::format_display_value(target, e.value),
                    EditRequest::AutoValue {
                        tick: e.tick,
                        value: e.value,
                    },
                    click_key,
                );
                let [x1, y1, x2, y2] = curve_points_text(e.shape);
                cell_text(row, x1, click_key, i);
                cell_text(row, y1, click_key, i);
                cell_text(row, x2, click_key, i);
                cell_text(row, y2, click_key, i);
                cell_editable(
                    row,
                    "eb_auto_edit",
                    i,
                    shape_text(e.shape),
                    EditRequest::AutoShape {
                        tick: e.tick,
                        shape: e.shape,
                    },
                    click_key,
                );
            },
        );

        apply_automation_popups(ui, doc, "eb_auto_edit", track, lane_idx, target, bar_lookup);
        apply_automation_ops(ui, doc, state, "eb_auto_edit", track, target);

        take_row_click(ui, "eb_auto").map(|i| JumpRequest {
            tick: events[page_start + i].tick,
            note,
        })
    })
}
