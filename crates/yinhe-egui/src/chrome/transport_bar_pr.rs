//! transport bar 右侧的 PR 控制组（原 PR 顶部 control bar 功能迁入，control bar 已删除）：
//! 音轨名称（主音轨切换 + 显示音轨勾选）/ 多轨道显示（幽灵）/ 瀑布流方向 / 和弦指示器。
//! 量化按钮在时间码左侧，按聚焦视图显示 AR/PR 量化（见 transport_bar）。
//!
//! 布局（视觉从左到右）：音轨名、幽灵、III（纵向）、三（横向）、和弦；
//! 在 `right_to_left` 布局中绘制，整组贴右且不参与工具收拢。
//!
//! 本模块不持有/修改文档状态，只产生 [PrBarEvent]，由 main_loop 应用。

use eframe::egui;
use egui_material_icons::MaterialIcon;
use egui_material_icons::icons::{ICON_DEHAZE, ICON_KEYBOARD_ARROW_DOWN, ICON_MASKED_TRANSITIONS};
use rust_i18n::t;

use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_types::Orientation;

/// 右侧控制组元素间距（紧凑，与工具区的 add_space(2.0) 同量级）。
pub(crate) const GAP: f32 = 4.0;

/// PR 控制事件（由 main_loop 应用到 doc.edit）。
pub enum PrBarEvent {
    /// 切换主音轨：track_selected 替换为仅此轨。
    SwitchMainTrack(u16),
    /// 设置某轨显示开关（track_pianoroll_visible[t]）。
    SetTrackVisible(u16, bool),
    /// 全选/清空显示音轨（track_pianoroll_visible 全部置为同一值）。
    SetAllVisible(bool),
}

/// PR 控制数据（全部只读；状态修改走事件）。
pub struct PrBarData<'a> {
    pub ppq: u32,
    pub quantize: QuantizePreset,
    /// 轨道显示信息缓存（edit.track_cache.info，含 Conductor 行）。
    pub track_infos: &'a [yinhe_core::TrackInfo],
    /// PR 显示音轨勾选状态（edit.track_pianoroll_visible，popup 右半写它）。
    /// 与 AR 显隐（track_visible）分离，互不影响。
    pub pr_track_visible: &'a [bool],
    /// 主音轨（= 选中轨索引最小者；无选中 = None，不显示回退轨）。
    pub main_track: Option<u16>,
    /// 和弦指示器文本（实时 MIDI 按键优先，其次播放中光标处和弦）。
    pub chord: Option<String>,
}

/// 小号文字字体（量化/音轨名，跟随 transport 工具栏字号体系）。
pub(crate) fn small_font() -> egui::FontId {
    egui::FontId::proportional(crate::theme::SMALL_FONT)
}

/// 图标字体（transport 按钮统一字号）。
pub(crate) fn icon_font(icon: MaterialIcon) -> egui::FontId {
    egui::FontId::new(crate::theme::TRANSPORT_BTN_FONT, icon.font_family())
}

/// 测量文本/图标排版宽度（与 hover_button 的分配尺寸同源，保证收拢判断准确）。
pub(crate) fn measure(ui: &egui::Ui, text: &str, font: egui::FontId) -> f32 {
    ui.painter()
        .layout_no_wrap(text.to_owned(), font, egui::Color32::PLACEHOLDER)
        .size()
        .x
}

/// 单个图标按钮宽度。
pub(crate) fn icon_width(ui: &egui::Ui, icon: MaterialIcon) -> f32 {
    measure(ui, icon.codepoint, icon_font(icon))
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

/// 当前主音轨显示名（无主音轨 = 「无轨」）。
fn track_name(data: &PrBarData<'_>) -> String {
    data.main_track
        .and_then(|t| data.track_infos.iter().find(|i| i.index == t))
        .map(track_label)
        .unwrap_or_else(|| t!("pr_bar.no_track").to_string())
}

/// 右侧控制组总宽（含元素间距）：供工具区判断是否收拢。
pub(crate) fn right_group_width(ui: &egui::Ui, data: &PrBarData<'_>) -> f32 {
    let mut widths = vec![
        measure(ui, &track_name(data), small_font())
            + 6.0
            + measure(
                ui,
                ICON_KEYBOARD_ARROW_DOWN.codepoint,
                icon_font(ICON_KEYBOARD_ARROW_DOWN),
            ),
        icon_width(ui, ICON_MASKED_TRANSITIONS),
        icon_width(ui, ICON_DEHAZE),
        icon_width(ui, ICON_DEHAZE),
    ];
    if let Some(chord) = &data.chord {
        widths.push(measure(ui, chord, chord_font()));
    }
    widths.iter().sum::<f32>() + GAP * (widths.len() - 1) as f32
}

fn chord_font() -> egui::FontId {
    egui::FontId::proportional(crate::theme::BODY_FONT)
}

/// 绘制右侧控制组并产生事件（整组在 `right_to_left` 布局中贴右；不收拢）。
pub(crate) fn show_right_group(
    ui: &mut egui::Ui,
    data: &PrBarData<'_>,
    vertical: bool,
    events: &mut Vec<PrBarEvent>,
    set_orientation: &mut Option<Orientation>,
    hovered_hint: &mut Option<String>,
) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        // ── 和弦指示器（最右，只读文本）──
        if let Some(chord) = &data.chord {
            let galley = ui.painter().layout_no_wrap(
                chord.clone(),
                chord_font(),
                egui::Color32::PLACEHOLDER,
            );
            let (rect, _) = ui.allocate_exact_size(galley.size(), egui::Sense::hover());
            ui.painter()
                .galley(rect.min, galley, crate::theme::text_primary());
            ui.add_space(GAP);
        }

        // ── 三 = 横向瀑布流 ──
        let h_resp = crate::widgets::hover::hover_button(
            ui,
            ICON_DEHAZE.codepoint,
            icon_font(ICON_DEHAZE),
            crate::theme::text_label(),
            !vertical,
        );
        if h_resp.clicked() && vertical {
            *set_orientation = Some(Orientation::Horizontal);
        }
        if h_resp.hovered() {
            *hovered_hint = Some(t!("hint.orientation.horizontal").to_string());
        }
        ui.add_space(GAP);

        // ── III = 纵向瀑布流（同一图标旋转 90°）──
        let v_resp = crate::widgets::hover::hover_button_rotated(
            ui,
            ICON_DEHAZE.codepoint,
            icon_font(ICON_DEHAZE),
            crate::theme::text_label(),
            vertical,
            std::f32::consts::FRAC_PI_2,
        );
        if v_resp.clicked() && !vertical {
            *set_orientation = Some(Orientation::Vertical);
        }
        if v_resp.hovered() {
            *hovered_hint = Some(t!("hint.orientation.vertical").to_string());
        }
        ui.add_space(GAP);

        // ── 多轨道显示（幽灵切换）──
        ghost_button(ui, data, events);
        ui.add_space(GAP);

        // ── 音轨名称（点击弹出主音轨/显示音轨 popup）──
        track_button(ui, data, events);
    });
}

/// 多轨道显示（幽灵）按钮：有任意非主轨可见 = 开（高亮），点击批量显/隐。
fn ghost_button(
    ui: &mut egui::Ui,
    data: &PrBarData<'_>,
    events: &mut Vec<PrBarEvent>,
) -> egui::Response {
    let show_others = data
        .pr_track_visible
        .iter()
        .enumerate()
        .any(|(i, &v)| v && Some(i as u16) != data.main_track);
    let resp = crate::widgets::hover::hover_button(
        ui,
        ICON_MASKED_TRANSITIONS.codepoint,
        icon_font(ICON_MASKED_TRANSITIONS),
        crate::theme::text_label(),
        show_others,
    );
    if resp.clicked() {
        // 开→关：仅主轨可见（其余 false，主轨仍由 pr_visible 强制可见）；关→开：全部 true
        events.push(PrBarEvent::SetAllVisible(!show_others));
    }
    if resp.hovered() {
        let tip = if show_others {
            t!("pr_bar.show_others_on")
        } else {
            t!("pr_bar.show_others_off")
        };
        resp.on_hover_text(tip)
    } else {
        resp
    }
}

/// 音轨名称按钮（文字 + 下拉箭头，无背景；点击弹出音轨 popup）。
fn track_button(
    ui: &mut egui::Ui,
    data: &PrBarData<'_>,
    events: &mut Vec<PrBarEvent>,
) -> egui::Response {
    let name = track_name(data);
    let text_galley = ui
        .painter()
        .layout_no_wrap(name, small_font(), egui::Color32::PLACEHOLDER);
    let icon_galley = ui.painter().layout_no_wrap(
        ICON_KEYBOARD_ARROW_DOWN.codepoint.to_owned(),
        icon_font(ICON_KEYBOARD_ARROW_DOWN),
        egui::Color32::PLACEHOLDER,
    );
    let size = egui::vec2(
        text_galley.size().x + 6.0 + icon_galley.size().x,
        text_galley.size().y.max(icon_galley.size().y),
    );
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    // 与 hover_button 共用同一四态配色（时间码右侧控件配色方案统一）。
    let color = crate::widgets::hover::hover_button_color(&resp, crate::theme::text_label(), false);
    let cy = rect.center().y;
    ui.painter().galley(
        egui::pos2(rect.min.x, cy - text_galley.size().y * 0.5),
        text_galley,
        color,
    );
    ui.painter().galley(
        egui::pos2(
            rect.min.x + size.x - icon_galley.size().x,
            cy - icon_galley.size().y * 0.5,
        ),
        icon_galley,
        color,
    );

    egui::Popup::from_toggle_button_response(&resp)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| track_popup(ui, data, events));
    resp
}

/// 音轨 popup：左半切换主音轨，右半勾选显示音轨（首项「全选」）。
///
/// 注意：两栏宽度必须 min/max 同时锁死——menu_item_button 的宽度 =
/// available_width（铺满整行），popup 又是内容自适应宽度，只设 min 会形成
/// 「按钮请求可用宽度 → popup 变宽 → 可用宽度更大」的每帧正反馈，popup 向右飞出去。
fn track_popup(ui: &mut egui::Ui, data: &PrBarData<'_>, events: &mut Vec<PrBarEvent>) {
    ui.set_max_height(560.0);
    // 行高/间隙与通用 menu 24/4 统一，右栏 checkbox 同高
    ui.spacing_mut().item_spacing.y = 4.0;
    ui.spacing_mut().item_spacing.x = 4.0;
    ui.spacing_mut().interact_size.y = 24.0;
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
                    for info in data.track_infos {
                        let is_main = data.main_track == Some(info.index);
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
                    let n = data.pr_track_visible.len();
                    let all = n > 0 && data.pr_track_visible.iter().all(|&v| v);
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
                    for info in data.track_infos {
                        let mut vis = data
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
