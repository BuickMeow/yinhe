use super::cards::{instrument_targets, vertical_title_shape};
use super::knobs::{begin_knob_drag, finish_knob_drag};
use super::*;
use yinhe_editor_core::document::Document;
use yinhe_types::automation::{CHANNEL_DSP_PARAMS, MidiBinding, XSYNTH_PARAMS};

/// 音源面板列表：通道处理参数（前 5）+ XSynth 参数（后 7）；显示名用参数表
/// 名字（如 "Sustain"），CC 绑定参数走低层 CC lane，PB/RPN 保留设备参数。
#[test]
fn instrument_targets_merge_channel_dsp_and_xsynth() {
    let params = instrument_targets(4);
    assert_eq!(params.len(), CHANNEL_DSP_PARAMS.len() + XSYNTH_PARAMS.len());
    // Pan（下标 2）在参数表里是中心参考参数 → 居中归零。
    assert!(params[2].3, "Pan 应居中归零");

    // 通道处理的 Volume/Expression 默认满增益（表默认 1.0），
    // 不能是 CC 通用默认值 0——否则面板无事件时显示 0。
    assert_eq!(params[0].2, 1.0, "Volume 默认应为满增益");
    assert_eq!(params[1].2, 1.0, "Expression 默认应为满增益");

    let names: Vec<&str> = params.iter().map(|(_, name, _, _)| name.as_str()).collect();
    let expect: Vec<&str> = CHANNEL_DSP_PARAMS
        .iter()
        .chain(XSYNTH_PARAMS)
        .map(|p| p.name)
        .collect();
    assert_eq!(names, expect, "顺序应为通道处理参数在前、XSynth 在后");

    let targets: Vec<AutomationTarget> =
        params.into_iter().map(|(target, _, _, _)| target).collect();
    // 通道处理参数全部是 CC 绑定（Volume=CC7 / Expression=CC11 / Pan=CC10 /
    // Cutoff=CC74 / Resonance=CC71）。
    for p in CHANNEL_DSP_PARAMS {
        let MidiBinding::Cc(cc) = p.midi else {
            panic!("通道处理参数应绑定 CC：{}", p.name);
        };
        assert!(
            targets.contains(&AutomationTarget::CC { controller: cc }),
            "CC{cc} 应列出"
        );
    }
    // XSynth：全部内置参数统一为低层 MIDI 目标（CC / PitchBend / RPN）。
    for cc in [64u8, 72, 73] {
        assert!(targets.contains(&AutomationTarget::CC { controller: cc }));
    }
    assert!(targets.contains(&AutomationTarget::PitchBend));
    for parameter in [0u16, 1, 2] {
        assert!(
            targets.contains(&AutomationTarget::Rpn { parameter }),
            "RPN{parameter} 应列出"
        );
    }
}

/// 拖动会话（空 lane）：松手写事件并 push undo，undo/redo 往返一致。
#[test]
fn knob_drag_writes_event_and_undo_roundtrip() {
    let mut doc = Document::empty();
    let target = AutomationTarget::CC { controller: 1 };
    // track 1 才是 MIDI 轨（track 0 是 Conductor/Master，不能挂 CC 自动化）。
    let lane = |doc: &Document| -> Vec<AutomationEvent> {
        doc.data.model.tracks[1]
            .automation_lanes
            .iter()
            .find(|l| l.target == target)
            .map(|l| l.events.clone())
            .unwrap_or_default()
    };

    let mut drag = begin_knob_drag(&mut doc, &[1], target.clone(), 240).expect("目标轨存在");
    assert!(drag.lane_idx.is_none(), "空 lane 无索引");
    assert!(drag.before.is_empty(), "编辑前无事件");

    drag.last_norm = 0.75;
    drag.moved = true;
    finish_knob_drag(&mut doc, drag);

    let events = lane(&doc);
    assert_eq!(events.len(), 1, "lane 懒创建并写入事件");
    assert_eq!(events[0].tick, 240);
    assert_eq!(events[0].value, 0.75);
    assert!(doc.history.can_undo());

    assert!(doc.undo());
    assert!(lane(&doc).is_empty(), "undo 应移除新增事件");

    assert!(doc.redo());
    let events = lane(&doc);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].value, 0.75);
}

/// 拖动会话（已有同 tick 事件）：只更新值并保留 id，undo 恢复旧值。
#[test]
fn knob_drag_updates_existing_event_and_undo_restores() {
    let mut doc = Document::empty();
    let target = AutomationTarget::CC { controller: 7 };
    doc.add_automation_event(
        1,
        target.clone(),
        AutomationEvent {
            id: 0,
            tick: 100,
            value: 0.2,
            shape: target.default_shape(),
        },
    )
    .expect("lane 懒创建");
    let old_id = doc.data.model.tracks[1].automation_lanes[0].events[0].id;
    assert_ne!(old_id, 0, "新增事件应已发号");

    let mut drag = begin_knob_drag(&mut doc, &[1], target.clone(), 100).expect("目标轨存在");
    assert!(drag.lane_idx.is_some(), "已有 lane 应记录索引");
    assert_eq!(drag.before.len(), 1, "编辑前快照应含已有事件");

    drag.last_norm = 0.9;
    drag.moved = true;
    finish_knob_drag(&mut doc, drag);

    let events = doc.data.model.tracks[1].automation_lanes[0].events.clone();
    assert_eq!(events.len(), 1, "同 tick 只更新不新增");
    assert_eq!(events[0].value, 0.9);
    assert_eq!(events[0].id, old_id, "更新保留事件 id");

    assert!(doc.undo());
    let events = doc.data.model.tracks[1].automation_lanes[0].events.clone();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].value, 0.2, "undo 恢复旧值");
    assert_eq!(events[0].id, old_id);
}
/// 收起态竖排标题：旋转后的可视中心必须落在标题条矩形中心。
#[test]
fn vertical_title_is_centered() {
    let ctx = egui::Context::default();
    ctx.run_ui(Default::default(), |ui| {
        let rect = egui::Rect::from_min_size(egui::pos2(20.0, 40.0), egui::vec2(24.0, 180.0));
        let galley = ui.painter().layout_no_wrap(
            "XSynth".to_owned(),
            egui::FontId::proportional(13.0),
            egui::Color32::WHITE,
        );
        let bb = vertical_title_shape(rect, galley, egui::Color32::WHITE).visual_bounding_rect();
        assert!((bb.center().x - rect.center().x).abs() < 1.0, "{bb:?}");
        assert!((bb.center().y - rect.center().y).abs() < 1.0, "{bb:?}");
    })
    .drop_without_applying_deltas();
}

/// Conductor/Master 轨映射到主输出设备链，而不是继续留在上一个通道语境。
#[test]
fn master_track_maps_to_master_dock_context() {
    let doc = Document::empty();
    let master = &doc.data.model.tracks[0];
    assert_eq!(track_context(master), Some(DockContext::Master));
}
