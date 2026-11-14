//! 通用文本事件详情（Marker / Lyrics / Chord）。

use super::*;

// ── 通用文本事件（Marker / Lyrics / Chord） ──

pub(super) fn show_text_events_detail(
    ui: &mut egui::Ui,
    doc: &mut Document,
    bar_lookup: &BarLookup,
    state: &mut EventBrowserState,
    table_id: &str,
    label: &str,
    kind: TextEventKind,
) -> Option<JumpRequest> {
    with_text_cache(doc, kind, |doc, events| {
        let edit_salt = format!("{}_edit", table_id);
        let (page_start, _) = event_table_section(
            ui,
            state,
            table_id,
            label,
            events,
            &[
                ("#", 40.0),
                (t!("event_browser.header.tick").as_ref(), 70.0),
                (t!("event_browser.header.position").as_ref(), 80.0),
                (label, 200.0),
            ],
            |item| item.0,
            |i, row, click_key, state, item, page_ticks, page_start| {
                let (tick, text) = item;
                cell_row_header(
                    row, state, &edit_salt, i, page_start, *tick, page_ticks, click_key,
                );
                cell_editable(
                    row,
                    &edit_salt,
                    i,
                    format!("{}", tick),
                    EditRequest::TextEventTick { kind, tick: *tick },
                    click_key,
                );
                cell_position(
                    row,
                    bar_lookup,
                    &edit_salt,
                    i,
                    *tick,
                    move |nt| EditRequest::TextEventTick { kind, tick: nt },
                    click_key,
                );
                cell_editable(
                    row,
                    &edit_salt,
                    i,
                    text.clone(),
                    EditRequest::TextEventText { kind, tick: *tick },
                    click_key,
                );
            },
        );
        apply_text_popups(ui, doc, &edit_salt, bar_lookup);
        // 分派删除/插入操作
        match kind {
            TextEventKind::Marker => apply_marker_ops(ui, doc, state, &edit_salt),
            TextEventKind::ConductorLyrics => {
                apply_conductor_lyrics_ops(ui, doc, state, &edit_salt)
            }
            TextEventKind::ConductorChord => apply_conductor_chord_ops(ui, doc, state, &edit_salt),
            TextEventKind::Lyrics { track } => apply_lyrics_ops(ui, doc, state, &edit_salt, track),
            TextEventKind::Chord { track } => apply_chord_ops(ui, doc, state, &edit_salt, track),
        }
        take_row_click(ui, table_id).map(|i| JumpRequest {
            tick: events[page_start + i].0,
            note: None,
        })
    })
}
