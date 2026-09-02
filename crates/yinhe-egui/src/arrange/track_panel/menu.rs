use std::sync::Arc;

use eframe::egui;
use rust_i18n::t;
use yinhe_types::AutomationTarget;

use super::types::TrackAction;

/// 「创建自动化」子菜单（主行右键 + 加号占位行共用）：复用 PR AM 面板的
/// target 列表（跳过 Tempo；已有 lane 的 target 不重复显示），自定义 CC 用
/// DragValue 选控制器号。选择后 push CreateAutomation 交给 arrange.rs 落模型。
pub fn create_automation_menu(
    ui: &mut egui::Ui,
    idx: usize,
    tracks: &[Arc<yinhe_core::TrackData>],
    actions: &mut Vec<TrackAction>,
) {
    // 面板轮廓由调用方提供（badge popup 用 Popup::new 的 Frame::menu；
    // 右键菜单自带 context_menu 面板），这里只渲染无边框等宽的菜单项。
    ui.set_min_width(160.0);
    ui.set_max_width(160.0);
    let existing: Vec<AutomationTarget> = tracks
        .get(idx)
        .map(|t| {
            t.automation_lanes
                .iter()
                .map(|l| l.target.clone())
                .collect()
        })
        .unwrap_or_default();
    for target in crate::piano_view::automation_panel::AUTOMATION_TARGETS {
        if matches!(target, AutomationTarget::Tempo) || existing.contains(target) {
            continue;
        }
        let label = super::super::am_lanes::lane_label(target);
        if ui
            .add(crate::widgets::menu::menu_item_button(ui, false, label))
            .clicked()
        {
            actions.push(TrackAction::CreateAutomation {
                idx,
                target: target.clone(),
            });
            ui.close();
        }
    }
    ui.separator();
    // 自定义 CC：菜单内 DragValue（0..=127）+ 无边框「创建」按钮。
    let cc_id = egui::Id::new(("arr_custom_cc", idx));
    let mut cc = ui.ctx().data_mut(|d| d.get_temp::<u8>(cc_id)).unwrap_or(7);
    ui.horizontal(|ui| {
        ui.label(t!("arrange.custom_cc"));
        if ui
            .add(egui::DragValue::new(&mut cc).range(0..=127))
            .changed()
        {
            ui.ctx().data_mut(|d| d.insert_temp(cc_id, cc));
        }
        if ui
            .add(crate::widgets::menu::menu_item_button(
                ui,
                false,
                t!("arrange.create"),
            ))
            .clicked()
        {
            let target = AutomationTarget::CC { controller: cc };
            if !existing.contains(&target) {
                actions.push(TrackAction::CreateAutomation { idx, target });
            }
            ui.close();
        }
    });
}
