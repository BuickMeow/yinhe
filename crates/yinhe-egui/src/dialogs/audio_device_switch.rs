use eframe::egui;
use rust_i18n::t;

/// "音频设备切换"对话框的用户动作。
///
/// `show_viewport` 每帧返回一个；`None` 表示用户还没做出选择。
pub(crate) enum AudioDeviceSwitchAction {
    None,
    /// 用户点了一个设备名，请求切换。
    Switch(String),
    /// 用户点了"刷新设备列表"。
    Refresh,
    /// 用户点了"保持当前设备"（仅在 `allow_keep_current = true` 时出现）。
    KeepCurrent,
    /// 用户点了"退出 yinhe"。
    Exit,
}

/// 显示"音频设备已断开/变更"对话框，让用户选一个新的输出设备。
///
/// `devices` 是当前系统可用的设备名列表（由调用方从
/// `yinhe_audio::list_output_devices()` 拉取，刷新时也由调用方更新）。
/// `error` 是上一次 spawn 失败的错误信息（如果有），用来在对话框底部红字提示。
/// `allow_keep_current` = true 时显示"保持当前设备"按钮（设备列表变更场景，
/// 流还活着）；= false 时不显示（stream_error 场景，流已死必须切换）。
///
/// 对话框不能被标题栏关闭按钮关掉 —— 用户必须选一个设备、保持当前或点"退出"。
/// 每帧 `raise_viewport` 会把它重新拉到前台。
pub(crate) fn show_viewport(
    ctx: &egui::Context,
    devices: &[String],
    error: Option<&str>,
    allow_keep_current: bool,
) -> AudioDeviceSwitchAction {
    let viewport_id = egui::ViewportId::from_hash_of("audio_device_switch_dialog");

    let action_rc = std::rc::Rc::new(std::cell::RefCell::new(AudioDeviceSwitchAction::None));
    let action_capture = action_rc.clone();
    let devices_vec = devices.to_vec();
    let error_str = error.map(|s| s.to_string());
    let ctx_clone = ctx.clone();

    ctx_clone.show_viewport_immediate(
        viewport_id,
        crate::chrome::dialog::viewport_builder(t!("dialog.audio_switch.title").as_ref(), [460.0, 440.0], false),
        move |vctx, _class| {
            let mut hide = false;
            if vctx.input(|i| i.viewport().close_requested()) {
                hide = true;
            }
            egui::CentralPanel::default()
                .frame(egui::Frame {
                    fill: crate::theme::APP_BG,
                    ..Default::default()
                })
                .show(vctx, |ui| {
                    crate::chrome::dialog::title_bar(ui, t!("dialog.audio_switch.title").as_ref(), &mut hide);
                    egui::Frame::new()
                        .inner_margin(egui::Margin {
                            left: 12,
                            right: 12,
                            top: 0,
                            bottom: 12,
                        })
                        .show(ui, |ui| {
                            ui.set_max_width(460.0);

                            ui.vertical_centered(|ui| {
                                ui.add_space(8.0);
                                if allow_keep_current {
                                    ui.label(t!("dialog.audio_switch.devices_changed").as_ref());
                                    ui.add_space(4.0);
                                    ui.label(t!("dialog.audio_switch.select_new").as_ref());
                                } else {
                                    ui.label(t!("dialog.audio_switch.stream_error").as_ref());
                                    ui.add_space(4.0);
                                    ui.label(t!("dialog.audio_switch.select_device").as_ref());
                                }
                                ui.add_space(8.0);
                            });

                            // 设备列表 —— 用 radio button 替代宽按钮
                            egui::ScrollArea::vertical()
                                .max_height(220.0)
                                .show(ui, |ui| {
                                    if devices_vec.is_empty() {
                                        ui.vertical_centered(|ui| {
                                            ui.add_space(12.0);
                                            ui.label(
                                                egui::RichText::new(t!("dialog.audio_switch.no_devices").as_ref())
                                                    .color(egui::Color32::from_gray(140)),
                                            );
                                        });
                                    }
                                    for name in &devices_vec {
                                        let resp = ui.add(
                                            egui::RadioButton::new(false, name),
                                        );
                                        if resp.clicked() {
                                            *action_capture.borrow_mut() =
                                                AudioDeviceSwitchAction::Switch(name.clone());
                                            hide = true;
                                        }
                                    }
                                });

                            ui.add_space(8.0);

                            ui.vertical_centered(|ui| {
                                if ui.button(t!("settings.refresh_devices").as_ref()).clicked() {
                                    *action_capture.borrow_mut() =
                                        AudioDeviceSwitchAction::Refresh;
                                }

                                if allow_keep_current {
                                    ui.add_space(8.0);
                                    if ui.button(t!("dialog.audio_switch.keep_current").as_ref()).clicked() {
                                        *action_capture.borrow_mut() =
                                            AudioDeviceSwitchAction::KeepCurrent;
                                        hide = true;
                                    }
                                }

                                if let Some(err) = &error_str {
                                    ui.add_space(8.0);
                                    ui.label(
                                        egui::RichText::new(err)
                                            .color(egui::Color32::from_rgb(232, 80, 80)),
                                    );
                                }

                                ui.add_space(16.0);
                                if ui.button(t!("common.exit_app").as_ref()).clicked() {
                                    *action_capture.borrow_mut() =
                                        AudioDeviceSwitchAction::Exit;
                                    hide = true;
                                }
                            });
                        });
                });
            if hide {
                vctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            }
        },
    );

    let action = std::mem::replace(&mut *action_rc.borrow_mut(), AudioDeviceSwitchAction::None);
    action
}
