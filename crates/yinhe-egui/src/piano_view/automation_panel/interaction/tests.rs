use super::*;
use crate::widgets::tools_panel::Tool;
use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_types::{AutomationEdit, AutomationEvent, AutomationTarget, SegmentShape};

fn tempo_panel() -> AutomationPanelView {
    AutomationPanelView {
        selected_target: AutomationTarget::Tempo,
        show_velocity: false,
        panel_height: 80.0,
        value_zoom: 1.0,
        value_scroll: 0.0,
        base: yinhe_types::TimelineViewBase {
            pixels_per_tick: 1.0,
            scroll_x: 0.0,
            scroll_y: 0.0,
            left_panel_width: 0.0,
            dirty: true,
            track_panel_row_height: 40.0,
            track_panel_scroll_y: 0.0,
            follow_target: None,
            follow_anim_start: 0.0,
            follow_anim_elapsed: 0.0,
        },
        ..Default::default()
    }
}

fn tempo_lane(events: Vec<(u32, f32)>) -> AutomationLane {
    AutomationLane {
        target: AutomationTarget::Tempo,
        track: 0,
        events: events
            .into_iter()
            .map(|(tick, value)| AutomationEvent {
                tick,
                value,
                shape: SegmentShape::Step,
            })
            .collect(),
    }
}

fn edit_ctx() -> AutomationEditCtx<'static> {
    AutomationEditCtx {
        active_tool: Tool::Pencil,
        active_track: Some(0),
        quantize: QuantizePreset::Fraction(1, 16),
        ppq: 480,
        bar_line_data: None,
    }
}

fn panel_rect() -> egui::Rect {
    egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 80.0))
}

fn run_frame(
    ctx: &egui::Context,
    raw: egui::RawInput,
    panel: &AutomationPanelView,
    lane: &AutomationLane,
) -> Vec<AutomationEdit> {
    let mut edits = Vec::new();
    ctx.run_ui(raw, |ui| {
        let mut info: Option<InfoContent> = None;
        let mut right_tab: Option<RightTab> = None;
        edits = handle_automation_interaction(
            ui,
            panel_rect(),
            panel_rect(),
            panel,
            &[],
            Some(lane),
            0,
            &edit_ctx(),
            ui.id().with(0),
            &[[0.8, 0.8, 0.8, 1.0]],
            &mut info,
            &mut right_tab,
            false,
        )
        .0;
    })
    .textures_delta
    .clear();
    edits
}

fn press_event(pos: egui::Pos2) -> egui::RawInput {
    let mut raw = egui::RawInput::default();
    raw.events.push(egui::Event::PointerMoved(pos));
    raw.events.push(egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::default(),
    });
    raw
}

fn edit_ctx_tool(tool: Tool) -> AutomationEditCtx<'static> {
    AutomationEditCtx {
        active_tool: tool,
        active_track: Some(0),
        quantize: QuantizePreset::Fraction(1, 16),
        ppq: 480,
        bar_line_data: None,
    }
}

fn run_frame_full(
    ctx: &egui::Context,
    raw: egui::RawInput,
    panel: &AutomationPanelView,
    lane: &AutomationLane,
    tool: Tool,
) -> (Vec<AutomationEdit>, Option<egui::Rect>, Option<SelOp>) {
    let (mut edits, mut marquee, mut sel_op) = (Vec::new(), None, None);
    ctx.run_ui(raw, |ui| {
        let mut info: Option<InfoContent> = None;
        let mut right_tab: Option<RightTab> = None;
        let (e, _g, _di, _hi, m, so) = handle_automation_interaction(
            ui,
            panel_rect(),
            panel_rect(),
            panel,
            &[],
            Some(lane),
            0,
            &edit_ctx_tool(tool),
            ui.id().with(0),
            &[[0.8, 0.8, 0.8, 1.0]],
            &mut info,
            &mut right_tab,
            false,
        );
        edits = e;
        marquee = m;
        sel_op = so;
    })
    .textures_delta
    .clear();
    (edits, marquee, sel_op)
}

fn drag_event(pos: egui::Pos2) -> egui::RawInput {
    let mut raw = egui::RawInput::default();
    raw.events.push(egui::Event::PointerMoved(pos));
    raw
}

fn release_event(pos: egui::Pos2) -> egui::RawInput {
    let mut raw = egui::RawInput::default();
    raw.events.push(egui::Event::PointerMoved(pos));
    raw.events.push(egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::default(),
    });
    raw
}

#[test]
fn tempo_anchor_drag_above_panel_exceeds_display_max() {
    let ctx = egui::Context::default();
    let panel = tempo_panel();
    let lane = tempo_lane(vec![(0, 120.0)]);
    let anchor = egui::pos2(0.0, 0.0);
    let above = egui::pos2(120.0, -20.0);
    let _ = run_frame(&ctx, press_event(anchor), &panel, &lane);
    let _ = run_frame(&ctx, drag_event(above), &panel, &lane);
    let edits = run_frame(&ctx, release_event(above), &panel, &lane);
    let move_edit = edits
        .iter()
        .find(|e| matches!(e, AutomationEdit::Move { .. }))
        .expect("拖拽应提交 Move");
    match move_edit {
        AutomationEdit::Move {
            old_tick,
            new_tick,
            new_value,
            ..
        } => {
            assert_eq!(*old_tick, 0);
            assert_eq!(*new_tick, 120);
            assert!(
                *new_value > 120.0,
                "BPM 应突破显示上限 120，实际 {new_value}"
            );
            assert_eq!(*new_value, 150.0);
        }
        _ => unreachable!(),
    }
}

#[test]
fn select_tool_marquee_is_vertical() {
    let ctx = egui::Context::default();
    let panel = tempo_panel();
    let lane = tempo_lane(vec![(0, 120.0)]);
    let start = egui::pos2(100.0, 10.0);
    let end = egui::pos2(300.0, 70.0);
    let _ = run_frame_full(&ctx, press_event(start), &panel, &lane, Tool::Select);
    let (_, marquee, _) = run_frame_full(&ctx, drag_event(end), &panel, &lane, Tool::Select);
    let (_, _, sel_op) = run_frame_full(&ctx, release_event(end), &panel, &lane, Tool::Select);
    let rect = marquee.expect("拖拽中应产生 marquee_rect");
    assert_eq!(rect.min.y, 0.0);
    assert_eq!(rect.max.y, 80.0);
    assert_eq!(rect.min.x, 100.0);
    assert_eq!(rect.max.x, 300.0);
    match sel_op {
        Some(SelOp::Set(SelRectOp::Set(r))) => {
            assert_eq!(r.tick_start, 100.0);
            assert_eq!(r.tick_end, 300.0);
            assert!(r.value_range.is_none());
        }
        other => panic!("期望 Set(Set(vertical rect))，实际 {other:?}"),
    }
}

#[test]
fn select_tool_double_click_anchor_deletes() {
    let ctx = egui::Context::default();
    let panel = tempo_panel();
    let lane = tempo_lane(vec![(120, 120.0)]);
    let pos = egui::pos2(120.0, 0.0);
    let _ = run_frame_full(&ctx, press_event(pos), &panel, &lane, Tool::Select);
    let _ = run_frame_full(&ctx, release_event(pos), &panel, &lane, Tool::Select);
    let _ = run_frame_full(&ctx, press_event(pos), &panel, &lane, Tool::Select);
    let (edits, _, _) = run_frame_full(&ctx, release_event(pos), &panel, &lane, Tool::Select);
    assert!(
        edits
            .iter()
            .any(|e| matches!(e, AutomationEdit::Delete { tick: 120, .. })),
        "选择工具双击锚点应删除，实际 {edits:?}"
    );
}
