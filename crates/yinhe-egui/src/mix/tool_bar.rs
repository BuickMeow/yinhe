//! 混音台左侧竖排工具栏（PR 面板工具条同款）：添加总线 / 扫描插件。
//!
//! 取代原混音台顶部条：两个操作按钮移入左侧竖条常驻；「N 个效果器」等扫描状态
//! 文本不再常驻显示，改为悬停「扫描插件」时在 mode bar 讲解行展示
//! （扫描中进度 / 上次扫描到的乐器与效果器数量 / 失败包数）。机架最近一次
//! 加载/激活错误也改为悬停竖条时展示，避免常驻占位。

use eframe::egui;
use egui_material_icons::icons::{ICON_ADD_LINK, ICON_TRAVEL_EXPLORE};
use rust_i18n::t;

use super::MixAction;
use super::plugin_instance::PluginEntry;
use crate::app::App;

/// 在给定矩形内渲染混音台左侧工具栏（由 mix 面板左缘扣出的竖条）。
pub(crate) fn show(app: &App, ui: &mut egui::Ui, rect: egui::Rect, actions: &mut Vec<MixAction>) {
    let btn_size = egui::vec2(
        crate::theme::TRANSPORT_BTN_SIZE,
        crate::theme::TRANSPORT_BTN_SIZE,
    );
    let mut hovered_hint: Option<String> = None;
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui.set_clip_rect(rect.intersect(ui.clip_rect()));
        ui.painter()
            .rect_filled(rect, 0.0, crate::theme::track_bg());
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
            ui.add_space(6.0);
            // 添加总线。
            let resp = icon_button(ui, "add_bus", ICON_ADD_LINK, btn_size, false);
            if resp.clicked() {
                actions.push(MixAction::AddBus);
            }
            if resp.hovered() {
                hovered_hint = Some(t!("hint.mix.add_bus").to_string());
            }
            // 扫描插件：扫描中图标走强调色，作为进度反馈。
            let resp = icon_button(
                ui,
                "scan_plugins",
                ICON_TRAVEL_EXPLORE,
                btn_size,
                app.mix.scan_in_progress,
            );
            if resp.clicked() {
                actions.push(MixAction::RescanPlugins);
            }
            if resp.hovered() {
                hovered_hint = Some(scan_hint(
                    app.mix.scan_in_progress,
                    app.mix.scanned.as_deref(),
                    app.mix.scan_errors,
                ));
            }
        });
    });

    let ctx = ui.ctx();
    if let Some(hint) = hovered_hint {
        crate::widgets::hint::set(ctx, hint);
    } else if ui
        .input(|i| i.pointer.hover_pos())
        .is_some_and(|p| rect.contains(p))
    {
        // 机架错误（插件加载/激活失败）优先于常规说明：删除顶部条后此处是
        // 该错误唯一的展示位（悬停工具栏空白处可见）。
        if let Some(err) = app
            .workspace
            .active_doc
            .and_then(|idx| app.mixer_racks.get(idx))
            .and_then(|r| r.last_error.as_ref())
        {
            crate::widgets::hint::set(ctx, t!("hint.mix.rack_error", error = err.clone()));
        }
    }
}

/// 扫描按钮讲解：扫描中显示进度；完成后显示上次扫描到的乐器/效果器数量与失败包数；
/// 尚未扫描过时提示重新扫描。
pub(crate) fn scan_hint(
    in_progress: bool,
    scanned: Option<&[PluginEntry]>,
    scan_errors: usize,
) -> String {
    if in_progress {
        return t!("mix.scanning").to_string();
    }
    let Some(plugins) = scanned else {
        return t!("hint.mix.scan_plugins").to_string();
    };
    let inst = plugins.iter().filter(|p| p.is_instrument).count();
    let fx = plugins.iter().filter(|p| p.is_effect).count();
    let mut hint = t!("hint.mix.scan_result", instruments = inst, effects = fx).to_string();
    if scan_errors > 0 {
        hint.push_str(&format!(
            " · {}",
            t!("mix.scan_errors", count = scan_errors)
        ));
    }
    hint
}

/// 图标按钮（PR 工具栏同款：32px 方形幽灵按钮；`accent` = 选中底 + 强调色图标）。
fn icon_button(
    ui: &mut egui::Ui,
    id_salt: &str,
    icon: egui_material_icons::MaterialIcon,
    size: egui::Vec2,
    accent: bool,
) -> egui::Response {
    let color = if accent {
        crate::theme::accent_active()
    } else {
        crate::theme::text_primary()
    };
    let sel_bg = accent.then(crate::theme::selected_bg);
    ui.push_id(("mix_tool", id_salt), |ui| {
        crate::widgets::flat::flat_button_filled_ghost(
            ui,
            icon.rich_text()
                .size(crate::theme::TRANSPORT_BTN_FONT)
                .color(color),
            size,
            sel_bg,
            true,
        )
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_mixer::PluginFormat;

    fn entry(is_instrument: bool, is_effect: bool) -> PluginEntry {
        PluginEntry {
            format: PluginFormat::Clap,
            path: std::path::PathBuf::from("/tmp/plug.clap"),
            id: "id".to_string(),
            name: "plug".to_string(),
            vendor: "vendor".to_string(),
            is_instrument,
            is_effect,
            error: None,
        }
    }

    /// 完成后讲解包含乐器/效果器数量；扫描中与未扫描各有独立文案。
    #[test]
    fn scan_hint_reports_counts_and_states() {
        let plugins = vec![entry(true, false), entry(true, false), entry(false, true)];
        let done = scan_hint(false, Some(&plugins), 0);
        assert!(
            done.contains('2') && done.contains('1'),
            "应含 2 乐器 1 效果器"
        );

        let with_errors = scan_hint(false, Some(&plugins), 3);
        assert!(with_errors.contains('3'), "失败包数应拼进讲解");

        let scanning = scan_hint(true, Some(&plugins), 0);
        let never = scan_hint(false, None, 0);
        assert_ne!(scanning, done);
        assert_ne!(never, done, "未扫描过与已完成文案应不同");
    }
}
