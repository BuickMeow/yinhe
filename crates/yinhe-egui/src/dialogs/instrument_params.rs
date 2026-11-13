//! 内置音源参数窗口（独立视口）：旋钮 + 分类。
//!
//! 参数存工程（`YinModel.synth_params`），不修改音色库；拖动时同步写模型
//! 并给音频线程发 `SetSynthParams`（滤波器类参数实时生效，包络/音高
//! 类参数对之后的新音符生效）。

use eframe::egui;
use rust_i18n::t;

use yinhe_types::{SYNTH_PARAMS, SynthParamCategory, SynthParamId, SynthParams, synth_param_info};

use crate::app::App;

pub(crate) struct InstrumentParamsPanel {
    channel: u8,
    /// 0 = 全部；1..=4 = 分类下标 + 1。
    page: usize,
}

impl InstrumentParamsPanel {
    pub(crate) fn new(channel: u8) -> Self {
        Self { channel, page: 0 }
    }
}

fn category_name(cat: SynthParamCategory) -> String {
    match cat {
        SynthParamCategory::Tone => t!("synth_params.tone").to_string(),
        SynthParamCategory::Level => t!("synth_params.level").to_string(),
        SynthParamCategory::Envelope => t!("synth_params.envelope").to_string(),
        SynthParamCategory::Pitch => t!("synth_params.pitch").to_string(),
    }
}

fn value_text(info: &yinhe_types::SynthParamInfo, value: f32) -> String {
    if info.unit.is_empty() {
        format!("{:.*}", info.decimals as usize, value)
    } else {
        format!("{:.*} {}", info.decimals as usize, value, info.unit)
    }
}

/// 每帧渲染窗口（未打开时无操作）。
pub(crate) fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut panel) = app.mix.synth_params_panel.take() else {
        return;
    };
    let Some(idx) = app.workspace.active_doc else {
        return;
    };

    let id = egui::ViewportId::from_hash_of("synth_params_panel");
    crate::chrome::dialog::raise_on_open(ctx, id);
    let title = format!(
        "{} — {}",
        t!("synth_params.title"),
        crate::mix::channel_label(panel.channel)
    );
    let mut close = false;
    ctx.show_viewport_immediate(
        id,
        crate::chrome::dialog::viewport_builder(title.as_ref(), [520.0, 560.0], true),
        |vctx, _class| {
            if vctx.input(|i| i.viewport().close_requested()) {
                close = true;
            }
            let mut closed = close;
            egui::CentralPanel::default()
                .frame(egui::Frame {
                    fill: crate::theme::app_bg(),
                    ..Default::default()
                })
                .show(vctx, |ui| {
                    crate::chrome::dialog::title_bar(ui, title.as_ref(), &mut closed, false);
                    egui::Frame::new()
                        .inner_margin(egui::Margin {
                            left: 12,
                            right: 12,
                            top: 0,
                            bottom: 12,
                        })
                        .show(ui, |ui| {
                            let Some(doc) = app.workspace.documents.get_mut(idx) else {
                                return;
                            };
                            if panel.channel as usize >= 256 {
                                return;
                            }
                            let mut params = doc.data.model.synth_params_at(panel.channel as usize);
                            let mut changed = false;

                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if reset_icon(ui, t!("synth_params.reset_hint").as_ref())
                                        .clicked()
                                    {
                                        params = SynthParams::default();
                                        changed = true;
                                    }
                                },
                            );
                            ui.separator();

                            // 左分类 / 右参数。
                            ui.horizontal_top(|ui| {
                                ui.vertical(|ui| {
                                    ui.set_width(118.0);
                                    ui.selectable_value(
                                        &mut panel.page,
                                        0,
                                        t!("synth_params.all").to_string(),
                                    );
                                    for (i, cat) in SynthParamCategory::ALL.iter().enumerate() {
                                        ui.selectable_value(
                                            &mut panel.page,
                                            i + 1,
                                            category_name(*cat),
                                        );
                                    }
                                });
                                ui.separator();
                                egui::ScrollArea::vertical()
                                    .auto_shrink([false, false])
                                    .show(ui, |ui| {
                                        let cats: Vec<SynthParamCategory> = if panel.page == 0 {
                                            SynthParamCategory::ALL.to_vec()
                                        } else {
                                            vec![SynthParamCategory::ALL[panel.page - 1]]
                                        };
                                        for cat in cats {
                                            if panel.page == 0 {
                                                ui.add_space(4.0);
                                                ui.label(
                                                    egui::RichText::new(category_name(cat))
                                                        .size(crate::theme::SMALL_FONT + 1.0)
                                                        .color(crate::theme::text_secondary()),
                                                );
                                            }
                                            for info in
                                                SYNTH_PARAMS.iter().filter(|p| p.category == cat)
                                            {
                                                if param_row(ui, info, &mut params) {
                                                    changed = true;
                                                }
                                            }
                                        }
                                    });
                            });

                            if changed {
                                let model = std::sync::Arc::make_mut(&mut doc.data.model);
                                *model.synth_params_mut(panel.channel as usize) = params;
                                doc.mark_mixer_dirty();
                                if let Some(audio) = app.audio_state.handle.as_ref() {
                                    audio
                                        .handle
                                        .send(yinhe_audio::AudioCommand::SetSynthParams {
                                            channel: panel.channel,
                                            params,
                                        });
                                }
                            }
                        });
                });
            if closed {
                close = true;
            }
        },
    );
    if !close {
        app.mix.synth_params_panel = Some(panel);
    }
}

fn reset_icon(ui: &mut egui::Ui, hint: &str) -> egui::Response {
    let icon = egui_material_icons::icons::ICON_RESTART_ALT;
    ui.add(
        egui::Button::new(
            egui::RichText::new(icon.codepoint)
                .font(egui::FontId::new(14.0, icon.font_family()))
                .color(crate::theme::text_muted()),
        )
        .frame(false),
    )
    .on_hover_text(hint)
}

/// 一行参数：旋钮 + 名称 + 数值 + 单参数重置。返回是否修改。
fn param_row(
    ui: &mut egui::Ui,
    info: &yinhe_types::SynthParamInfo,
    params: &mut SynthParams,
) -> bool {
    let mut changed = false;
    let mut value = params.get(info.id);
    let mut norm = info.normalize(value);
    ui.horizontal(|ui| {
        let resp = crate::widgets::knob::knob_centered(ui, &mut norm, 34.0);
        if resp.changed() {
            value = info.denormalize(norm);
            params.set(info.id, value);
            changed = true;
        }
        ui.vertical(|ui| {
            ui.label(
                egui::RichText::new(info.name)
                    .size(crate::theme::SMALL_FONT + 1.0)
                    .color(crate::theme::text_primary()),
            );
            ui.label(
                egui::RichText::new(value_text(info, value))
                    .size(crate::theme::SMALL_FONT)
                    .color(crate::theme::text_secondary()),
            );
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if reset_icon(ui, t!("synth_params.reset_one").as_ref()).clicked() {
                params.set(info.id, SynthParams::default().get(info.id));
                changed = true;
            }
        });
    });
    changed
}

/// 参数 id 的显示名（测试/诊断用）。
#[allow(dead_code)]
pub(crate) fn param_name(id: SynthParamId) -> &'static str {
    synth_param_info(id).name
}
