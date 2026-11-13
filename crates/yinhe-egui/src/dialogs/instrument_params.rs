//! 内置音源参数窗口（独立视口）：旋钮 + 分类。
//!
//! 参数存工程（`YinModel.synth_params`），不修改音色库；拖动时同步写模型
//! 并给音频线程发 `SetSynthParams`（滤波器类参数实时生效，包络/音高
//! 类参数对之后的新音符生效）。

use eframe::egui;
use rust_i18n::t;

use yinhe_types::{SYNTH_PARAMS, SynthParamCategory, SynthParams};

use crate::app::App;

/// 窗口 id（打开动作与窗口声明共用同一来源，保证 `raise_viewport` 命中）。
pub(crate) fn viewport_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("synth_params_panel")
}

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

    let id = viewport_id();
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
                            if body(ui, &mut params, &mut panel.page) {
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

/// 窗口主体（从 `show` 拆出，便于布局回归测试）。返回是否修改。
///
/// 每一行都用固定高度的 `allocate_ui_with_layout`：若直接把
/// `right_to_left` 挂在页面顶层，它会吃掉剩余全部高度，后续的分类与
/// 旋钮被排到窗口下方——窗口看起来就是"空的"。
fn body(ui: &mut egui::Ui, params: &mut SynthParams, page: &mut usize) -> bool {
    let mut changed = false;

    let btn_h = crate::widgets::control::h(ui.ctx());
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), btn_h),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if reset_icon(ui, t!("synth_params.reset_hint").as_ref()).clicked() {
                    *params = SynthParams::default();
                    changed = true;
                }
            });
        },
    );
    ui.separator();

    // 左分类 / 右参数。
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(118.0);
            ui.selectable_value(page, 0, t!("synth_params.all").to_string());
            for (i, cat) in SynthParamCategory::ALL.iter().enumerate() {
                ui.selectable_value(page, i + 1, category_name(*cat));
            }
        });
        ui.separator();
        // 显式给纵向布局：`ScrollArea` 会继承父 ui 的布局方向，直接挂在
        // `horizontal_top` 下会把所有参数行排成一行（超出窗口）。
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), ui.available_height()),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let cats: Vec<SynthParamCategory> = if *page == 0 {
                            SynthParamCategory::ALL.to_vec()
                        } else {
                            vec![SynthParamCategory::ALL[*page - 1]]
                        };
                        for cat in cats {
                            if *page == 0 {
                                ui.add_space(4.0);
                                ui.label(
                                    egui::RichText::new(category_name(cat))
                                        .size(crate::theme::SMALL_FONT + 1.0)
                                        .color(crate::theme::text_secondary()),
                                );
                            }
                            for info in SYNTH_PARAMS.iter().filter(|p| p.category == cat) {
                                if param_row(ui, info, params) {
                                    changed = true;
                                }
                            }
                        }
                    });
            },
        );
    });

    changed
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
    const KNOB_D: f32 = 34.0;
    let mut changed = false;
    let mut value = params.get(info.id);
    let mut norm = info.normalize(value);

    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), KNOB_D + 4.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            let resp = crate::widgets::knob::knob_centered(ui, &mut norm, KNOB_D);
            crate::widgets::a11y::name_slider(&resp, info.name);
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
        },
    );

    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable as _;

    /// 回归：所有参数、分类与重置按钮都必须落在窗口可视区内。
    ///
    /// 曾经页面顶层直接用 `ui.with_layout(right_to_left)`，它吃掉剩余高度，
    /// 参数列表整体被排到窗口下方，用户看到的是"空窗口 + 一个重置图标"。
    #[test]
    fn body_lays_out_every_control_inside_the_panel() {
        const W: f32 = 520.0;
        const H: f32 = 560.0;
        let mut params = SynthParams::default();
        let mut page = 0usize;
        let mut harness = egui_kittest::Harness::builder()
            .allow_missing_glyphs()
            .with_size(egui::vec2(W, H))
            .build_ui(move |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    body(ui, &mut params, &mut page);
                });
            });
        harness.step();

        // 名称文本与旋钮都带同一个可见名，两处都必须在窗口内。
        let mut prev_bottom = f32::NEG_INFINITY;
        for info in SYNTH_PARAMS {
            let nodes: Vec<_> = harness.get_all_by_label(info.name).collect();
            assert!(!nodes.is_empty(), "{} 没有渲染", info.name);
            let mut top = f32::INFINITY;
            for node in &nodes {
                let rect = node.rect();
                assert!(
                    rect.min.y >= 0.0 && rect.max.y <= H,
                    "{} 不在窗口内: {rect:?}",
                    info.name
                );
                top = top.min(rect.min.y);
            }
            assert!(
                top > prev_bottom,
                "{} 必须纵向排在上一行之后（y={top}，上一行底部 {prev_bottom}）",
                info.name
            );
            prev_bottom = nodes
                .iter()
                .map(|n| n.rect().max.y)
                .fold(f32::NEG_INFINITY, f32::max);
        }
        for cat in SynthParamCategory::ALL {
            let name = category_name(cat);
            let nodes: Vec<_> = harness.get_all_by_label(&name).collect();
            assert!(!nodes.is_empty(), "分类 {name} 没有渲染");
            for node in nodes {
                let rect = node.rect();
                assert!(
                    rect.min.y >= 0.0 && rect.max.y <= H,
                    "分类 {name} 不在窗口内: {rect:?}"
                );
            }
        }
    }
}
