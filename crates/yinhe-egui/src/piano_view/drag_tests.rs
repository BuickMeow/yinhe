use super::*;
use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_test_helpers::make_midi;

/// 构造测试用的钢琴卷帘视图：1px/tick、无滚动、key 高 10px。
fn test_view() -> yinhe_types::PianoRollView {
    yinhe_types::PianoRollView {
        base: yinhe_types::TimelineViewBase {
            pixels_per_tick: 1.0,
            scroll_x: 0.0,
            scroll_y: 0.0,
            left_panel_width: 0.0,
            dirty: false,
            track_panel_row_height: 40.0,
            track_panel_scroll_y: 0.0,
            follow_target: None,
            follow_anim_start: 0.0,
            follow_anim_elapsed: 0.0,
        },
        key_height: 10.0,
        viewport_h: 0.0,
        orientation: yinhe_types::Orientation::Horizontal,
    }
}

fn content() -> egui::Rect {
    egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0))
}

/// 取浮动工具条上的一点（用 compute_bar_rect 计算得到，避免硬编码坐标）。
fn bar_point(view: &yinhe_types::PianoRollView) -> egui::Pos2 {
    let eff = [(0.0f64, 100.0f64, 60u8, 70u8)];
    let pixel_rect = crate::selection::drag::music_sel_to_pixel_rect(
        view, eff[0].0, eff[0].1, eff[0].2, eff[0].3,
    );
    let bar = crate::widgets::selection_actions::compute_bar_rect(content(), pixel_rect)
        .expect("bar 应显示");
    bar.center()
}

#[test]
fn click_on_action_bar_does_not_move_playhead() {
    // 回归测试：点击浮动工具条（曾两次导致 playhead 意外跳转）
    let view = test_view();
    let eff = [(0.0, 100.0, 60, 70)];
    let pos = bar_point(&view);
    assert!(
        on_action_bar(pos, content(), &view, &eff, 6),
        "测试前提：该点应在工具条上"
    );
    let result = cursor_tick_from_click(
        pos,
        content(),
        content(),
        &view,
        &eff,
        QuantizePreset::Fraction(1, 4),
        480,
        None,
    );
    assert_eq!(result, None, "点在工具条上时不得移动播放指示器");
}

#[test]
fn click_outside_bar_moves_playhead() {
    let view = test_view();
    let eff = [(0.0, 100.0, 60, 70)];
    // 选框左侧远处、仍在 music_rect 内的点
    let pos = egui::pos2(200.0, 300.0);
    assert!(!on_action_bar(pos, content(), &view, &eff, 6));
    let result = cursor_tick_from_click(
        pos,
        content(),
        content(),
        &view,
        &eff,
        QuantizePreset::Fraction(1, 4),
        480,
        None,
    );
    assert!(result.is_some(), "工具条外的点击应正常定位");
}

#[test]
fn click_outside_music_rect_returns_none() {
    let view = test_view();
    let eff = [(0.0, 100.0, 60, 70)];
    let pos = egui::pos2(100.0, 700.0); // 超出 music_rect 下边界
    let result = cursor_tick_from_click(
        pos,
        content(),
        content(),
        &view,
        &eff,
        QuantizePreset::Fraction(1, 4),
        480,
        None,
    );
    assert_eq!(result, None);
}

/// 跑一帧 sel_drag_frame（Select 工具）。
/// 返回 (note_event, preview_reqs, pencil_drag)，供双击写音符/单音符伸缩测试断言。
#[allow(clippy::too_many_arguments)]
fn run_sel_frame(
    ctx: &egui::Context,
    raw: egui::RawInput,
    view: &mut yinhe_types::PianoRollView,
    midi: &dyn yinhe_types::NoteSource,
    selected: &mut yinhe_core::Selection,
    cursor_tick: &mut Option<f64>,
    note_drag_delta: &mut Option<(i64, i32, bool)>,
    note_resize_delta: &mut Option<(yinhe_editor_core::ResizeSide, i64)>,
    sel_rect: &mut yinhe_editor_core::edit_state::SelRectState,
    track_selected: &std::collections::HashSet<u16>,
    write_track: Option<u16>,
) -> (
    Option<(yinhe_core::NoteEvent, u16)>,
    Vec<crate::piano_view::PreviewReq>,
    Option<yinhe_types::PencilNoteDrag>,
) {
    let mut out: (
        Option<(yinhe_core::NoteEvent, u16)>,
        Vec<crate::piano_view::PreviewReq>,
        Option<yinhe_types::PencilNoteDrag>,
    ) = (None, Vec::new(), None);
    // run_ui 返回的 FullOutput 含字体纹理 delta，丢弃前必须 clear（epaint 断言）。
    ctx.run_ui(raw, |ui| {
        let (_, _, previews, note_event, pencil_drag, _) = sel_drag_frame(
            ui,
            content(),
            content(),
            view,
            Some(midi),
            selected,
            QuantizePreset::Fraction(1, 16),
            480,
            None,
            10000.0,
            cursor_tick,
            note_drag_delta,
            note_resize_delta,
            sel_rect,
            &[[0.5, 0.5, 0.5, 1.0]],
            &[true],
            track_selected,
            write_track,
            None,
            false,
            yinhe_editor_core::audio_settings::QuickDeleteMode::Off,
            None,
        );
        out = (note_event, previews, pencil_drag);
    })
    .textures_delta
    .clear();
    out
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

/// 回归测试：移动音符后松开鼠标不得让演奏指示线跳到释放位置。
/// （release 帧 note_drag_origin 已被清 None，曾导致 marquee 的 simple-click
/// 路径把 cursor_tick 设为释放点，playhead 错误跳转。）
#[test]
fn release_after_note_move_does_not_move_playhead() {
    let ctx = egui::Context::default();
    let mut view = test_view();
    // 模拟已初始化的视口（viewport_h==0 时 clamp_scroll 会触发首次初始化，
    // 重算 key_height/scroll_y，干扰本测试的坐标假设）。
    view.viewport_h = 600.0;
    let midi = make_midi(vec![(100, 0, 480, 0, 100)]);
    // 选框覆盖音符 (tick 0..480, key 100)。key 100 → y = (127-100)*10 = 270。
    let mut selected = yinhe_core::Selection::default();
    selected.add_rect_track(0, 480, 100, 100, 0, 0);
    let mut sel_rect = yinhe_editor_core::edit_state::SelRectState::default();
    sel_rect.push_rect((0.0, 480.0, 100, 100), false);
    let mut cursor_tick: Option<f64> = None;
    let mut note_drag_delta: Option<(i64, i32, bool)> = None;
    let mut note_resize_delta: Option<(yinhe_editor_core::ResizeSide, i64)> = None;

    // 音符中间按下 → 拖到 tick 360（1/16 网格：间隔 120）→ 松开。
    let press = egui::pos2(240.0, 275.0);
    let release = egui::pos2(360.0, 275.0);
    let _ = run_sel_frame(
        &ctx,
        press_event(press),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &std::collections::HashSet::new(),
        Some(0),
    );
    let _ = run_sel_frame(
        &ctx,
        drag_event(release),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &std::collections::HashSet::new(),
        Some(0),
    );
    let _ = run_sel_frame(
        &ctx,
        release_event(release),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &std::collections::HashSet::new(),
        Some(0),
    );

    assert_eq!(
        note_drag_delta,
        Some((120, 0, false)),
        "音符应移动 +120 tick"
    );
    assert_eq!(cursor_tick, None, "移动后松开不得把 playhead 跳到释放位置");
}

/// 双击空白处 → 创建音符（选择工具）。
#[test]
fn double_click_creates_note() {
    let ctx = egui::Context::default();
    let mut view = test_view();
    view.viewport_h = 600.0;
    let midi = make_midi(vec![]);
    let mut selected = yinhe_core::Selection::default();
    let mut sel_rect = yinhe_editor_core::edit_state::SelRectState::default();
    let mut cursor_tick: Option<f64> = None;
    let mut note_drag_delta: Option<(i64, i32, bool)> = None;
    let mut note_resize_delta: Option<(yinhe_editor_core::ResizeSide, i64)> = None;

    // 双击位置：tick 360（1/16 网格 480×4/16=120 的网格点）、key 90 → y = (127-90)*10 + 5 = 375。
    let pos = egui::pos2(360.0, 375.0);
    let _ = run_sel_frame(
        &ctx,
        press_event(pos),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &std::collections::HashSet::new(),
        Some(0),
    );
    let _ = run_sel_frame(
        &ctx,
        release_event(pos),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &std::collections::HashSet::new(),
        Some(0),
    );
    let _ = run_sel_frame(
        &ctx,
        press_event(pos),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &std::collections::HashSet::new(),
        Some(0),
    );
    // egui 在第二击 release 帧判定 double-click。
    let (note_event, previews, _) = run_sel_frame(
        &ctx,
        release_event(pos),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &std::collections::HashSet::new(),
        Some(0),
    );

    let (note, track) = note_event.expect("双击空白应创建音符");
    assert_eq!(track, 0);
    assert_eq!(note.start_tick, 360, "起点按量化 snap");
    assert_eq!(note.end_tick, 480, "长度 = 一个量化间隔");
    assert_eq!(note.key, 90);
    assert!(
        matches!(
            previews.first(),
            Some(crate::piano_view::PreviewReq::Note(_))
        ),
        "双击创建应触发听觉预览"
    );
}

/// 双击已有音符的位置 → 不创建（保持选择工具行为）。
#[test]
fn double_click_on_existing_note_does_not_create() {
    let ctx = egui::Context::default();
    let mut view = test_view();
    view.viewport_h = 600.0;
    let midi = make_midi(vec![(90, 300, 330, 0, 100)]);
    let mut selected = yinhe_core::Selection::default();
    let mut sel_rect = yinhe_editor_core::edit_state::SelRectState::default();
    let mut cursor_tick: Option<f64> = None;
    let mut note_drag_delta: Option<(i64, i32, bool)> = None;
    let mut note_resize_delta: Option<(yinhe_editor_core::ResizeSide, i64)> = None;

    // 音符 (tick 300..330, key 90)：中心点 (315, 375)。
    let pos = egui::pos2(315.0, 375.0);
    let _ = run_sel_frame(
        &ctx,
        press_event(pos),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &std::collections::HashSet::new(),
        Some(0),
    );
    let _ = run_sel_frame(
        &ctx,
        release_event(pos),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &std::collections::HashSet::new(),
        Some(0),
    );
    let _ = run_sel_frame(
        &ctx,
        press_event(pos),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &std::collections::HashSet::new(),
        Some(0),
    );
    // egui 在第二击 release 帧判定 double-click。
    let (note_event, _, _) = run_sel_frame(
        &ctx,
        release_event(pos),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &std::collections::HashSet::new(),
        Some(0),
    );

    assert!(note_event.is_none(), "双击已有音符不得创建新音符");
}

/// 无选中音轨时双击：PR 编辑目标 = None（不再回退到第一个非 Conductor 轨），
/// 不得创建音符。
#[test]
fn double_click_without_selection_creates_nothing() {
    let ctx = egui::Context::default();
    let mut view = test_view();
    view.viewport_h = 600.0;
    let midi = make_midi(vec![]);
    let mut selected = yinhe_core::Selection::default();
    let mut sel_rect = yinhe_editor_core::edit_state::SelRectState::default();
    let mut cursor_tick: Option<f64> = None;
    let mut note_drag_delta: Option<(i64, i32, bool)> = None;
    let mut note_resize_delta: Option<(yinhe_editor_core::ResizeSide, i64)> = None;

    let pos = egui::pos2(300.0, 375.0);
    let _ = run_sel_frame(
        &ctx,
        press_event(pos),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &std::collections::HashSet::new(),
        None,
    );
    let _ = run_sel_frame(
        &ctx,
        release_event(pos),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &std::collections::HashSet::new(),
        None,
    );
    let _ = run_sel_frame(
        &ctx,
        press_event(pos),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &std::collections::HashSet::new(),
        None,
    );
    // egui 在第二击 release 帧判定 double-click。
    let (note_event, _, _) = run_sel_frame(
        &ctx,
        release_event(pos),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &std::collections::HashSet::new(),
        None,
    );

    assert!(note_event.is_none(), "无选中音轨时双击不得创建音符");
}

/// 框选到音符 → 普通选框；框选空区域 → 自动变垂直选框（全 128 键）。
#[test]
fn empty_marquee_becomes_vertical_selection() {
    let ctx = egui::Context::default();
    let mut view = test_view();
    view.viewport_h = 600.0;
    // 音符在 key 100（tick 100..200），框选 key 85..95 区域 → 无音符。
    let midi = make_midi(vec![(100, 100, 200, 0, 100)]);
    let mut selected = yinhe_core::Selection::default();
    let mut sel_rect = yinhe_editor_core::edit_state::SelRectState::default();
    let mut cursor_tick: Option<f64> = None;
    let mut note_drag_delta: Option<(i64, i32, bool)> = None;
    let mut note_resize_delta: Option<(yinhe_editor_core::ResizeSide, i64)> = None;

    // key 85 → y=(127-85)*10=420；key 95 → y=(127-95)*10=320。
    let start = egui::pos2(50.0, 420.0);
    let end = egui::pos2(150.0, 320.0);
    let _ = run_sel_frame(
        &ctx,
        press_event(start),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &std::collections::HashSet::new(),
        None,
    );
    let _ = run_sel_frame(
        &ctx,
        drag_event(end),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &std::collections::HashSet::new(),
        None,
    );
    let _ = run_sel_frame(
        &ctx,
        release_event(end),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &std::collections::HashSet::new(),
        None,
    );

    assert_eq!(sel_rect.rects.len(), 1, "应有一个选框");
    let (t0, t1, kl, kh) = sel_rect.rects[0];
    assert_eq!((kl, kh), (0, 127), "空区域框选应变为全 128 键垂直选框");
    assert!(t0 < t1);
    assert!(
        sel_rect.has_auto_vertical(),
        "自动切换的垂直选框应打标记（拖动锁定上下）"
    );
    // 选中范围也应覆盖全键。
    assert!(
        selected.rects.iter().any(|r| r.2 == 0 && r.3 == 127),
        "selected 应包含全键范围"
    );
}

/// 回归测试：空区域框选自动生成的垂直选框拖动时上下锁定（与 SelectVertical 一致）。
/// （曾 bug：普通 Select 的垂直选框仍可上下移动，破坏全键 0..127 语义。）
#[test]
fn vertical_marquee_drag_locks_key() {
    let ctx = egui::Context::default();
    let mut view = test_view();
    view.viewport_h = 600.0;
    let midi = make_midi(vec![]);
    let mut selected = yinhe_core::Selection::default();
    let mut sel_rect = yinhe_editor_core::edit_state::SelRectState::default();
    // 自动生成的垂直选框（全键 0..127，tick 0..480）——空区域框选场景。
    sel_rect.push_rect((0.0, 480.0, 0, 127), true);
    let mut cursor_tick: Option<f64> = None;
    let mut note_drag_delta: Option<(i64, i32, bool)> = None;
    let mut note_resize_delta: Option<(yinhe_editor_core::ResizeSide, i64)> = None;

    // key 89 → y=(127-89)*10=380：选框内按下，拖到 y=480（key 79，即向下 10 键）。
    let press = egui::pos2(300.0, 380.0);
    let end = egui::pos2(420.0, 480.0);
    let _ = run_sel_frame(
        &ctx,
        press_event(press),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &std::collections::HashSet::new(),
        Some(0),
    );
    let _ = run_sel_frame(
        &ctx,
        drag_event(end),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &std::collections::HashSet::new(),
        Some(0),
    );
    let _ = run_sel_frame(
        &ctx,
        release_event(end),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &std::collections::HashSet::new(),
        Some(0),
    );

    assert_eq!(
        note_drag_delta,
        Some((120, 0, false)),
        "垂直选框只能水平移动，dk 必须为 0"
    );
    assert_eq!(
        sel_rect.rects,
        vec![(120.0, 600.0, 0, 127)],
        "垂直选框移动后仍须保持全键 0..127"
    );
}

/// 回归测试：用户手动框选出的全键选框（0..127）仍可上下移动——
/// 只有空区域框选自动切换的垂直选框才锁定上下。
#[test]
fn manual_full_key_marquee_can_move_vertically() {
    let ctx = egui::Context::default();
    let mut view = test_view();
    view.viewport_h = 600.0;
    let midi = make_midi(vec![]);
    let mut selected = yinhe_core::Selection::default();
    let mut sel_rect = yinhe_editor_core::edit_state::SelRectState::default();
    // 手动全键选框（0..127，无自动垂直标记）。
    sel_rect.push_rect((0.0, 480.0, 0, 127), false);
    let mut cursor_tick: Option<f64> = None;
    let mut note_drag_delta: Option<(i64, i32, bool)> = None;
    let mut note_resize_delta: Option<(yinhe_editor_core::ResizeSide, i64)> = None;

    // key 89 → y=(127-89)*10=380：选框内按下，拖到 y=480（key 79，向下 10 键）。
    let press = egui::pos2(300.0, 380.0);
    let end = egui::pos2(420.0, 480.0);
    let _ = run_sel_frame(
        &ctx,
        press_event(press),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &std::collections::HashSet::new(),
        Some(0),
    );
    let _ = run_sel_frame(
        &ctx,
        drag_event(end),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &std::collections::HashSet::new(),
        Some(0),
    );
    let _ = run_sel_frame(
        &ctx,
        release_event(end),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &std::collections::HashSet::new(),
        Some(0),
    );

    assert_eq!(
        note_drag_delta,
        Some((120, -10, false)),
        "手动全键选框应保留上下移动权利"
    );
    assert_eq!(
        sel_rect.rects,
        vec![(120.0, 600.0, 0, 117)],
        "手动全键选框上下移动后 key 范围随之偏移"
    );
}

/// 框选到音符 → 保持普通选框（不垂直化）。
#[test]
fn marquee_with_notes_stays_rectangular() {
    let ctx = egui::Context::default();
    let mut view = test_view();
    view.viewport_h = 600.0;
    // key 90（tick 100..200）在框选范围内。
    let midi = make_midi(vec![(90, 100, 200, 0, 100)]);
    let mut selected = yinhe_core::Selection::default();
    let mut sel_rect = yinhe_editor_core::edit_state::SelRectState::default();
    let mut cursor_tick: Option<f64> = None;
    let mut note_drag_delta: Option<(i64, i32, bool)> = None;
    let mut note_resize_delta: Option<(yinhe_editor_core::ResizeSide, i64)> = None;

    // key 85..95 区域框选（key 90 音符在内）。
    let start = egui::pos2(50.0, 420.0);
    let end = egui::pos2(150.0, 320.0);
    let _ = run_sel_frame(
        &ctx,
        press_event(start),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &std::collections::HashSet::new(),
        None,
    );
    let _ = run_sel_frame(
        &ctx,
        drag_event(end),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &std::collections::HashSet::new(),
        None,
    );
    let _ = run_sel_frame(
        &ctx,
        release_event(end),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &std::collections::HashSet::new(),
        None,
    );

    assert_eq!(sel_rect.rects.len(), 1);
    let (_, _, kl, kh) = sel_rect.rects[0];
    assert!(
        kl >= 85 && kh <= 95,
        "有音符的选框应保持矩形范围，实际 kl={kl} kh={kh}"
    );
}

/// 单音符边缘伸缩（不用先选中）：press 音符右边缘 → 拖 → release 提交。
#[test]
fn select_tool_resizes_single_note_without_selection() {
    let ctx = egui::Context::default();
    let mut view = test_view();
    view.viewport_h = 600.0;
    // 音符 (tick 300..330, key 90)：右边缘 x=330，key 90 → y=375。
    let midi = make_midi(vec![(90, 300, 330, 0, 100)]);
    let mut selected = yinhe_core::Selection::default();
    let mut sel_rect = yinhe_editor_core::edit_state::SelRectState::default();
    let mut cursor_tick: Option<f64> = None;
    let mut note_drag_delta: Option<(i64, i32, bool)> = None;
    let mut note_resize_delta: Option<(yinhe_editor_core::ResizeSide, i64)> = None;

    let press = egui::pos2(330.0, 375.0);
    let release = egui::pos2(360.0, 375.0);
    let _ = run_sel_frame(
        &ctx,
        press_event(press),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &std::collections::HashSet::new(),
        Some(0),
    );
    let _ = run_sel_frame(
        &ctx,
        drag_event(release),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &std::collections::HashSet::new(),
        Some(0),
    );
    let (_, _, pencil_drag) = run_sel_frame(
        &ctx,
        release_event(release),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &std::collections::HashSet::new(),
        Some(0),
    );

    assert!(
        matches!(
            pencil_drag,
            Some(yinhe_types::PencilNoteDrag::ResizeRight {
                track: 0,
                start_tick: 300,
                key: 90,
                new_end_tick: 360,
            })
        ),
        "音符右边缘应从 330 伸到 360，实际 {pencil_drag:?}"
    );
    assert_eq!(note_drag_delta, None, "未选中时按音符边缘不得启动选区移动");
    assert!(selected.is_empty(), "选区不应被修改");
    assert!(sel_rect.is_empty(), "选框不应被修改");
}

/// 单音符左边缘伸缩（不用先选中）。
#[test]
fn select_tool_resizes_single_note_left_edge() {
    let ctx = egui::Context::default();
    let mut view = test_view();
    view.viewport_h = 600.0;
    // 音符 (tick 300..480, key 90)：左边缘 x=300，拖到 x=240（1/16 网格点）。
    let midi = make_midi(vec![(90, 300, 480, 0, 100)]);
    let mut selected = yinhe_core::Selection::default();
    let mut sel_rect = yinhe_editor_core::edit_state::SelRectState::default();
    let mut cursor_tick: Option<f64> = None;
    let mut note_drag_delta: Option<(i64, i32, bool)> = None;
    let mut note_resize_delta: Option<(yinhe_editor_core::ResizeSide, i64)> = None;

    let press = egui::pos2(300.0, 375.0);
    let release = egui::pos2(240.0, 375.0);
    let _ = run_sel_frame(
        &ctx,
        press_event(press),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &std::collections::HashSet::new(),
        Some(0),
    );
    let _ = run_sel_frame(
        &ctx,
        drag_event(release),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &std::collections::HashSet::new(),
        Some(0),
    );
    let (_, _, pencil_drag) = run_sel_frame(
        &ctx,
        release_event(release),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &std::collections::HashSet::new(),
        Some(0),
    );

    assert!(
        matches!(
            pencil_drag,
            Some(yinhe_types::PencilNoteDrag::ResizeLeft {
                track: 0,
                start_tick: 300,
                key: 90,
                new_start_tick: 240,
            })
        ),
        "音符左边缘应从 300 缩到 240，实际 {pencil_drag:?}"
    );
    assert_eq!(cursor_tick, None, "伸缩后松开不得把 playhead 跳到释放位置");
}

/// 回归：未选中音轨（write_track=None）时，选框工具不得移动音符。
/// （之前 write_track 无选中时回退到第一个非 Conductor 轨，单音符移动仍可用。）
#[test]
fn select_tool_moves_nothing_without_selected_track() {
    let ctx = egui::Context::default();
    let mut view = test_view();
    view.viewport_h = 600.0;
    let midi = make_midi(vec![(90, 300, 330, 0, 100)]);
    let mut selected = yinhe_core::Selection::default();
    let mut sel_rect = yinhe_editor_core::edit_state::SelRectState::default();
    let mut cursor_tick: Option<f64> = None;
    let mut note_drag_delta: Option<(i64, i32, bool)> = None;
    let mut note_resize_delta: Option<(yinhe_editor_core::ResizeSide, i64)> = None;

    let press = egui::pos2(315.0, 375.0);
    let release = egui::pos2(435.0, 375.0);
    let _ = run_sel_frame(
        &ctx,
        press_event(press),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &std::collections::HashSet::new(),
        None,
    );
    let _ = run_sel_frame(
        &ctx,
        drag_event(release),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &std::collections::HashSet::new(),
        None,
    );
    let (_, _, pencil_drag) = run_sel_frame(
        &ctx,
        release_event(release),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &std::collections::HashSet::new(),
        None,
    );

    assert!(
        pencil_drag.is_none(),
        "未选中音轨时不得移动音符，实际 {pencil_drag:?}"
    );
    assert_eq!(note_drag_delta, None, "未选中音轨时不得启动选区移动");
}

/// 单音符移动（有选中音轨，不用先选中音符）：press 音符中部 → 拖 → release 提交。
#[test]
fn select_tool_moves_single_note_without_selection() {
    let ctx = egui::Context::default();
    let mut view = test_view();
    view.viewport_h = 600.0;
    // 音符 (tick 300..330, key 90)：中心 (315, 375)，无任何选框。
    let midi = make_midi(vec![(90, 300, 330, 0, 100)]);
    let mut selected = yinhe_core::Selection::default();
    let mut sel_rect = yinhe_editor_core::edit_state::SelRectState::default();
    let mut cursor_tick: Option<f64> = None;
    let mut note_drag_delta: Option<(i64, i32, bool)> = None;
    let mut note_resize_delta: Option<(yinhe_editor_core::ResizeSide, i64)> = None;

    // press tick 315 → snap 360；release tick 435 → snap 480：dt = +120。
    let press = egui::pos2(315.0, 375.0);
    let release = egui::pos2(435.0, 375.0);
    let _ = run_sel_frame(
        &ctx,
        press_event(press),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &std::collections::HashSet::new(),
        Some(0),
    );
    let _ = run_sel_frame(
        &ctx,
        drag_event(release),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &std::collections::HashSet::new(),
        Some(0),
    );
    let (_, _, pencil_drag) = run_sel_frame(
        &ctx,
        release_event(release),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &std::collections::HashSet::new(),
        Some(0),
    );

    assert!(
        matches!(
            pencil_drag,
            Some(yinhe_types::PencilNoteDrag::Move {
                track: 0,
                start_tick: 300,
                key: 90,
                delta_ticks: 120,
                delta_keys: 0,
            })
        ),
        "未选中音符应直接移动 +120 tick，实际 {pencil_drag:?}"
    );
    assert_eq!(note_drag_delta, None, "不得启动选区移动");
    assert!(selected.is_empty(), "选区不应被修改");
    assert!(sel_rect.is_empty(), "选框不应被修改");
}

/// bug 回归：框选作用域 = track_selected（只选中 track 5 时框选只作用于它）。
#[test]
fn marquee_respects_track_selected() {
    let ctx = egui::Context::default();
    let mut view = test_view();
    view.viewport_h = 600.0;
    // track 0 和 track 5 在框选区域内都有音符。
    let midi = make_midi(vec![(90, 100, 200, 0, 100), (90, 100, 200, 5, 100)]);
    let mut selected = yinhe_core::Selection::default();
    let mut sel_rect = yinhe_editor_core::edit_state::SelRectState::default();
    let mut cursor_tick: Option<f64> = None;
    let mut note_drag_delta: Option<(i64, i32, bool)> = None;
    let mut note_resize_delta: Option<(yinhe_editor_core::ResizeSide, i64)> = None;

    // 框选 tick 50..250、key 85..95 区域（两个音符都在内）。
    let start = egui::pos2(50.0, 420.0);
    let end = egui::pos2(250.0, 320.0);
    let _ = run_sel_frame(
        &ctx,
        press_event(start),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &[5u16].into_iter().collect(),
        None,
    );
    let _ = run_sel_frame(
        &ctx,
        drag_event(end),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &[5u16].into_iter().collect(),
        None,
    );
    let _ = run_sel_frame(
        &ctx,
        release_event(end),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &[5u16].into_iter().collect(),
        None,
    );

    assert_eq!(selected.rects.len(), 1, "框选应只产生一个选区 rect");
    let (_, _, _, _, tl, th) = selected.rects[0];
    assert_eq!((tl, th), (5, 5), "框选应只作用于选中音轨 5");
    assert!(selected.contains(5, 100, 90), "选中音轨音符应被选中");
    assert!(!selected.contains(0, 100, 90), "未选中音轨的音符不得被选中");
}

/// 回归测试：按住 Alt 拖动未选中的单音符 → 走复制通道（note_drag_delta.alt == true），
/// 而不是普通移动。配合 document.duplicate_selected_to 的 revision bump（Bug 7）
/// 保证副本可见（Bug 9 的根因是复制后 GPU 缓存不失效，导致看起来'拖不动'）。
#[test]
fn select_tool_alt_copies_single_note() {
    let ctx = egui::Context::default();
    let mut view = test_view();
    view.viewport_h = 600.0;
    let midi = make_midi(vec![(90, 300, 330, 0, 100)]);
    let mut selected = yinhe_core::Selection::default();
    let mut sel_rect = yinhe_editor_core::edit_state::SelRectState::default();
    let mut cursor_tick: Option<f64> = None;
    let mut note_drag_delta: Option<(i64, i32, bool)> = None;
    let mut note_resize_delta: Option<(yinhe_editor_core::ResizeSide, i64)> = None;

    let mods = egui::Modifiers {
        alt: true,
        ..Default::default()
    };
    let press_evt = |pos: egui::Pos2| {
        let mut raw = egui::RawInput::default();
        raw.events.push(egui::Event::ModifiersChanged(mods));
        raw.events.push(egui::Event::PointerMoved(pos));
        raw.events.push(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: mods,
        });
        raw
    };

    let press = egui::pos2(315.0, 375.0);
    let release = egui::pos2(435.0, 375.0);
    let _ = run_sel_frame(
        &ctx,
        press_evt(press),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &std::collections::HashSet::new(),
        Some(0),
    );
    let _ = run_sel_frame(
        &ctx,
        drag_event(release),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &std::collections::HashSet::new(),
        Some(0),
    );
    let (_, _, pencil_drag) = run_sel_frame(
        &ctx,
        release_event(release),
        &mut view,
        &midi,
        &mut selected,
        &mut cursor_tick,
        &mut note_drag_delta,
        &mut note_resize_delta,
        &mut sel_rect,
        &std::collections::HashSet::new(),
        Some(0),
    );

    // Alt+单音符拖动 → 复制通道（alt=true），并清除原选区、把该音符置为唯一选中；
    // 不得走铅笔移动通道（PencilNoteDrag）。
    assert_eq!(
        note_drag_delta,
        Some((120, 0, true)),
        "Alt 单音符拖动应走复制通道 (dt=120, dk=0, alt=true)"
    );
    assert!(pencil_drag.is_none(), "Alt 复制不得走铅笔移动通道");
}

/// 回归测试：选框拖拽状态机进行中（含 Alt 克隆）时 sel_drag_in_progress 必须为 true。
/// effective_tool 据此锁定选择工具——否则 Alt 克隆拖出音符原位后 hover 命中失败，
/// 会被临时切成铅笔工具、中断本次拖拽。
#[test]
fn sel_drag_in_progress_reflects_persisted_state() {
    let ctx = egui::Context::default();
    let raw = egui::RawInput::default();

    // run_ui 返回的 FullOutput 含字体纹理 delta，丢弃前必须 clear（epaint 断言）。
    let run = |raw: egui::RawInput, f: &mut dyn FnMut(&egui::Ui)| {
        let mut out = ctx.run_ui(raw, |ui| f(ui));
        out.textures_delta.clear();
    };

    // 无拖拽状态 → false
    let mut result = true;
    run(raw.clone(), &mut |ui| {
        result = sel_drag_in_progress(ui);
    });
    assert!(!result, "无拖拽状态时应为 false");

    // 写入"单音符移动（Alt 克隆）"持久化状态（跨帧保持）→ true
    run(raw.clone(), &mut |ui| {
        ui.data_mut(|d| {
            d.insert_persisted(
                ui.id().with("sel_note_move_state"),
                Some((0u16, 300u32, 90u8, 330u32, 300.0f64, 0i32, true)),
            )
        });
    });
    run(raw.clone(), &mut |ui| {
        result = sel_drag_in_progress(ui);
    });
    assert!(result, "Alt 克隆拖拽进行中应为 true");

    // 写入"选区整体移动（Alt 克隆）"持久化状态 → 同样为 true
    run(raw.clone(), &mut |ui| {
        ui.data_mut(|d| {
            d.insert_persisted(
                ui.id().with("sel_note_move_state"),
                Option::<(u16, u32, u8, u32, f64, i32, bool)>::None,
            );
            d.insert_persisted(
                ui.id().with("note_drag_origin"),
                Some((300.0f64, 90.0f64, true)),
            );
        });
    });
    run(raw, &mut |ui| {
        result = sel_drag_in_progress(ui);
    });
    assert!(result, "选区 Alt 克隆拖拽进行中应为 true");
}

/// 跑一帧锚点线工具（直线/剪刀共用状态机）。
fn run_anchor_frame(
    ctx: &egui::Context,
    raw: egui::RawInput,
    view: &mut yinhe_types::PianoRollView,
    line: &mut Option<yinhe_editor_core::edit_state::AnchorLine>,
    clear_on_click: bool,
    id_suffix: &'static str,
) {
    // run_ui 返回的 FullOutput 含字体纹理 delta，丢弃前必须 clear（epaint 断言）。
    ctx.run_ui(raw, |ui| {
        crate::piano_view::anchor_line::frame(
            ui,
            content(),
            content(),
            view,
            line,
            QuantizePreset::Fraction(1, 16),
            480,
            None,
            10000.0,
            id_suffix,
            clear_on_click,
        );
    })
    .textures_delta
    .clear();
}

/// 直线工具：拖出两个锚点（x 吸附量化、y 取行中心）后线保留，终点锚点可再拖。
#[test]
fn line_drag_creates_anchor_line_and_anchor_drags() {
    let ctx = egui::Context::default();
    let mut view = test_view();
    // 预初始化视口，避免 clamp_scroll 首次初始化重算 key_height/scroll。
    view.viewport_h = 600.0;
    let mut line = None;

    // (100,300) → key 97；(400,280) → key 99；x 吸附 120/360。
    run_anchor_frame(
        &ctx,
        press_event(egui::pos2(100.0, 300.0)),
        &mut view,
        &mut line,
        false,
        "line_tool_drag",
    );
    run_anchor_frame(
        &ctx,
        drag_event(egui::pos2(400.0, 280.0)),
        &mut view,
        &mut line,
        false,
        "line_tool_drag",
    );
    run_anchor_frame(
        &ctx,
        release_event(egui::pos2(400.0, 280.0)),
        &mut view,
        &mut line,
        false,
        "line_tool_drag",
    );
    assert_eq!(
        line,
        Some(yinhe_editor_core::edit_state::AnchorLine {
            start: (120.0, 97),
            end: (360.0, 99),
        }),
        "拖拽后线保留且锚点吸附"
    );

    // 终点锚点（key 99 行中心 y=285）拖到 (480,290) → key 98。
    run_anchor_frame(
        &ctx,
        press_event(egui::pos2(360.0, 285.0)),
        &mut view,
        &mut line,
        false,
        "line_tool_drag",
    );
    run_anchor_frame(
        &ctx,
        drag_event(egui::pos2(480.0, 290.0)),
        &mut view,
        &mut line,
        false,
        "line_tool_drag",
    );
    run_anchor_frame(
        &ctx,
        release_event(egui::pos2(480.0, 290.0)),
        &mut view,
        &mut line,
        false,
        "line_tool_drag",
    );
    assert_eq!(
        line,
        Some(yinhe_editor_core::edit_state::AnchorLine {
            start: (120.0, 97),
            end: (480.0, 98),
        }),
        "终点锚点应可再拖动"
    );
}

/// 剪刀：单击空白清空线；直线：单击保留单点线。
#[test]
fn scissors_click_clears_line_but_line_tool_keeps_it() {
    let ctx = egui::Context::default();
    let mut view = test_view();
    view.viewport_h = 600.0;
    let existing = Some(yinhe_editor_core::edit_state::AnchorLine {
        start: (120.0, 97),
        end: (360.0, 99),
    });
    let pos = egui::pos2(600.0, 300.0);

    // 剪刀：空白单击（按下、原地松开）→ 清空。
    let mut line = existing;
    run_anchor_frame(
        &ctx,
        press_event(pos),
        &mut view,
        &mut line,
        true,
        "scissors_drag",
    );
    run_anchor_frame(
        &ctx,
        release_event(pos),
        &mut view,
        &mut line,
        true,
        "scissors_drag",
    );
    assert!(line.is_none(), "剪刀单击空白应清空线");

    // 直线：同样操作保留单点线（✓ 时生成一个音符）。
    let mut line = None;
    run_anchor_frame(
        &ctx,
        press_event(pos),
        &mut view,
        &mut line,
        false,
        "line_tool_drag",
    );
    run_anchor_frame(
        &ctx,
        release_event(pos),
        &mut view,
        &mut line,
        false,
        "line_tool_drag",
    );
    assert_eq!(
        line,
        Some(yinhe_editor_core::edit_state::AnchorLine {
            start: (600.0, 97),
            end: (600.0, 97),
        }),
        "直线单击保留单点线"
    );
}

/// 跑一帧刷子，返回 (ghost 数, release 事件)。
fn run_brush_frame(
    ctx: &egui::Context,
    raw: egui::RawInput,
    view: &mut yinhe_types::PianoRollView,
) -> (usize, Option<crate::piano_view::PianoViewEvent>) {
    let mut out = (0usize, None);
    ctx.run_ui(raw, |ui| {
        let (ghosts, event) = crate::piano_view::brush::brush_frame(
            ui,
            content(),
            content(),
            view,
            Some(0),
            &[true],
            None,
            QuantizePreset::Fraction(1, 16),
            480,
            10000.0,
        );
        out = (ghosts.len(), event);
    })
    .textures_delta
    .clear();
    out
}

/// 刷子：按住拖动按量化格落音符（Bresenham 补格），松手批量提交。
#[test]
fn brush_drag_emits_grid_notes_on_release() {
    let ctx = egui::Context::default();
    let mut view = test_view();
    view.viewport_h = 600.0;

    // 1/16 = 120 tick：press (100,300) → 格 1（start 120）；drag (350,300) → 格 3（start 360）。
    let (ghosts, event) = run_brush_frame(&ctx, press_event(egui::pos2(100.0, 300.0)), &mut view);
    assert_eq!(ghosts, 1, "按下即预览 1 格");
    assert!(event.is_none());

    let (ghosts, event) = run_brush_frame(&ctx, drag_event(egui::pos2(350.0, 300.0)), &mut view);
    assert_eq!(ghosts, 3, "路径填充 3 格（120/240/360）");
    assert!(event.is_none());

    let (_, event) = run_brush_frame(&ctx, release_event(egui::pos2(350.0, 300.0)), &mut view);
    match event {
        Some(crate::piano_view::PianoViewEvent::AddNotes { track, notes }) => {
            assert_eq!(track, 0);
            assert_eq!(notes.len(), 3);
            assert_eq!(notes[0].start_tick, 120);
            assert_eq!(notes[0].end_tick, 240);
            assert_eq!(notes[0].key, 97);
            assert_eq!(notes[2].start_tick, 360);
        }
        _ => panic!("刷子松手应产生 AddNotes"),
    }
}

/// 跑一帧 grid_frame（网格工具）。
fn run_grid_frame(
    ctx: &egui::Context,
    raw: egui::RawInput,
    view: &mut yinhe_types::PianoRollView,
    selected: &mut yinhe_core::Selection,
    sel_rect: &mut yinhe_editor_core::edit_state::SelRectState,
    track_selected: &std::collections::HashSet<u16>,
) {
    ctx.run_ui(raw, |ui| {
        crate::piano_view::grid::grid_frame(
            ui,
            content(),
            content(),
            view,
            selected,
            sel_rect,
            QuantizePreset::Fraction(1, 16),
            480,
            None,
            10000.0,
            track_selected,
        );
    })
    .textures_delta
    .clear();
}

/// 网格工具：框选 release 后选框提交到 SelRectState 与 Selection。
#[test]
fn grid_marquee_release_commits_rect() {
    let ctx = egui::Context::default();
    let mut view = test_view();
    // 预初始化视口，避免 clamp_scroll 首次初始化重算 key_height/scroll。
    view.viewport_h = 600.0;
    let mut selected = yinhe_core::Selection::default();
    let mut sel_rect = yinhe_editor_core::edit_state::SelRectState::default();
    let empty = std::collections::HashSet::new();

    // (100,300) → key 97；(400,280) → key 99；x 按 1/16 吸附：120 / 360。
    run_grid_frame(
        &ctx,
        press_event(egui::pos2(100.0, 300.0)),
        &mut view,
        &mut selected,
        &mut sel_rect,
        &empty,
    );
    run_grid_frame(
        &ctx,
        drag_event(egui::pos2(400.0, 280.0)),
        &mut view,
        &mut selected,
        &mut sel_rect,
        &empty,
    );
    run_grid_frame(
        &ctx,
        release_event(egui::pos2(400.0, 280.0)),
        &mut view,
        &mut selected,
        &mut sel_rect,
        &empty,
    );

    assert_eq!(
        sel_rect.rects,
        vec![(120.0, 360.0, 97, 99)],
        "选框应为吸附后的矩形"
    );
    assert_eq!(selected.rects.len(), 1, "Selection 同步（track 空 = 全部）");
    let (ts, te, kl, kh, tlo, thi) = selected.rects[0];
    assert_eq!((ts, te, kl, kh), (120, 360, 97, 99));
    assert_eq!((tlo, thi), (0, u16::MAX));
}

/// 网格工具：拖动已提交选框内部 → 整体平移（按量化吸附）。
#[test]
fn grid_move_drag_offsets_committed_rect() {
    let ctx = egui::Context::default();
    let mut view = test_view();
    // 预初始化视口，避免 clamp_scroll 首次初始化重算 key_height/scroll。
    view.viewport_h = 600.0;
    let mut selected = yinhe_core::Selection::default();
    let mut sel_rect = yinhe_editor_core::edit_state::SelRectState::default();
    sel_rect.rects.push((120.0, 360.0, 97, 99));
    let empty = std::collections::HashSet::new();

    // 框像素范围：x 120..360，y 280..310（key99 顶到 key97 底）。
    // 从框内 (240,300) 拖到 (360,290)：dt=+120 tick，dk=+1。
    run_grid_frame(
        &ctx,
        press_event(egui::pos2(240.0, 300.0)),
        &mut view,
        &mut selected,
        &mut sel_rect,
        &empty,
    );
    run_grid_frame(
        &ctx,
        drag_event(egui::pos2(360.0, 290.0)),
        &mut view,
        &mut selected,
        &mut sel_rect,
        &empty,
    );
    run_grid_frame(
        &ctx,
        release_event(egui::pos2(360.0, 290.0)),
        &mut view,
        &mut selected,
        &mut sel_rect,
        &empty,
    );

    assert_eq!(
        sel_rect.rects,
        vec![(240.0, 480.0, 98, 100)],
        "选框应整体平移 dt=120, dk=1"
    );
}

/// 网格工具：在选框外按下 → 清空旧选框（随后 marquee 以 <3px 结束不产生新框）。
#[test]
fn grid_press_blank_clears_rect() {
    let ctx = egui::Context::default();
    let mut view = test_view();
    // 预初始化视口，避免 clamp_scroll 首次初始化重算 key_height/scroll。
    view.viewport_h = 600.0;
    let mut selected = yinhe_core::Selection::default();
    let mut sel_rect = yinhe_editor_core::edit_state::SelRectState::default();
    sel_rect.rects.push((120.0, 360.0, 97, 99));
    let empty = std::collections::HashSet::new();

    let pos = egui::pos2(600.0, 300.0);
    run_grid_frame(
        &ctx,
        press_event(pos),
        &mut view,
        &mut selected,
        &mut sel_rect,
        &empty,
    );
    run_grid_frame(
        &ctx,
        release_event(pos),
        &mut view,
        &mut selected,
        &mut sel_rect,
        &empty,
    );

    assert!(sel_rect.rects.is_empty(), "空白按下应清空旧选框");
    assert!(selected.rects.is_empty(), "选区同步清空");
}

/// 回归测试：框选物化后选区平移到落点，第二次拖动收集不得把落点处的
/// 路人音符（同 key 同 tick 区间）收编进拖动组。
/// （修复前 `Selection` 是纯矩形，移动提交 `offset` 后矩形覆盖到路人，
/// 再次按下拖动就把路人一起搬走。）
#[test]
fn marquee_members_not_hijacked_by_bystanders_after_move() {
    let ctx = egui::Context::default();
    let mut view = test_view();
    view.viewport_h = 600.0;
    // A: key 90, tick 100..200, v100（被框选）；B: key 90, tick 300..400, v80（落点路人）
    let midi = make_midi(vec![(90, 100, 200, 0, 100), (90, 300, 400, 0, 80)]);
    let mut selected = yinhe_core::Selection::default();
    let mut sel_rect = yinhe_editor_core::edit_state::SelRectState::default();
    let mut cursor_tick: Option<f64> = None;
    let mut note_drag_delta: Option<(i64, i32, bool)> = None;
    let mut note_resize_delta: Option<(yinhe_editor_core::ResizeSide, i64)> = None;

    // 从空白（tick 10）拖到 tick 250：选框 [0,240) 覆盖 A，press 点不命中音符。
    // key 90 → y ∈ [370,380]（music_rect 高 600）。
    let start = egui::pos2(10.0, 375.0);
    let end = egui::pos2(250.0, 375.0);
    for raw in [press_event(start), drag_event(end), release_event(end)] {
        let _ = run_sel_frame(
            &ctx,
            raw,
            &mut view,
            &midi,
            &mut selected,
            &mut cursor_tick,
            &mut note_drag_delta,
            &mut note_resize_delta,
            &mut sel_rect,
            &std::collections::HashSet::new(),
            None,
        );
    }
    assert_eq!(
        selected.explicit_member_count(),
        Some(1),
        "PR 框选提交应物化成员位图（恰好命中 A）"
    );

    // 模拟第一次移动提交：选区矩形跟随到落点（+200 tick）。
    selected.offset(200, 0);
    // 移动后的模型：A 移到 300..400（id 不变、v100），B 仍在 300..400。
    let moved_midi = make_midi(vec![(90, 300, 400, 0, 100), (90, 300, 400, 0, 80)]);

    let collected = crate::selection::drag::collect_selected_notes(
        &selected,
        Some(&moved_midi as &dyn yinhe_types::NoteSource),
        &[true],
        &std::collections::HashSet::new(),
    );
    assert_eq!(collected.len(), 1, "第二次拖动只能收集成员音符");
    assert_eq!(
        collected[0].velocity, 100,
        "收集到的必须是被框选的 A（v100），不能是落点路人 B（v80）"
    );
}

/// 回归：拖动 ghost 的视口裁剪必须与渲染层同源（方向感知 API + 内容区尺寸）。
/// 曾误用 `visible_tick_range(music_rect.width())` / `visible_cross_range(...height())`
/// —— music_rect 比 content_rect 少一个面板宽度，对应 tick 被当作视口外，
/// 缩小视图时 ghost 被大量误裁（表现为"拖动没有预览/ghost 不动"）。
#[test]
fn note_drag_ghost_survives_viewport_cull() {
    let ctx = egui::Context::default();
    let mut view = test_view();
    // 左侧键盘/轨道面板宽 200：music_rect 比 content_rect 窄（旧代码在此暴露）。
    view.base.left_panel_width = 200.0;
    let content_rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1400.0, 600.0));
    let music_rect = egui::Rect::from_min_size(egui::pos2(200.0, 0.0), egui::vec2(1200.0, 600.0));
    // 音符在 tick 1100（> 旧代码的 1000 视口右界，< 新代码的 1200）。
    let midi = make_midi(vec![(70, 1100, 1400, 0, 100)]);
    let mut selected = yinhe_core::Selection::default();
    selected.add_rect_track(0, 2000, 0, 127, 0, 0);
    let mut sel_rect = yinhe_editor_core::edit_state::SelRectState::default();
    sel_rect.push_rect((0.0, 2000.0, 0, 127), false);
    let mut cursor_tick = None;
    let mut note_drag_delta = None;
    let mut note_resize_delta = None;
    let track_selected = std::collections::HashSet::new();

    // 屏幕坐标：tick 1100 → x = 200 + 1100 = 1300；key 70 → y = (127-70)*10 = 570。
    let press = egui::pos2(1300.0, 570.0);
    // 拖到视口内（避免触发 auto-scroll）：tick 1080→1050，位移 -30。
    let drag = egui::pos2(1250.0, 570.0);
    let (ghost_count, hidden_count);
    let mut run = |raw: egui::RawInput, inject: bool| {
        let mut out = (0usize, 0usize);
        ctx.run_ui(raw, |ui| {
            let (ghosts, hidden, _, _, _, _) = sel_drag_frame(
                ui,
                content_rect,
                music_rect,
                &mut view,
                Some(&midi as &dyn yinhe_types::NoteSource),
                &mut selected,
                QuantizePreset::Fraction(1, 16),
                480,
                None,
                10000.0,
                &mut cursor_tick,
                &mut note_drag_delta,
                &mut note_resize_delta,
                &mut sel_rect,
                &[[0.5, 0.5, 0.5, 1.0]],
                &[true],
                &track_selected,
                // write_track = Some → can_edit，走选区拖动分支（drag_notes 非空）。
                Some(0),
                None,
                false,
                yinhe_editor_core::audio_settings::QuickDeleteMode::Off,
                None,
            );
            out = (ghosts.len(), hidden.len());
            if inject {
                // 在 sel_drag_frame 之后注入拖拽状态（覆盖本帧 save 的空状态）：
                // 绕开 press 分支细节，聚焦被测点——拖动帧的 ghost 视口裁剪。
                ui.data_mut(|d| {
                    d.insert_persisted(
                        ui.id().with("note_drag_origin"),
                        Some((1100.0f64, 70.0f64, false)),
                    );
                    d.insert_persisted(
                        ui.id().with("drag_notes"),
                        Some(std::sync::Arc::new(vec![
                            crate::selection::drag::CollectedNote {
                                track: 0,
                                start_tick: 1100,
                                end_tick: 1400,
                                key: 70,
                                velocity: 100,
                            },
                        ])),
                    );
                });
            }
        })
        .textures_delta
        .clear();
        out
    };
    // 帧 1：press 建立 pointer down 状态 + 注入拖拽状态。
    let _ = run(press_event(press), true);
    // 帧 2：拖动 → 产出 ghost/hidden。
    (ghost_count, hidden_count) = run(drag_event(drag), false);
    assert!(
        ghost_count > 0,
        "拖动帧必须产生 ghost 预览（视口裁剪不得误裁面板宽度对应的 tick）"
    );
    assert!(hidden_count > 0, "拖动帧必须隐藏原音符");
}
