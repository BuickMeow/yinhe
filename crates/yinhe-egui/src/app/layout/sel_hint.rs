//! 薄壳：选框提示已下沉至 yinhe-editor-core::sel_hint，此处仅 re-export 保持调用方不变。
pub use yinhe_editor_core::sel_hint::SelHintInfo;

use crate::app::App;
use yinhe_editor_core::document::Document;

impl App {
    pub(crate) fn compute_sel_hint(doc: &Document) -> Option<SelHintInfo> {
        yinhe_editor_core::sel_hint::compute_sel_hint(doc)
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
