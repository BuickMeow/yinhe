//! 事件浏览器入口。
//!
//! 模块结构：
//! - [`state`] — `EventBrowserState` / `SelectedItem` / `JumpRequest`
//! - [`bar_lookup`] — Tick → 拍号位置格式化（支持变拍号）
//! - [`tree`] — 左侧树状导航（project/mapping/Conductor/Port/Channel/Track）
//! - [`detail`] — 右侧详情面板（按 `SelectedItem` 分发）
//! - [`table`] — 表格构建与单元格渲染（含左键跳转 / 右键编辑）
//! - [`edit`] — 右键编辑 popup（value / shape）
//!
//! `SelectedItem::Automation` 统一覆盖 CC / PitchBend / RPN / NRPN / Tempo，
//! 通过 `AutomationTarget` 区分具体类型，不再为每种写单独变体。

mod bar_lookup;
mod detail;
mod edit;
mod edit_ops;
mod state;
mod table;
mod tree;

use eframe::egui;

use yinhe_core::YinModel;
use yinhe_editor_core::document::Document;

use state::ArchiveKey;

// 对外暴露的公共类型（right_panel 通过 `event_browser::` 引用）
pub use state::{EventBrowserState, JumpRequest};

/// 状态维护：revision 变化时清理展开/选中态。返回 model 基本信息。
fn maintain_state(
    doc: &Document,
    state: &mut EventBrowserState,
) -> (u32, u8, Vec<(u32, u8)>, usize) {
    let (ppq, default_num, ts, tracks_len) = {
        let m = &doc.data.model;
        (
            m.meta.ppq,
            m.tempo_map.time_sig_default.0,
            bar_lookup::ts_changes(m),
            m.tracks.len(),
        )
    };
    let fingerprint = doc.data.revision;
    if state.fingerprint != Some(fingerprint) {
        if state.fingerprint.is_none() {
            state.expanded_keys.clear();
            for t in &doc.data.model.tracks {
                state.expanded_keys.insert(ArchiveKey::Port(t.port));
            }
        }
        if let Some(idx) = state.selected_track
            && idx as usize >= tracks_len
        {
            state.selected_track = None;
        }
        state.fingerprint = Some(fingerprint);
    }
    (ppq, default_num, ts, tracks_len)
}

/// 「树图」选项卡：文件夹树视图（事件浏览器的上栏）。
pub fn show_tree(ui: &mut egui::Ui, doc: Option<&mut Document>, state: &mut EventBrowserState) {
    crate::right_panel::region_hint(ui, rust_i18n::t!("hint.panel.event_tree"));
    let Some(doc) = doc else {
        crate::widgets::hint::empty_hint(
            ui,
            "\u{ff08}\u{672a}\u{6253}\u{5f00}\u{6587}\u{6863}\u{ff09}",
        );
        return;
    };
    let _ = maintain_state(doc, state);

    // 不画背景（right_panel 已铺 app_bg，再 fill 叠两层）；边距由 dock 统一施加
    let frame_bg = egui::Frame::NONE;
    let row_h = crate::widgets::control::h(ui.ctx());
    crate::widgets::scroll::rows_scroll_both(ui, "eb_tree", row_h, None, |ui| {
        frame_bg.show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.vertical(|ui| tree::render_tree(ui, doc, state));
        });
    });
}

/// 「事件」选项卡：事件表格 / 详情（事件浏览器的下栏）。返回跳转请求。
pub fn show_events(
    ui: &mut egui::Ui,
    doc: Option<&mut Document>,
    state: &mut EventBrowserState,
) -> Option<JumpRequest> {
    let Some(doc) = doc else {
        crate::widgets::hint::empty_hint(
            ui,
            "\u{ff08}\u{672a}\u{6253}\u{5f00}\u{6587}\u{6863}\u{ff09}",
        );
        return None;
    };
    let (ppq, default_num, ts, _) = maintain_state(doc, state);
    let bar_lookup = bar_lookup::BarLookup::build(ppq, default_num, &ts);

    // 不画背景（right_panel 已铺 app_bg，再 fill 叠两层）；边距由 dock 统一施加
    let frame_bg = egui::Frame::NONE;
    let mut jump_request: Option<JumpRequest> = None;
    egui::ScrollArea::both()
        .id_salt("eb_detail")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            frame_bg.show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                let sel = state.selected_item.clone();
                let track_idx = state.selected_track;
                if let Some(ref sel) = sel {
                    jump_request = detail::show_event_detail(ui, sel, doc, &bar_lookup, state);
                } else if let Some(idx) = track_idx {
                    let model = &doc.data.model;
                    if let Some(track) = model.tracks.get(idx as usize) {
                        detail::show_track_detail(ui, idx, track, model);
                    } else {
                        detail::show_overview(ui, model);
                    }
                } else {
                    let model = &doc.data.model;
                    detail::show_overview(ui, model);
                }
            });
        });
    jump_request
}

// ── 共享 helper（供子模块复用） ──
//
// 这两个函数被 tree.rs 和 detail.rs 同时使用，放在模块根避免重复。
// 子模块通过 `super::port_letter` / `super::group_tracks_by_port_channel` 引用。

fn port_letter(port: u8) -> char {
    if port < 26 {
        (b'A' + port) as char
    } else {
        '?'
    }
}

fn group_tracks_by_port_channel(
    model: &YinModel,
    conductor_idx: Option<u16>,
) -> std::collections::BTreeMap<u8, std::collections::BTreeMap<u8, Vec<u16>>> {
    let mut out: std::collections::BTreeMap<u8, std::collections::BTreeMap<u8, Vec<u16>>> =
        std::collections::BTreeMap::new();
    for (i, t) in model.tracks.iter().enumerate() {
        if Some(i as u16) == conductor_idx {
            continue;
        }
        out.entry(t.port)
            .or_default()
            .entry(t.channel)
            .or_default()
            .push(i as u16);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_letter_basic() {
        assert_eq!(port_letter(0), 'A');
        assert_eq!(port_letter(1), 'B');
        assert_eq!(port_letter(15), 'P');
        assert_eq!(port_letter(25), 'Z');
        assert_eq!(port_letter(26), '?');
        assert_eq!(port_letter(255), '?');
    }

    #[test]
    fn group_tracks_by_port_channel_orders_and_groups() {
        use std::sync::Arc;
        use yinhe_core::{TrackData, YinModel};

        let mut t0 = TrackData::new(0, 0);
        t0.name = "A0c0".into();
        let mut t1 = TrackData::new(0, 1);
        t1.name = "A0c1".into();
        let mut t2 = TrackData::new(1, 0);
        t2.name = "B0c0".into();
        let mut t3 = TrackData::new(0, 0);
        t3.name = "A0c0_dup".into();

        let model = YinModel {
            tracks: vec![Arc::new(t0), Arc::new(t1), Arc::new(t2), Arc::new(t3)],
            ..Default::default()
        };

        let groups = group_tracks_by_port_channel(&model, None);
        assert_eq!(groups.len(), 2);
        let p0 = &groups[&0];
        assert_eq!(p0.len(), 2);
        assert_eq!(p0[&0], vec![0, 3]);
        assert_eq!(p0[&1], vec![1]);
        assert_eq!(groups[&1][&0], vec![2]);
    }
}
