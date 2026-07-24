//! PPQ 变更确认对话框（标准 viewport 形式）。
//!
//! 当用户修改项目 PPQ 且工程中有音符时弹出，询问是否缩放音符 tick。
//! 独立 viewport，不受主窗口 tab/面板开关影响。

use eframe::egui;
use rust_i18n::t;

/// 用户选择。
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum PpqRescaleAction {
    /// 用户还没做出选择。
    None,
    /// 是（缩放音符）。
    Rescale,
    /// 否（保持音符）。
    NoRescale,
    /// 取消（还原 PPQ）。
    Cancel,
}

/// 显示 PPQ rescale 确认对话框。
///
/// `old` / `new` 为 PPQ 变更前后的值。
/// 返回 [`PpqRescaleAction::None`] 表示用户还没选择（窗口仍然打开）。
pub(crate) fn show_viewport(ctx: &egui::Context, old: u32, new: u32) -> PpqRescaleAction {
    let viewport_id = egui::ViewportId::from_hash_of("ppq_rescale_confirm_dialog");

    let action_rc: std::rc::Rc<std::cell::RefCell<Option<PpqRescaleAction>>> =
        std::rc::Rc::new(std::cell::RefCell::new(None));
    let action_cb = action_rc.clone();
    let ctx_clone = ctx.clone();

    ctx_clone.show_viewport_immediate(
        viewport_id,
        crate::chrome::dialog::viewport_builder(t!("dialog.ppq_rescale.title").as_ref(), [380.0, 200.0], false),
        move |vctx, _class| {
            let mut close = false;
            if vctx.input(|i| i.viewport().close_requested()) {
                *action_cb.borrow_mut() = Some(PpqRescaleAction::Cancel);
                close = true;
            }
            egui::CentralPanel::default()
                .frame(egui::Frame {
                    fill: crate::theme::APP_BG,
                    ..Default::default()
                })
                .show(vctx, |ui| {
                    crate::chrome::dialog::title_bar(ui, t!("dialog.ppq_rescale.title").as_ref(), &mut close);
                    egui::Frame::new()
                        .inner_margin(egui::Margin {
                            left: 12,
                            right: 12,
                            top: 0,
                            bottom: 12,
                        })
                        .show(ui, |ui| {
                            ui.set_max_width(360.0);
                            ui.add_space(6.0);
                            ui.label(t!("dialog.ppq_rescale.desc", old = old, new = new).as_ref());
                            ui.add_space(4.0);
                            ui.label(t!("dialog.ppq_rescale.question").as_ref());
                            ui.add_space(6.0);
                            ui.label(
                                egui::RichText::new(
                                    t!("dialog.ppq_rescale.hint").as_ref(),
                                )
                                .color(egui::Color32::from_gray(140))
                                .size(11.0),
                            );
                            ui.add_space(12.0);
                            ui.horizontal(|ui| {
                                ui.spacing_mut().button_padding = egui::vec2(10.0, 4.0);
                                if ui.button(t!("dialog.ppq_rescale.yes").as_ref()).clicked() {
                                    *action_cb.borrow_mut() = Some(PpqRescaleAction::Rescale);
                                    close = true;
                                }
                                ui.add_space(4.0);
                                if ui.button(t!("dialog.ppq_rescale.no").as_ref()).clicked() {
                                    *action_cb.borrow_mut() = Some(PpqRescaleAction::NoRescale);
                                    close = true;
                                }
                                ui.add_space(4.0);
                                if ui.button(t!("common.cancel").as_ref()).clicked() {
                                    *action_cb.borrow_mut() = Some(PpqRescaleAction::Cancel);
                                    close = true;
                                }
                            });
                        });
                });
            if close {
                vctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            }
        },
    );

    action_rc
        .borrow_mut()
        .take()
        .unwrap_or(PpqRescaleAction::None)
}
