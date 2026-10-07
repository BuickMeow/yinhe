//! 「属性概要」选项卡：当前选中轨的音符数 / 事件数 / 音色。

use eframe::egui;

use yinhe_editor_core::document::Document;

use rust_i18n::t;

/// 「属性概要」选项卡入口。
pub(crate) fn show_summary_panel(ui: &mut egui::Ui, doc: &Document) {
    crate::right_panel::region_hint(ui, t!("hint.panel.summary"));
    let num_tracks = doc.data.model.tracks.len();
    if num_tracks == 0 {
        crate::widgets::hint::empty_hint(ui, t!("track.no_tracks").as_ref());
        return;
    }
    let Some(track_idx) = doc
        .edit
        .track_selected
        .iter()
        .next()
        .copied()
        .map(|i| (i as usize).min(num_tracks - 1))
    else {
        crate::widgets::hint::empty_hint(ui, t!("panel.select_track_hint").as_ref());
        return;
    };
    let ti = doc.edit.track_cache.info.get(track_idx).cloned();
    show_summary(ui, doc, track_idx, ti);
}

/// 属性概要页。
fn show_summary(
    ui: &mut egui::Ui,
    doc: &Document,
    track_idx: usize,
    ti: Option<yinhe_core::TrackInfo>,
) {
    let Some(ti) = ti else { return };
    crate::widgets::rows::value_row(ui, t!("track.note_count"), format!("{}", ti.note_count));
    crate::widgets::rows::value_row(ui, t!("track.event_count"), format!("{}", ti.event_count));
    let global_ch = ti.port as u32 * 16 + (ti.channel as u32 - 1);
    if let Some(pc) = doc.edit.track_cache.pc_map.get(&(global_ch as u8)) {
        crate::widgets::rows::value_row(ui, t!("track.program"), format!("PC {}", pc));
    }
    let _ = track_idx;
}
