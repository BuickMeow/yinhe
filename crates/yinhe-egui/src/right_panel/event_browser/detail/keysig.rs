//! 调号事件详情。

use super::*;

// ── KeySig ──

pub(super) fn show_keysig_detail(
    ui: &mut egui::Ui,
    doc: &mut Document,
    bar_lookup: &BarLookup,
    state: &mut EventBrowserState,
) -> Option<JumpRequest> {
    // 先 clone 出 owned 数据，避免不可变借用阻塞后续 &mut doc 编辑
    let mut sorted: Vec<yinhe_types::KeySigEvent> = doc.data.model.conductor.key_sig.clone();
    sorted.sort_by_key(|e| e.tick);
    let (page_start, _) = event_table_section(
        ui,
        state,
        "eb_ks",
        t!("event_browser.title.keysig").as_ref(),
        &sorted,
        &[
            ("#", 40.0),
            (t!("event_browser.header.tick").as_ref(), 70.0),
            (t!("event_browser.header.position").as_ref(), 80.0),
            (t!("event_browser.header.keysig").as_ref(), 100.0),
            (t!("event_browser.header.root").as_ref(), 60.0),
            (t!("event_browser.header.scale").as_ref(), 80.0),
        ],
        |e| e.tick,
        |i, row, click_key, state, e, page_ticks, page_start| {
            cell_row_header(
                row,
                state,
                "eb_ks_edit",
                i,
                page_start,
                e.tick,
                page_ticks,
                click_key,
            );
            cell_editable(
                row,
                "eb_ks_edit",
                i,
                format!("{}", e.tick),
                EditRequest::KeySigTick { tick: e.tick },
                click_key,
            );
            cell_position(
                row,
                bar_lookup,
                "eb_ks_edit",
                i,
                e.tick,
                |nt| EditRequest::KeySigTick { tick: nt },
                click_key,
            );
            cell_text(row, keysig_text(e.root, e.scale), click_key, i);
            cell_editable(
                row,
                "eb_ks_edit",
                i,
                format!("{} ({})", ROOT_NAMES[e.root as usize % 12], e.root),
                EditRequest::KeySigRoot { tick: e.tick },
                click_key,
            );
            cell_editable(
                row,
                "eb_ks_edit",
                i,
                e.scale.display_name(),
                EditRequest::KeySigScale { tick: e.tick },
                click_key,
            );
        },
    );
    apply_keysig_popups(ui, doc, "eb_ks_edit", bar_lookup);
    apply_keysig_ops(ui, doc, state, "eb_ks_edit");
    take_row_click(ui, "eb_ks").map(|i| JumpRequest {
        tick: sorted[page_start + i].tick,
        note: None,
    })
}

/// 12 个 pitch class 的显示名（0=C, 1=C#/Db, ..., 11=B）。
const ROOT_NAMES: [&str; 12] = [
    "C", "C#/Db", "D", "D#/Eb", "E", "F", "F#/Gb", "G", "G#/Ab", "A", "A#/Bb", "B",
];

/// 调号文本：根音名 + 音阶名（如 "D 多利亚"）。
fn keysig_text(root: u8, scale: yinhe_types::ScaleType) -> String {
    let name = ROOT_NAMES[root as usize % 12];
    format!("{} {}", name, scale.display_name())
}
