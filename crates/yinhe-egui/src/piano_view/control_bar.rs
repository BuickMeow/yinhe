//! PR 控制栏：最顶部、时间标尺上方的一栏（egui 绘制，不进 GPU），紧贴窗口顶部，时间标尺在其下贴内容以便查看/跳转。
//!
//! 内容（左→右）：量化按钮（原左上角，移至此处）、允许重叠开关、
//! 显示其他音轨（masked transitions 一键显/隐幽灵）、
//! 音轨名称（点击弹出左右两栏 popup：左半切换主音轨，右半勾选显示音轨，
//! 首项「全选」）、和弦指示器（右对齐）。
//!
//! 样式与底部 mode_bar 看齐：背景 `app_bg`、无背景 hover，仅前景三态变色。
//! 栏本身不持有/修改文档状态，只产生 [PrBarEvent]，由 app/layout.rs 应用。

use eframe::egui;
use rust_i18n::t;

use yinhe_editor_core::quantize::QuantizePreset;

/// 控制栏事件（由 layout 应用到 doc.edit）。
pub enum PrBarEvent {
    /// 量化预设变更。
    Quantize(QuantizePreset),
    /// 切换主音轨：track_selected 替换为仅此轨。
    SwitchMainTrack(u16),
    /// 设置某轨显示开关（track_visible[t]）。
    SetTrackVisible(u16, bool),
    /// 全选/清空显示音轨（track_visible 全部置为同一值）。
    SetAllVisible(bool),
}

/// 控制栏输入（全部只读；状态修改走事件）。
pub struct PrBarData<'a> {
    pub ppq: u32,
    pub quantize: QuantizePreset,
    /// 轨道显示信息缓存（edit.track_info_cache，含 Conductor 行）。
    pub track_infos: &'a [yinhe_core::TrackInfo],
    /// PR 显示音轨勾选状态（edit.track_pianoroll_visible，popup 右半写它）。
    /// 与 AR 显隐（track_visible）分离，互不影响。
    pub pr_track_visible: &'a [bool],
    /// 主音轨（= 选中轨索引最小者；无选中 = None，不显示回退轨）。
    pub main_track: Option<u16>,
    /// 和弦指示器文本（实时 MIDI 按键优先，其次播放中光标处和弦）。
    pub chord: Option<&'a str>,
}

/// 轨道行显示名：未命名轨用「轨道 #n (未命名)」。
fn track_label(info: &yinhe_core::TrackInfo) -> String {
    if info.name.is_empty() {
        t!("event_browser.track_unnamed", n = info.index).to_string()
    } else {
        t!(
            "event_browser.track_named",
            n = info.index,
            name = &info.name
        )
        .to_string()
    }
}

/// 绘制控制栏并处理交互，事件推入 `events`。
/// 样式与底部 mode_bar 看齐：栏背景 `app_bg`，所有图标/文字仅前景变色、无背景 hover（`hover_button` 同款三态）。
pub fn show(ui: &mut egui::Ui, bar: egui::Rect, ctx: &PrBarData<'_>, events: &mut Vec<PrBarEvent>) {
    // 背景与 mode_bar 一致（app_bg）+ 底部 1px 分隔线
    ui.painter().rect_filled(bar, 0.0, crate::theme::app_bg());
    ui.painter().hline(
        bar.min.x..=bar.max.x,
        bar.max.y - 0.5,
        egui::Stroke::new(1.0, crate::theme::line_fg()),
    );

    // ── 量化按钮（原 PR 左上角，移至本栏最左；无背景、仅前景变色）──
    let quant_rect = egui::Rect::from_min_size(
        bar.min + egui::vec2(4.0, 0.0),
        egui::vec2(28.0, bar.height()),
    );
    let quant_resp = ui.interact(
        quant_rect,
        egui::Id::new("pr_bar_quantize_btn"),
        egui::Sense::click(),
    );
    let quant_hovered = quant_resp.hovered();
    let quant_color = if quant_hovered {
        crate::theme::contrast_fg()
    } else {
        crate::theme::mode_bar_text()
    };
    ui.painter().text(
        quant_rect.center(),
        egui::Align2::CENTER_CENTER,
        ctx.quantize.label(),
        egui::FontId::proportional(crate::scaling::scaled_font(
            ui.ctx(),
            crate::theme::SMALL_FONT,
        )),
        quant_color,
    );
    // popup 复用 quantize 弹窗（无背景版，AR 仍走原 quantize_button 保留 track_bg）。
    let mut pending_q = None;
    egui::Popup::from_toggle_button_response(&quant_resp)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            crate::widgets::quantize_popup::show(ui, ctx.ppq, ctx.quantize, &mut pending_q);
        });
    if let Some(q) = pending_q {
        events.push(PrBarEvent::Quantize(q));
    }
    if quant_hovered {
        quant_resp.on_hover_text(ctx.quantize.label());
    }

    // ── 「显示其他音轨」幽灵切换（masked transitions；量化右侧）──
    // 有任意非主轨可见 = 开（高亮），否则关；点击走 SetAllVisible 批量切换。
    let show_others = ctx
        .pr_track_visible
        .iter()
        .enumerate()
        .any(|(i, &v)| v && Some(i as u16) != ctx.main_track);
    let ghost_rect = egui::Rect::from_min_size(
        egui::pos2(quant_rect.max.x + 2.0, bar.min.y),
        egui::vec2(18.0, bar.height()),
    );
    let ghost_resp = ui.interact(
        ghost_rect,
        egui::Id::new("pr_bar_show_others_btn"),
        egui::Sense::click(),
    );
    let ghost_hovered = ghost_resp.hovered();
    let ghost_icon = egui_material_icons::icons::ICON_MASKED_TRANSITIONS;
    let ghost_color = if show_others {
        if ghost_hovered {
            crate::theme::contrast_fg()
        } else {
            crate::theme::accent_active()
        }
    } else if ghost_hovered {
        crate::theme::contrast_fg()
    } else {
        crate::theme::mode_bar_text()
    };
    ui.painter().text(
        ghost_rect.center(),
        egui::Align2::CENTER_CENTER,
        ghost_icon.codepoint,
        egui::FontId::new(crate::theme::ICON_FONT, ghost_icon.font_family()),
        ghost_color,
    );
    if ghost_resp.clicked() {
        // 开→关：仅主轨可见（其余 false，主轨仍由 pr_visible 强制可见）；关→开：全部 true
        events.push(PrBarEvent::SetAllVisible(!show_others));
    }
    if ghost_hovered {
        let tip = if show_others {
            t!("pr_bar.show_others_on")
        } else {
            t!("pr_bar.show_others_off")
        };
        ghost_resp.on_hover_text(tip);
    }

    // ── 音轨名称按钮（点击弹出左右两栏 popup；无背景）──
    let name = ctx
        .main_track
        .and_then(|t| ctx.track_infos.iter().find(|i| i.index == t))
        .map(track_label)
        .unwrap_or_else(|| t!("pr_bar.no_track").to_string());
    // 名称 + 下拉箭头（material 图标，不用 Unicode 字符，避免字体缺字显示方框）。
    let font = egui::FontId::proportional(crate::scaling::scaled_font(
        ui.ctx(),
        crate::theme::SMALL_FONT,
    ));
    let icon = egui_material_icons::icons::ICON_KEYBOARD_ARROW_DOWN;
    let icon_font = egui::FontId::new(
        crate::scaling::scaled_font(ui.ctx(), crate::theme::ICON_FONT),
        icon.font_family(),
    );
    let text_w = ui
        .painter()
        .layout_no_wrap(name.clone(), font.clone(), crate::theme::mode_bar_text())
        .size()
        .x;
    let icon_w = ui
        .painter()
        .layout_no_wrap(
            icon.codepoint.to_string(),
            icon_font.clone(),
            crate::theme::mode_bar_text(),
        )
        .size()
        .x;
    let btn_rect = egui::Rect::from_min_size(
        egui::pos2(ghost_rect.max.x + 6.0, bar.min.y + 2.0),
        egui::vec2(text_w + 6.0 + icon_w + 16.0, bar.height() - 4.0),
    );
    let btn_resp = ui.interact(
        btn_rect,
        egui::Id::new("pr_bar_track_btn"),
        egui::Sense::click(),
    );
    // 无背景，仅前景变色（与 mode_bar 一致）
    let icon_color = if btn_resp.hovered() {
        crate::theme::contrast_fg()
    } else {
        crate::theme::mode_bar_text()
    };
    let text_x = btn_rect.min.x + 8.0;
    let cy = btn_rect.center().y;
    ui.painter().text(
        egui::pos2(text_x, cy),
        egui::Align2::LEFT_CENTER,
        name,
        font,
        icon_color,
    );
    ui.painter().text(
        egui::pos2(text_x + text_w + 6.0, cy),
        egui::Align2::LEFT_CENTER,
        icon.codepoint,
        icon_font,
        icon_color,
    );

    egui::Popup::from_toggle_button_response(&btn_resp)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| track_popup(ui, ctx, events));

    // ── 和弦指示器（右对齐）──
    if let Some(chord) = ctx.chord {
        ui.painter().text(
            egui::pos2(bar.max.x - 8.0, bar.center().y),
            egui::Align2::RIGHT_CENTER,
            chord,
            egui::FontId::proportional(crate::scaling::scaled_font(
                ui.ctx(),
                crate::theme::BODY_FONT,
            )),
            crate::theme::text_primary(),
        );
    }
}

/// 音轨 popup：左半切换主音轨，右半勾选显示音轨（首项「全选」）。
///
/// 注意：两栏宽度必须 min/max 同时锁死——menu_item_button 的宽度 =
/// available_width（铺满整行），popup 又是内容自适应宽度，只设 min 会形成
/// 「按钮请求可用宽度 → popup 变宽 → 可用宽度更大」的每帧正反馈，popup 向右飞出去。
fn track_popup(ui: &mut egui::Ui, ctx: &PrBarData<'_>, events: &mut Vec<PrBarEvent>) {
    ui.set_max_height(560.0);
    // 行高/间隙与通用 menu 24/4 统一，右栏 checkbox 同高
    ui.spacing_mut().item_spacing.y = 4.0;
    ui.spacing_mut().item_spacing.x = 4.0;
    ui.spacing_mut().interact_size.y = 24.0;
    // 字体跟随 font_scale（原硬编码 SMALL_FONT/ICON_FONT 不受缩放，改用 scaled_font）
    // 列表最小高度：音轨少时 popup 也不会缩成一小条（内容自适应高度的副作用）。
    const LIST_MIN_H: f32 = 280.0;
    ui.horizontal(|ui| {
        // ── 左半：切换主音轨（单击 = 选中仅此轨）──
        ui.vertical(|ui| {
            ui.set_min_width(170.0);
            ui.set_max_width(170.0);
            ui.label(t!("pr_bar.main_track"));
            ui.separator();
            egui::ScrollArea::vertical()
                .id_salt("pr_bar_main_list")
                .min_scrolled_height(LIST_MIN_H)
                .max_height(500.0)
                .show(ui, |ui| {
                    for info in ctx.track_infos {
                        let is_main = ctx.main_track == Some(info.index);
                        if ui
                            .add(crate::widgets::menu::menu_item_button(
                                ui,
                                is_main,
                                track_label(info),
                            ))
                            .clicked()
                        {
                            events.push(PrBarEvent::SwitchMainTrack(info.index));
                            ui.close();
                        }
                    }
                });
        });
        ui.separator();
        // ── 右半：显示音轨（首项「全选」+ 各轨勾选）──
        ui.vertical(|ui| {
            ui.set_min_width(170.0);
            ui.set_max_width(170.0);
            ui.label(t!("pr_bar.show_tracks"));
            ui.separator();
            egui::ScrollArea::vertical()
                .id_salt("pr_bar_visible_list")
                .min_scrolled_height(LIST_MIN_H)
                .max_height(500.0)
                .show(ui, |ui| {
                    let n = ctx.pr_track_visible.len();
                    let all = n > 0 && ctx.pr_track_visible.iter().all(|&v| v);
                    let mut checked = all;
                    if crate::widgets::checkbox::check_scope(ui, |ui| {
                        ui.checkbox(&mut checked, t!("pr_bar.select_all"))
                    })
                    .inner
                    .clicked()
                    {
                        events.push(PrBarEvent::SetAllVisible(!all));
                    }
                    ui.separator();
                    for info in ctx.track_infos {
                        let mut vis = ctx
                            .pr_track_visible
                            .get(info.index as usize)
                            .copied()
                            .unwrap_or(false);
                        if crate::widgets::checkbox::check_scope(ui, |ui| {
                            ui.checkbox(&mut vis, track_label(info))
                        })
                        .inner
                        .clicked()
                        {
                            events.push(PrBarEvent::SetTrackVisible(info.index, vis));
                        }
                    }
                });
        });
    });
}
