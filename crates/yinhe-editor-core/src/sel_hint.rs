use yinhe_types::time_format::format_tick_bar_beat_with_time_sig;
use yinhe_types::{AnchorSelRect, AutomationTarget};

use crate::batch_ops::summarize_selected;
use crate::document::Document;

/// 讲解行的选框统计（layout.rs 每帧计算，视图命中选框后显示）。
#[derive(Clone)]
pub struct SelHintInfo {
    /// 选中音符数（PR/AR）或事件数（AM）。
    pub count: u64,
    /// 时间跨度（bar.beat.tick→bar.beat.tick）。
    pub span: String,
}

fn selected_am_events(
    doc: &Document,
    panel: &yinhe_types::AutomationPanelView,
    rects: &[AnchorSelRect],
) -> Vec<(u32, f32)> {
    let target = &panel.selected_target;
    let events: Vec<(u32, f32)> = if matches!(target, AutomationTarget::Tempo) {
        doc.data
            .model
            .conductor
            .tempo
            .events
            .iter()
            .map(|e| (e.tick, e.value))
            .collect()
    } else {
        let Some(track) = doc
            .edit
            .main_track()
            .and_then(|i| doc.data.model.tracks.get(i as usize))
        else {
            return Vec::new();
        };
        let Some(lane) = track.automation_lanes.iter().find(|l| l.target == *target) else {
            return Vec::new();
        };
        lane.events.iter().map(|e| (e.tick, e.value)).collect()
    };
    events
        .into_iter()
        .filter(|(t, v)| rects.iter().any(|r| r.contains(*t, *v)))
        .collect()
}

pub fn compute_sel_hint(doc: &Document) -> Option<SelHintInfo> {
    let model = &doc.data.model;
    let ppq = model.meta.ppq;
    let (def_num, def_den) = model.tempo_map.time_sig_default;
    let sig_events = model.tempo_map.time_sig_events.as_slice();
    let fmt = |t: f64| format_tick_bar_beat_with_time_sig(t, ppq, sig_events, def_num, def_den);

    let pr_rects = doc.edit.sel_rect.effective_rects();
    let ar_rects = &doc.edit.arr_sel_rect;
    let am_rects: Vec<&AnchorSelRect> = doc
        .edit
        .controller_panels
        .iter()
        .filter(|p| !p.show_velocity && !p.anchor_sel_rects.is_empty())
        .flat_map(|p| p.anchor_sel_rects.iter())
        .collect();

    if !pr_rects.is_empty() {
        let (t0, t1) = pr_rects.iter().fold(
            (f64::INFINITY, f64::NEG_INFINITY),
            |(a, b), &(ts, te, _, _)| (a.min(ts), b.max(te)),
        );
        Some(SelHintInfo {
            count: summarize_selected(model, &doc.edit.selected).count,
            span: format!("{}→{}", fmt(t0), fmt(t1)),
        })
    } else if !ar_rects.is_empty() {
        let (t0, t1) = ar_rects.iter().fold(
            (f64::INFINITY, f64::NEG_INFINITY),
            |(a, b), &(ts, te, _, _)| (a.min(ts), b.max(te)),
        );
        Some(SelHintInfo {
            count: summarize_selected(model, &doc.edit.selected).count,
            span: format!("{}→{}", fmt(t0), fmt(t1)),
        })
    } else if !am_rects.is_empty() {
        let (t0, t1) = am_rects
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), r| {
                (
                    a.min(r.tick_start.min(r.tick_end)),
                    b.max(r.tick_start.max(r.tick_end)),
                )
            });
        let count: usize = doc
            .edit
            .controller_panels
            .iter()
            .filter(|p| !p.show_velocity && !p.anchor_sel_rects.is_empty())
            .map(|p| selected_am_events(doc, p, &p.anchor_sel_rects).len())
            .sum();
        Some(SelHintInfo {
            count: count as u64,
            span: format!("{}→{}", fmt(t0), fmt(t1)),
        })
    } else {
        None
    }
}
