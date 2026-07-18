use eframe::egui;

/// 不可恢复的 GPU 硬件/驱动错误提示对话框。
///
/// 触发条件（在 `App::show_dialogs` 中）：任一 `RenderContext::device_lost()`
/// 为 true（GPU 驱动 TDR、设备热拔、显存耗尽等）。
///
/// GPU device lost 应用层无重建路径，唯一选择是退出并让用户重启。
/// 音频流错误（cpal `stream_error`）不走这里 —— 那个可以换设备重建，
/// 走 `dialogs::audio_device_switch`。
///
/// 调用方每帧调用：只要触发条件成立就持续显示，用户点击"退出"后由调用方
/// 设置 `should_exit = true`，下一帧 main_loop 会发送 `ViewportCommand::Close`。
///
/// 返回 true 表示用户选择了退出。
pub(crate) fn show_viewport(ctx: &egui::Context) -> bool {
    let viewport_id = egui::ViewportId::from_hash_of("gpu_device_lost_dialog");
    crate::chrome::dialog::raise_viewport(ctx, viewport_id);

    let exit_cb = std::rc::Rc::new(std::cell::RefCell::new(false));
    let exit_capture = exit_cb.clone();
    let ctx_clone = ctx.clone();

    ctx_clone.show_viewport_immediate(
        viewport_id,
        crate::chrome::dialog::viewport_builder("需要重启", [460.0, 200.0], false),
        move |vctx, _class| {
            let mut close = false;
            if vctx.input(|i| i.viewport().close_requested()) {
                close = true;
            }
            egui::CentralPanel::default()
                .frame(egui::Frame {
                    fill: crate::theme::APP_BG,
                    ..Default::default()
                })
                .show(vctx, |ui| {
                    crate::chrome::dialog::title_bar(ui, "需要重启", &mut close);
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
                                ui.label(
                                    "GPU 设备已不可恢复地丢失\n\
                                     （驱动 TDR / 设备热拔 / 显存耗尽等）。\n\
                                     钢琴卷帘渲染已永久停止，继续操作也无法恢复。",
                                );
                                ui.add_space(4.0);
                                ui.label("请保存当前工程并退出程序后重新启动。");
                                ui.add_space(16.0);
                                if ui.button("退出 yinhe").clicked() {
                                    close = true;
                                    *exit_capture.borrow_mut() = true;
                                }
                            });
                        });
                });
            if close {
                vctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            }
        },
    );

    *exit_cb.borrow()
}
