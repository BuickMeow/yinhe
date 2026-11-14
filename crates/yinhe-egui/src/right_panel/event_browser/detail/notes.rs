//! 音符事件详情。

use super::*;

// ── Notes ──

pub(super) fn show_notes_detail(
    ui: &mut egui::Ui,
    doc: &mut Document,
    bar_lookup: &BarLookup,
    state: &mut EventBrowserState,
    track: u16,
) -> Option<JumpRequest> {
    // 全轨音符收集 + 排序是 3GB 级开销（1.64 亿场景），按 (doc_id, track,
    // revision) 缓存；任何编辑 bump revision 后自动重建。
    let revision = doc.data.revision;
    let cache_valid = matches!(
        &state.notes_cache,
        Some((d, t, r, _)) if *d == doc.doc_id && *t == track && *r == revision
    );
    if !cache_valid {
        let model = &doc.data.model;
        let track_count = model
            .track_note_count
            .get(track as usize)
            .copied()
            .unwrap_or(0) as usize;
        let mut notes: Vec<yinhe_core::NoteEvent> = Vec::with_capacity(track_count);
        for (key, bucket) in model.notes.iter().enumerate() {
            if !model.bucket_track_stats[key].contains_key(&track) {
                continue;
            }
            for n in bucket.iter().filter(|n| n.track == track) {
                notes.push(yinhe_core::NoteEvent {
                    id: n.id,
                    start_tick: n.start_tick,
                    end_tick: n.end_tick,
                    key: key as u8,
                    velocity: n.velocity,
                });
            }
        }
        notes.sort_by_key(|n| n.start_tick);
        state.notes_cache = Some((doc.doc_id, track, revision, notes));
    }
    // 短暂取出 owned 数据，避免与 `&mut state`（分页/行选择）借用冲突。
    let (cache_doc, cache_track, cache_rev, notes) = state.notes_cache.take()?;
    let (page_start, _) = event_table_section(
        ui,
        state,
        "eb_notes",
        t!("event_browser.title.notes").as_ref(),
        &notes,
        &[
            ("#", 40.0),
            ("id", 70.0),
            (t!("event_browser.header.tick").as_ref(), 70.0),
            (t!("event_browser.header.position").as_ref(), 80.0),
            ("gate", 60.0),
            (t!("event_browser.header.end_tick").as_ref(), 80.0),
            (t!("event_browser.header.end_position").as_ref(), 90.0),
            (t!("event_browser.header.key").as_ref(), 50.0),
            (t!("event_browser.header.velocity").as_ref(), 50.0),
        ],
        |n| n.start_tick,
        |i, row, click_key, state, n, page_ticks, page_start| {
            let note_ref = NoteRef {
                id: n.id,
                start_tick: n.start_tick,
                end_tick: n.end_tick,
                key: n.key,
                velocity: n.velocity,
                track,
            };
            let gate = n.end_tick.saturating_sub(n.start_tick);
            cell_row_header(
                row,
                state,
                "eb_notes_edit",
                i,
                page_start,
                n.start_tick,
                page_ticks,
                click_key,
            );
            cell_text(row, format!("#{}", n.id), click_key, i);
            cell_editable(
                row,
                "eb_notes_edit",
                i,
                format!("{}", n.start_tick),
                EditRequest::NoteStartTick { note: note_ref },
                click_key,
            );
            let nr_start = note_ref;
            cell_position(
                row,
                bar_lookup,
                "eb_notes_edit",
                i,
                n.start_tick,
                move |nt| EditRequest::NoteStartTick {
                    note: NoteRef {
                        start_tick: nt,
                        ..nr_start
                    },
                },
                click_key,
            );
            cell_editable(
                row,
                "eb_notes_edit",
                i,
                format!("{}", gate),
                EditRequest::NoteGate { note: note_ref },
                click_key,
            );
            cell_editable(
                row,
                "eb_notes_edit",
                i,
                format!("{}", n.end_tick),
                EditRequest::NoteEndTick { note: note_ref },
                click_key,
            );
            let nr_end = note_ref;
            cell_position(
                row,
                bar_lookup,
                "eb_notes_edit",
                i,
                n.end_tick,
                move |nt| EditRequest::NoteEndTick {
                    note: NoteRef {
                        end_tick: nt,
                        ..nr_end
                    },
                },
                click_key,
            );
            cell_editable(
                row,
                "eb_notes_edit",
                i,
                format!("{}", n.key),
                EditRequest::NoteKey { note: note_ref },
                click_key,
            );
            cell_editable(
                row,
                "eb_notes_edit",
                i,
                format!("{}", n.velocity),
                EditRequest::NoteVelocity { note: note_ref },
                click_key,
            );
        },
    );
    // 音符：切到音符所在 track
    let jump = take_row_click(ui, "eb_notes").map(|i| {
        let n = &notes[page_start + i];
        JumpRequest {
            tick: n.start_tick,
            note: Some((track, n.key)),
        }
    });
    state.notes_cache = Some((cache_doc, cache_track, cache_rev, notes));
    apply_note_popups(ui, doc, "eb_notes_edit", bar_lookup);
    apply_notes_ops(ui, doc, state, "eb_notes_edit", track);
    jump
}
