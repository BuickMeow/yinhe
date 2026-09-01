//! 薄壳：和弦指示器已下沉至 yinhe-editor-core::chord::indicator_text，此处仅 re-export 保持调用方不变。
pub use yinhe_editor_core::chord::indicator_text as chord_indicator_text;

#[cfg(test)]
mod tests {
    use super::*;

    fn chord_tick(doc: &yinhe_editor_core::document::Document, tick: f64) -> Option<String> {
        let main = doc.edit.main_track();
        chord_indicator_text(
            &std::collections::HashMap::new(),
            true,
            Some(tick),
            Some(doc.model()),
            main,
            &doc.edit.track_pianoroll_visible,
            &doc.edit.track_overrides,
            doc.edit.conductor_track_idx,
        )
    }

    fn add_chord(
        doc: &mut yinhe_editor_core::document::Document,
        track: u16,
        keys: &[u8],
        tick: u32,
        vel: u8,
    ) {
        for &key in keys {
            let ev = yinhe_core::NoteEvent {
                id: 0,
                start_tick: tick,
                end_tick: tick + 480,
                key,
                velocity: vel,
            };
            doc.add_note(track, ev).expect("add_note");
        }
    }

    #[test]
    fn chord_per_track_prefers_main_over_visible_when_same_pcs() {
        let mut doc = yinhe_editor_core::document::Document::empty();
        doc.edit.track_selected.clear();
        doc.edit.track_selected.insert(1);
        add_chord(&mut doc, 1, &[60, 64, 67], 0, 100);
        add_chord(&mut doc, 2, &[67, 71, 74], 0, 100);
        let chord = chord_tick(&doc, 0.0).expect("should recognize");
        assert_eq!(chord, "C");
    }

    #[test]
    fn chord_picks_most_complete_across_tracks() {
        let mut doc = yinhe_editor_core::document::Document::empty();
        doc.edit.track_selected.clear();
        doc.edit.track_selected.insert(1);
        add_chord(&mut doc, 1, &[60, 64, 67], 0, 100);
        add_chord(&mut doc, 2, &[60, 64, 67, 71], 0, 100);
        let chord = chord_tick(&doc, 0.0).expect("should recognize");
        assert_eq!(chord, "Cmaj7");
    }

    #[test]
    fn chord_skips_muted_track() {
        let mut doc = yinhe_editor_core::document::Document::empty();
        doc.edit.track_selected.clear();
        doc.edit.track_selected.insert(1);
        add_chord(&mut doc, 1, &[60, 64, 67], 0, 100);
        add_chord(&mut doc, 2, &[60, 64, 67, 71], 0, 100);
        doc.edit.track_overrides[2].muted = true;
        let chord = chord_tick(&doc, 0.0).expect("should fallback");
        assert_eq!(chord, "C");
    }

    #[test]
    fn chord_skips_invisible_track() {
        let mut doc = yinhe_editor_core::document::Document::empty();
        doc.edit.track_selected.clear();
        doc.edit.track_selected.insert(1);
        add_chord(&mut doc, 1, &[60, 64, 67], 0, 100);
        add_chord(&mut doc, 2, &[60, 64, 67, 71], 0, 100);
        doc.edit.track_pianoroll_visible[2] = false;
        let chord = chord_tick(&doc, 0.0).expect("should fallback");
        assert_eq!(chord, "C");
    }

    #[test]
    fn chord_skips_velocity_one_notes() {
        let mut doc = yinhe_editor_core::document::Document::empty();
        doc.edit.track_selected.clear();
        doc.edit.track_selected.insert(1);
        add_chord(&mut doc, 1, &[60, 64, 67], 0, 100);
        add_chord(&mut doc, 2, &[60, 64, 67, 71], 0, 1);
        let chord = chord_tick(&doc, 0.0).expect("should pick audible");
        assert_eq!(chord, "C");
    }

    #[test]
    fn chord_per_track_anti_garbage_global_span_would_fail() {
        let mut doc = yinhe_editor_core::document::Document::empty();
        doc.edit.track_selected.clear();
        doc.edit.track_selected.insert(1);
        add_chord(&mut doc, 1, &[36, 40, 43], 0, 100);
        add_chord(&mut doc, 2, &[84, 88, 91], 0, 100);
        let chord = chord_tick(&doc, 0.0).expect("per-track should still recognize");
        assert_eq!(chord, "C");
    }

    #[test]
    fn chord_prefers_multi_over_single() {
        let mut doc = yinhe_editor_core::document::Document::empty();
        doc.edit.track_selected.clear();
        doc.edit.track_selected.insert(1);
        add_chord(&mut doc, 1, &[60], 0, 100);
        add_chord(&mut doc, 2, &[60, 64, 67], 0, 100);
        let chord = chord_tick(&doc, 0.0).expect("should prefer multi");
        assert_eq!(chord, "C");
    }
}
