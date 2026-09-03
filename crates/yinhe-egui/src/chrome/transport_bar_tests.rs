
use super::*;
use crate::audio_settings::AudioSettings;
use crate::file_loader::FileLoader;
use crate::view_interaction::FollowMode;
use crate::widgets::action_menu::{PopupRowSpec, measure_menu_width, popup_menu_row};
use crate::widgets::tools_panel::Tool;
use yinhe_editor_core::document::Document;

/// 回归测试：三个动作菜单宽度按内容测量（不同菜单各自定宽），
/// 中文环境实测 文件≈141 / 编辑≈133 / 播放≈160（旧固定值 220），
/// 长标签语言会自动撑宽。断言落在合理范围，防止测量逻辑回归
/// （如宽度退回常量、测量崩坏产生极端值）。
#[test]
fn menu_widths_are_content_aware() {
    let ctx = egui::Context::default();
    ctx.add_font(egui_material_icons::font_insert());
    // 先跑两帧：add_font 下一 pass 才生效，且 fonts 需 run() 初始化
    ctx.run_ui(Default::default(), |_| {})
        .drop_without_applying_deltas();
    ctx.run_ui(Default::default(), |_| {})
        .drop_without_applying_deltas();
    let settings = AudioSettings::default();
    let kbs = &settings.keybindings;
    let w_file = measure_menu_width(&ctx, &FILE_GROUPS, kbs);
    let w_edit = measure_menu_width(&ctx, &EDIT_GROUPS, kbs);
    let play: [&[PlayMenuAction]; 2] = [
        &[
            PlayMenuAction::PlayPause { playing: false },
            PlayMenuAction::Stop,
            PlayMenuAction::Record { recording: false },
            PlayMenuAction::StepInput { active: false },
        ],
        &[PlayMenuAction::Follow(FollowMode::None, true)],
    ];
    let w_play = measure_menu_width(&ctx, &play, kbs);
    for (name, w) in [("file", w_file), ("edit", w_edit), ("play", w_play)] {
        assert!((100.0..=500.0).contains(&w), "{name} 菜单宽度异常: {w}");
    }
}

/// 回归测试：播放菜单各行的垂直间距必须一致。
/// 此前出现过"播放/暂停"与"停止"之间间距异常的问题。
#[test]
fn play_menu_rows_have_consistent_spacing() {
    let mut ys: Vec<f32> = Vec::new();
    let mut spacing_y = 0.0f32;
    let mut interact_y = 0.0f32;
    let ctx = egui::Context::default();
    // 注册 material icons 字体（popup_menu_row 用图标字体渲染）
    ctx.add_font(egui_material_icons::font_insert());
    let output = ctx.run_ui(Default::default(), |ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.spacing_mut().interact_size.y = 22.0;
        spacing_y = ui.spacing().item_spacing.y;
        interact_y = ui.spacing().interact_size.y;
        ui.set_min_width(200.0);
        ui.set_max_width(200.0);
        // 带 shortcut（与真实 popup 一致：播放/暂停 Space、停止 Esc）
        // 含录音/步进行（无快捷键），覆盖全部播放菜单行的间距一致性。
        let rows: [(PlayMenuAction, Option<&str>); 6] = [
            (PlayMenuAction::PlayPause { playing: false }, Some("Space")),
            (PlayMenuAction::Stop, Some("Esc")),
            (PlayMenuAction::Record { recording: false }, None),
            (PlayMenuAction::StepInput { active: false }, None),
            (PlayMenuAction::Follow(FollowMode::None, true), None),
            (PlayMenuAction::Follow(FollowMode::Page, false), None),
        ];
        for (r, shortcut) in rows {
            let (resp, _) = popup_menu_row(
                ui,
                PopupRowSpec {
                    icon: r.icon(),
                    label: &t!(r.label_key()),
                    shortcut,
                    enabled: true,
                    selected: r.is_selected(),
                    accent: r.icon_accent(),
                    pin: None,
                    chevron: false,
                },
            );
            ys.push(resp.rect.min.y);
        }
    });
    output.drop_without_applying_deltas();
    let gaps: Vec<f32> = ys.windows(2).map(|w| w[1] - w[0]).collect();
    assert!(
        gaps.iter().all(|g| (g - gaps[0]).abs() < 0.5),
        "播放菜单行间距不一致: {gaps:?}，ys={ys:?}"
    );
    // 行间距 = 行高 22 + item_spacing 6（与通用 menu 22/6 统一）
    let expected = 22.0 + spacing_y;
    assert!(
        gaps.iter().all(|g| (g - expected).abs() < 1.5),
        "行间距异常: {gaps:?}（期望约 {expected}）"
    );
}

// ─────────────────────────────────────────────────────────────
// 双击空白区最大化 / 空白区拖拽窗口 的回归测试（egui_kittest 无头模拟）
// 空白区判定基于 egui hit test（点击未被任何 widget 消费 / 指针下
// 无任何可交互 widget），因此 transport bar 上透明隐藏按钮（图钉、
// hover 图标等）也会被正确排除，双击它们不会误触发最大化。
// ─────────────────────────────────────────────────────────────

use egui_kittest::Harness;

/// 测试状态：记录隐藏按钮（模拟 transport bar 上的透明图标）是否被点击。
#[derive(Default)]
struct TbTestState {
    hidden_clicked: bool,
}

fn make_transport_harness<'a>(doc: Option<&'a Document>) -> Harness<'a, ()> {
    let mut file_loader = FileLoader::new(yinhe_editor_core::progress::new_shared());
    let mut follow_mode = FollowMode::None;
    let mut active_tool = Tool::Select;
    let mut status_hint: Option<String> = None;
    let mut settings = AudioSettings::default();

    let mut first_frame = true;
    Harness::builder()
        .with_size(egui::vec2(1200.0, 60.0))
        .build_ui_state(
            move |ui, _| {
                // 第一帧只注册 material-icons 字体（add_font 下一 pass 才生效，
                // 若同帧渲染图标按钮会 panic），后续帧才渲染 transport bar。
                if first_frame {
                    first_frame = false;
                    ui.ctx().add_font(egui_material_icons::font_insert());
                    return;
                }
                let mut ori = yinhe_types::Orientation::Horizontal;
                let mut ctx = TransportContext {
                    file_loader: &mut file_loader,
                    doc,
                    follow_mode: &mut follow_mode,
                    active_tool: &mut active_tool,
                    status_hint: &mut status_hint,
                    settings: &mut settings,
                    is_recording: false,
                    step_input: false,
                    orientation: &mut ori,
                };
                show(ui, &mut ctx);
            },
            (),
        )
}

/// 同 make_transport_harness，但在 transport bar 最右端空白区放一个
/// 透明按钮（Button::new("").frame(false)）模拟"隐藏图标"——它渲染在
/// transport bar 之后（hit test 顶层），是真实存在的交互 widget。
fn make_harness_with_hidden_button<'a>(doc: Option<&'a Document>) -> Harness<'a, TbTestState> {
    let mut file_loader = FileLoader::new(yinhe_editor_core::progress::new_shared());
    let mut follow_mode = FollowMode::None;
    let mut active_tool = Tool::Select;
    let mut status_hint: Option<String> = None;
    let mut settings = AudioSettings::default();

    let mut first_frame = true;
    Harness::builder()
        .with_size(egui::vec2(1200.0, 60.0))
        .build_ui_state(
            move |ui, state| {
                if first_frame {
                    first_frame = false;
                    ui.ctx().add_font(egui_material_icons::font_insert());
                    return;
                }
                let mut ori = yinhe_types::Orientation::Horizontal;
                let mut ctx = TransportContext {
                    file_loader: &mut file_loader,
                    doc,
                    follow_mode: &mut follow_mode,
                    active_tool: &mut active_tool,
                    status_hint: &mut status_hint,
                    settings: &mut settings,
                    is_recording: false,
                    step_input: false,
                    orientation: &mut ori,
                };
                show(ui, &mut ctx);
                // 透明隐藏按钮：位于 x 1150..1174、y 8..32
                let hidden_btn = ui.put(
                    egui::Rect::from_min_size(egui::pos2(1150.0, 8.0), egui::vec2(24.0, 24.0)),
                    egui::Button::new("").frame(false),
                );
                if hidden_btn.clicked() {
                    state.hidden_clicked = true;
                }
            },
            TbTestState::default(),
        )
}

/// 在给定时间注入一个指针事件并渲染一帧。
fn event_at(h: &mut Harness<'_, ()>, time: f64, event: egui::Event) {
    h.input_mut().time = Some(time);
    h.event(event);
    h.step();
}

fn event_at_state(h: &mut Harness<'_, TbTestState>, time: f64, event: egui::Event) {
    h.input_mut().time = Some(time);
    h.event(event);
    h.step();
}

fn press_at(h: &mut Harness<'_, ()>, pos: egui::Pos2, time: f64) {
    event_at(h, time, egui::Event::PointerMoved(pos));
    event_at(
        h,
        time + 0.001,
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::default(),
        },
    );
}

fn press_at_state(h: &mut Harness<'_, TbTestState>, pos: egui::Pos2, time: f64) {
    event_at_state(h, time, egui::Event::PointerMoved(pos));
    event_at_state(
        h,
        time + 0.001,
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::default(),
        },
    );
}

fn release_at(h: &mut Harness<'_, ()>, pos: egui::Pos2, time: f64) {
    event_at(
        h,
        time,
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::default(),
        },
    );
}

fn release_at_state(h: &mut Harness<'_, TbTestState>, pos: egui::Pos2, time: f64) {
    event_at_state(
        h,
        time,
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::default(),
        },
    );
}

fn click_at(h: &mut Harness<'_, ()>, pos: egui::Pos2, time: f64) {
    press_at(h, pos, time);
    release_at(h, pos, time + 0.05);
}

fn click_at_state(h: &mut Harness<'_, TbTestState>, pos: egui::Pos2, time: f64) {
    press_at_state(h, pos, time);
    release_at_state(h, pos, time + 0.05);
}

/// 两次单击，间隔 0.15s（小于 400ms 双击窗口）。
fn double_click_at(h: &mut Harness<'_, ()>, pos: egui::Pos2, time: f64) {
    click_at(h, pos, time);
    click_at(h, pos, time + 0.15);
}

fn double_click_at_state(h: &mut Harness<'_, TbTestState>, pos: egui::Pos2, time: f64) {
    click_at_state(h, pos, time);
    click_at_state(h, pos, time + 0.15);
}

fn has_command(h: &Harness<'_, ()>, cmd: &egui::ViewportCommand) -> bool {
    h.output()
        .viewport_output
        .get(&egui::ViewportId::ROOT)
        .is_some_and(|o| o.commands.iter().any(|c| c == cmd))
}

fn has_command_state(h: &Harness<'_, TbTestState>, cmd: &egui::ViewportCommand) -> bool {
    h.output()
        .viewport_output
        .get(&egui::ViewportId::ROOT)
        .is_some_and(|o| o.commands.iter().any(|c| c == cmd))
}

/// 回归测试：双击 transport bar 真空白区域应发送最大化命令。
#[test]
fn double_click_blank_area_toggles_maximize() {
    let doc = yinhe_test_helpers::make_test_document();
    let mut h = make_transport_harness(Some(&doc));
    double_click_at(&mut h, egui::pos2(1100.0, 20.0), 1.0);
    assert!(
        has_command(&h, &egui::ViewportCommand::Maximized(true)),
        "双击空白区应发送 Maximized 命令，实际命令: {:?}",
        h.output()
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map(|o| &o.commands)
    );
}

/// 回归测试：双击按钮区域不得触发最大化。
#[test]
fn double_click_on_button_does_not_maximize() {
    let doc = yinhe_test_helpers::make_test_document();
    let mut h = make_transport_harness(Some(&doc));
    // 最左侧按钮（文件菜单按钮）中心
    double_click_at(&mut h, egui::pos2(24.0, 20.0), 1.0);
    assert!(
        !has_command(&h, &egui::ViewportCommand::Maximized(true)),
        "双击按钮不应触发最大化"
    );
}

/// 回归测试：双击 transport bar 上的透明隐藏按钮（图钉/hover 图标等）
/// 不得触发最大化——空白区判定基于 hit test，隐藏按钮也是 widget。
#[test]
fn double_click_on_hidden_button_does_not_maximize() {
    let doc = yinhe_test_helpers::make_test_document();
    let mut h = make_harness_with_hidden_button(Some(&doc));
    // 隐藏按钮中心（x 1150..1174，y 8..32）
    double_click_at_state(&mut h, egui::pos2(1162.0, 20.0), 1.0);
    assert!(
        !has_command_state(&h, &egui::ViewportCommand::Maximized(true)),
        "双击隐藏按钮不应触发最大化"
    );
}

/// 回归测试：隐藏按钮（透明图标）自身必须可点击——不被拖拽/双击逻辑吞掉。
#[test]
fn hidden_button_still_clickable() {
    let doc = yinhe_test_helpers::make_test_document();
    let mut h = make_harness_with_hidden_button(Some(&doc));
    assert!(!h.state().hidden_clicked);
    click_at_state(&mut h, egui::pos2(1162.0, 20.0), 1.0);
    assert!(h.state().hidden_clicked, "隐藏按钮应响应单击");
}

/// 单击空白区不应触发最大化（防止单次点击误触发）。
#[test]
fn single_click_blank_area_does_not_maximize() {
    let doc = yinhe_test_helpers::make_test_document();
    let mut h = make_transport_harness(Some(&doc));
    click_at(&mut h, egui::pos2(1100.0, 20.0), 1.0);
    assert!(!has_command(&h, &egui::ViewportCommand::Maximized(true)));
}

/// 空白区单击（无位移）不应启动窗口拖动——press 不立即 StartDrag，
/// 这是 click（进而双击）得以产生的保证。
#[test]
fn click_blank_area_does_not_start_drag() {
    let doc = yinhe_test_helpers::make_test_document();
    let mut h = make_transport_harness(Some(&doc));
    let pos = egui::pos2(1100.0, 20.0);
    press_at(&mut h, pos, 3.0);
    assert!(
        !has_command(&h, &egui::ViewportCommand::StartDrag),
        "按下未移动时不应 StartDrag"
    );
    release_at(&mut h, pos, 3.05);
    assert!(!has_command(&h, &egui::ViewportCommand::StartDrag));
}

/// 空白区按住并移动超过点击阈值应启动窗口拖动。
#[test]
fn drag_blank_area_starts_window_drag() {
    let doc = yinhe_test_helpers::make_test_document();
    let mut h = make_transport_harness(Some(&doc));
    let start = egui::pos2(1100.0, 20.0);
    press_at(&mut h, start, 2.0);
    assert!(!has_command(&h, &egui::ViewportCommand::StartDrag));
    // 移动 10px（超过 max_click_dist 默认 6px）→ 启动窗口拖动
    event_at(
        &mut h,
        2.1,
        egui::Event::PointerMoved(start + egui::vec2(10.0, 0.0)),
    );
    assert!(
        has_command(&h, &egui::ViewportCommand::StartDrag),
        "空白区拖动应发送 StartDrag"
    );
    release_at(&mut h, start + egui::vec2(10.0, 0.0), 2.15);
}

/// 隐藏按钮上按下并移动不应启动窗口拖动（隐藏按钮不是空白区）。
#[test]
fn drag_on_hidden_button_does_not_start_drag() {
    let doc = yinhe_test_helpers::make_test_document();
    let mut h = make_harness_with_hidden_button(Some(&doc));
    let start = egui::pos2(1162.0, 20.0);
    press_at_state(&mut h, start, 2.0);
    event_at_state(
        &mut h,
        2.1,
        egui::Event::PointerMoved(start + egui::vec2(10.0, 0.0)),
    );
    assert!(
        !has_command_state(&h, &egui::ViewportCommand::StartDrag),
        "隐藏按钮上拖动不应启动窗口拖动"
    );
    release_at_state(&mut h, start + egui::vec2(10.0, 0.0), 2.15);
}
