//! 内置 XSynth 的配置窗口（虚拟乐器界面）：配置该源通道使用的音色库。
//!
//! 与 VST 插件"每个实例有自己的界面"对应——XSynth 是内置合成器，此窗口就是
//! 它的"界面"：
//! - 默认使用**全局音色库**（设置 → 音色库）；
//! - 可为当前通道单独配置一组音色库（随工程保存；同一通道多轨共享）。

use std::cell::RefCell;
use std::rc::Rc;

use eframe::egui;
use rust_i18n::t;
use yinhe_editor_core::config::SfEntry;

use crate::app::App;

/// 显示 XSynth 配置窗口；返回 true = 配置有变更（调用方需重载音频）。
pub(crate) fn show_viewport(app: &mut App, ctx: &egui::Context) -> bool {
    let Some(channel) = app.mix.xsynth_config_for else {
        return false;
    };
    let Some(idx) = app.workspace.active_doc else {
        return false;
    };

    // 快照出本帧要编辑的数据（闭包内不借用 app）。
    let overrides = app.workspace.documents[idx]
        .edit
        .project_sf
        .overrides
        .clone();
    let global_entries = app.audio_settings.global_sf_config.entries.clone();
    let label = crate::mix::channel_label(channel);
    let title = format!("XSynth · {label}");

    let viewport_id = egui::ViewportId::from_hash_of(("xsynth_config_dialog", channel));
    let open_rc = Rc::new(RefCell::new(true));
    let open_out = Rc::clone(&open_rc);
    let changed_rc = Rc::new(RefCell::new(false));
    let changed_out = Rc::clone(&changed_rc);
    let ctx_clone = ctx.clone();

    let overrides_rc = Rc::new(RefCell::new(overrides));
    let overrides_out = Rc::clone(&overrides_rc);

    ctx_clone.show_viewport_immediate(
        viewport_id,
        crate::chrome::dialog::viewport_builder(&title, [380.0, 440.0], true),
        move |vctx, _class| {
            let mut overrides = overrides_rc.borrow_mut();
            let mut use_global = !overrides.iter().any(|(c, _)| *c == channel);
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
                    crate::chrome::dialog::title_bar(ui, &title, &mut close, false);
                    egui::Frame::new()
                        .inner_margin(egui::Margin {
                            left: 12,
                            right: 12,
                            top: 4,
                            bottom: 12,
                        })
                        .show(ui, |ui| {
                            // ── 模式选择：默认（全局） / 此通道专用 ──
                            let mut switch_to_global = false;
                            let mut switch_to_channel = false;
                            if ui
                                .radio_value(
                                    &mut use_global,
                                    true,
                                    t!("soundfont.use_global").as_ref(),
                                )
                                .changed()
                            {
                                switch_to_global = true;
                            }
                            if ui
                                .radio_value(
                                    &mut use_global,
                                    false,
                                    t!("soundfont.use_channel").as_ref(),
                                )
                                .changed()
                            {
                                switch_to_channel = true;
                            }
                            if switch_to_global {
                                overrides.retain(|(c, _)| *c != channel);
                                *changed_rc.borrow_mut() = true;
                            }
                            if switch_to_channel {
                                if !overrides.iter().any(|(c, _)| *c == channel) {
                                    overrides.push((channel, Vec::new()));
                                }
                                *changed_rc.borrow_mut() = true;
                            }

                            ui.add_space(6.0);
                            ui.separator();
                            ui.add_space(6.0);

                            if use_global {
                                ui.label(
                                    egui::RichText::new(t!("soundfont.global_hint"))
                                        .size(crate::theme::SMALL_FONT)
                                        .color(crate::theme::text_secondary()),
                                );
                                ui.add_space(6.0);
                                ui.label(
                                    egui::RichText::new(t!("soundfont.global_current"))
                                        .strong()
                                        .color(crate::theme::text_bright()),
                                );
                                ui.add_space(4.0);
                                if global_entries.is_empty() {
                                    ui.label(
                                        egui::RichText::new(t!("soundfont.not_configured"))
                                            .color(crate::theme::text_muted()),
                                    );
                                } else {
                                    for e in &global_entries {
                                        let color = if e.enabled {
                                            crate::theme::text_primary()
                                        } else {
                                            crate::theme::text_disabled()
                                        };
                                        ui.label(
                                            egui::RichText::new(&e.name)
                                                .size(crate::theme::SMALL_FONT)
                                                .color(color),
                                        );
                                    }
                                }
                                ui.add_space(6.0);
                                ui.label(
                                    egui::RichText::new(t!("soundfont.edit_in_settings"))
                                        .size(crate::theme::SMALL_FONT)
                                        .color(crate::theme::text_muted()),
                                );
                            } else {
                                // 工具栏在列表上方（sf_list 的滚动区占满剩余高度）。
                                ui.horizontal(|ui| {
                                    let entries = overrides
                                        .iter_mut()
                                        .find(|(c, _)| *c == channel)
                                        .map(|(_, e)| e);
                                    let Some(entries) = entries else {
                                        return;
                                    };
                                    if ui.button(t!("soundfont.add").as_ref()).clicked()
                                        && let Some(paths) = rfd::FileDialog::new()
                                            .add_filter("SoundFont", &["sf2", "sf3", "sfz"])
                                            .pick_files()
                                    {
                                        for path in paths {
                                            let name = path
                                                .file_stem()
                                                .and_then(|n| n.to_str())
                                                .unwrap_or("SoundFont")
                                                .to_string();
                                            entries.push(SfEntry {
                                                path: path.to_string_lossy().to_string(),
                                                name,
                                                enabled: true,
                                            });
                                        }
                                        *changed_rc.borrow_mut() = true;
                                    }
                                    if ui.button(t!("common.clear").as_ref()).clicked() {
                                        entries.clear();
                                        *changed_rc.borrow_mut() = true;
                                    }
                                });
                                ui.add_space(4.0);
                                let salt = format!("chan{channel}");
                                let entries = overrides
                                    .iter_mut()
                                    .find(|(c, _)| *c == channel)
                                    .map(|(_, e)| e);
                                if let Some(entries) = entries
                                    && crate::right_panel::sf_list::sf_list(ui, entries, &salt)
                                {
                                    *changed_rc.borrow_mut() = true;
                                }
                            }
                        });
                });
            if close {
                vctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                *open_out.borrow_mut() = false;
            }
        },
    );

    let changed = *changed_out.borrow();
    if changed {
        // 变更写回工程（保存快照再写入 .yin）。
        let overrides = overrides_out.borrow();
        app.workspace.documents[idx].edit.project_sf.overrides = overrides.clone();
    }
    if !*open_rc.borrow() {
        app.mix.xsynth_config_for = None;
    }
    changed
}
