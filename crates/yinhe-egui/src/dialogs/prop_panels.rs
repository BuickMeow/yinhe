//! 属性浮动面板（独立视口子窗口）：音轨属性 / 工程设置。
//!
//! 与右侧栏 Info 内容共用同一套渲染函数（`info_panel::show_track_info` /
//! `project_info::show`），只是换了视图容器。egui 的每个 viewport 有独立的
//! widget id 链，因此与侧栏渲染函数互不打扰，也不会 id clash。
//!
//! 与侧栏互斥切换（见 `App::set_float_panel` / `App::dock_float_panel`）：
//! 同一内容要么在侧栏、要么在浮窗，不会同时出现。

use std::cell::RefCell;
use std::rc::Rc;

use eframe::egui;
use rust_i18n::t;

use yinhe_editor_core::document::Document;

/// 音轨属性浮窗。
///
/// 返回 `port_changed`（端口/通道改变 → 音频引擎需重建）。
/// `open` 在用户点 X 关闭后置 false（调用方据此清掉浮窗状态）。
/// `dock_to_side` 在用户点「停靠到侧栏」后置 true（调用方把内容搬回侧栏）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn show_track_props_viewport(
    ctx: &egui::Context,
    doc: &mut Document,
    audio: Option<&yinhe_audio::CpalAudioHandle>,
    open: &mut bool,
    track_idx: u16,
    dock_to_side: &mut bool,
) -> bool {
    if !*open {
        return false;
    }
    let viewport_id = egui::ViewportId::from_hash_of("track_props_dialog");
    let open_rc = Rc::new(RefCell::new(true));
    let open_out = Rc::clone(&open_rc);
    let dock_rc = Rc::new(RefCell::new(false));
    let dock_out = Rc::clone(&dock_rc);
    let changed_rc = Rc::new(RefCell::new(false));
    let changed_out = Rc::clone(&changed_rc);
    let ctx_clone = ctx.clone();

    ctx_clone.show_viewport_immediate(
        viewport_id,
        crate::chrome::dialog::viewport_builder(
            t!("dialog.track_props.title").as_ref(),
            crate::theme::TRACK_PROPS_POPUP_SIZE,
            true,
        ),
        move |vctx, _class| {
            let mut close = false;
            if vctx.input(|i| i.viewport().close_requested()) {
                close = true;
            }
            egui::CentralPanel::default()
                .frame(egui::Frame {
                    fill: crate::theme::app_bg(),
                    ..Default::default()
                })
                .show(vctx, |ui| {
                    crate::chrome::dialog::title_bar(
                        ui,
                        t!("dialog.track_props.title").as_ref(),
                        &mut close,
                    );
                    egui::Frame::new()
                        .inner_margin(egui::Margin {
                            left: 10,
                            right: 10,
                            top: 4,
                            bottom: 10,
                        })
                        .show(ui, |ui| {
                            // 顶部操作行：停靠回侧栏（内容搬回右侧栏 Info tab）。
                            if ui
                                .add(crate::widgets::menu::menu_item_button(
                                    ui,
                                    false,
                                    t!("panel.dock_to_side").as_ref(),
                                ))
                                .clicked()
                            {
                                *dock_rc.borrow_mut() = true;
                                close = true;
                            }
                            ui.separator();
                            egui::ScrollArea::vertical()
                                .auto_shrink([false; 2])
                                .show(ui, |ui| {
                                    // 浮窗跟随 track_selected（内部下拉选择器换轨即改
                                    // 全局选中态）；打开时兜底选中目标轨。
                                    if !doc.edit.track_selected.contains(&track_idx) {
                                        doc.edit.track_selected.clear();
                                        doc.edit.track_selected.insert(track_idx);
                                    }
                                    // 侧栏的 info_content 不参与浮窗显示，传局部占位。
                                    let mut local_content = None;
                                    let changed = crate::right_panel::info_panel::show_track_info(
                                        ui,
                                        doc,
                                        audio,
                                        &mut local_content,
                                    );
                                    if changed {
                                        *changed_rc.borrow_mut() = true;
                                    }
                                });
                        });
                });
            if close {
                vctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                *open_rc.borrow_mut() = false;
            }
        },
    );

    if !*open_out.borrow() {
        *open = false;
    }
    *dock_to_side = *dock_out.borrow();
    *changed_out.borrow()
}

/// 工程设置浮窗。语义同 `show_track_props_viewport`（无 port_changed 返回值）。
pub(crate) fn show_project_settings_viewport(
    ctx: &egui::Context,
    doc: &mut Document,
    open: &mut bool,
    dock_to_side: &mut bool,
) {
    if !*open {
        return;
    }
    let viewport_id = egui::ViewportId::from_hash_of("project_settings_dialog");
    let open_rc = Rc::new(RefCell::new(true));
    let open_out = Rc::clone(&open_rc);
    let dock_rc = Rc::new(RefCell::new(false));
    let dock_out = Rc::clone(&dock_rc);
    let ctx_clone = ctx.clone();

    ctx_clone.show_viewport_immediate(
        viewport_id,
        crate::chrome::dialog::viewport_builder(
            t!("dialog.project_settings.title").as_ref(),
            crate::theme::PROJECT_SETTINGS_POPUP_SIZE,
            true,
        ),
        move |vctx, _class| {
            let mut close = false;
            if vctx.input(|i| i.viewport().close_requested()) {
                close = true;
            }
            egui::CentralPanel::default()
                .frame(egui::Frame {
                    fill: crate::theme::app_bg(),
                    ..Default::default()
                })
                .show(vctx, |ui| {
                    crate::chrome::dialog::title_bar(
                        ui,
                        t!("dialog.project_settings.title").as_ref(),
                        &mut close,
                    );
                    egui::Frame::new()
                        .inner_margin(egui::Margin {
                            left: 10,
                            right: 10,
                            top: 4,
                            bottom: 10,
                        })
                        .show(ui, |ui| {
                            if ui
                                .add(crate::widgets::menu::menu_item_button(
                                    ui,
                                    false,
                                    t!("panel.dock_to_side").as_ref(),
                                ))
                                .clicked()
                            {
                                *dock_rc.borrow_mut() = true;
                                close = true;
                            }
                            ui.separator();
                            egui::ScrollArea::vertical()
                                .auto_shrink([false; 2])
                                .show(ui, |ui| {
                                    crate::right_panel::project_info::show(ui, Some(doc));
                                });
                        });
                });
            if close {
                vctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                *open_rc.borrow_mut() = false;
            }
        },
    );

    if !*open_out.borrow() {
        *open = false;
    }
    *dock_to_side = *dock_out.borrow();
}
