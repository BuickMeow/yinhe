#![allow(unused_imports)]
use crate::app::App;
use crate::right_panel::info_panel::selection::selected_am_events;
use yinhe_editor_core::batch_ops::summarize_selected;
use yinhe_types::AnchorSelRect;
use yinhe_types::time_format::format_tick_bar_beat_with_time_sig;

/// 讲解行的选框统计（layout.rs 每帧计算，视图命中选框后显示）。
/// 三视图选框互斥，同一时刻最多只有一个来源为 Some。
#[derive(Clone)]
pub(crate) struct SelHintInfo {
    /// 选中音符数（PR/AR）或事件数（AM）。
    pub count: u64,
    /// 时间跨度（bar.beat.tick→bar.beat.tick）。
    pub span: String,
}

impl App {
    pub(crate) fn compute_sel_hint(
        doc: &yinhe_editor_core::document::Document,
    ) -> Option<SelHintInfo> {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_test_helpers::make_test_document;
    use yinhe_types::{AnchorSelRect, AutomationPanelView, AutomationTarget};

    #[test]
    fn sel_hint_pr_selection() {
        let mut doc = make_test_document();
        doc.edit.sel_rect.rects = vec![(0.0, 960.0, 60, 60)];
        doc.edit
            .selected
            .add_rect_track(0, 960, 60, 60, 0, u16::MAX);

        let hint = App::compute_sel_hint(&doc).expect("PR 选框应生成选框信息");
        assert_eq!(hint.count, 2);
        assert_eq!(hint.span, "1.1.000→1.3.000");
    }

    #[test]
    fn sel_hint_ar_selection() {
        let mut doc = make_test_document();
        doc.edit.arr_sel_rect = vec![(0.0, 960.0, 0, 0)];
        doc.edit
            .selected
            .add_rect_track(0, 960, 60, 60, 0, u16::MAX);

        let hint = App::compute_sel_hint(&doc).expect("AR 选框应生成选框信息");
        assert_eq!(hint.count, 2);
        assert_eq!(hint.span, "1.1.000→1.3.000");
    }

    #[test]
    fn sel_hint_am_selection() {
        let mut doc = make_test_document();
        doc.edit.track_selected = [1u16].into_iter().collect();
        doc.edit.controller_panels.clear();
        doc.edit.controller_panels.push(AutomationPanelView {
            show_velocity: false,
            selected_target: AutomationTarget::CC { controller: 7 },
            anchor_sel_rects: vec![AnchorSelRect {
                tick_start: 0.0,
                tick_end: 240.0,
                value_range: None,
            }],
            ..Default::default()
        });

        let hint = App::compute_sel_hint(&doc).expect("AM 选框应生成选框信息");
        assert_eq!(hint.count, 2);
        assert_eq!(hint.span, "1.1.000→1.1.240");
    }

    #[test]
    fn sel_hint_none_without_selection() {
        let doc = make_test_document();
        assert!(App::compute_sel_hint(&doc).is_none());
    }
}
