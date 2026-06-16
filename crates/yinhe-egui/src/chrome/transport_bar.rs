use eframe::egui;
use egui_material_icons::icons::*;

use crate::file_loader::FileLoader;
use yinhe_editor_core::document::Document;
use yinhe_editor_core::quantize::QuantizePreset;
use crate::view_interaction::FollowMode;
use crate::util::time_format;

/// Actions triggered from the file menu dropdown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileAction {
    NewProject,
    Open,
    Save,
    SaveAs,
    CloseDocument,
    ExportAudio,
    ExportMidi,
    Settings,
    Exit,
}

struct MenuItem {
    icon: egui_material_icons::MaterialIcon,
    label: &'static str,
    action: FileAction,
    enabled: bool,
}

/// Aggregated input for the transport bar — replaces 12 positional parameters.
pub struct TransportContext<'a> {
    pub file_loader: &'a mut FileLoader,
    pub doc: Option<&'a Document>,
    pub cpu_usage: f32,
    pub mem_mb: f64,
    pub follow_mode: &'a mut FollowMode,
    pub show_mem_breakdown: &'a mut bool,
}

/// Output from the transport bar — replaces `&mut bool` out-parameters.
pub struct TransportResponse {
    pub toggle_play: bool,
    pub pause_return: bool,
    pub stop_play: bool,
    pub pending_quantize: Option<QuantizePreset>,
    pub pending_file_action: Option<FileAction>,
}

pub fn show(ui: &mut egui::Ui, ctx: &mut TransportContext<'_>) -> TransportResponse {
    let has_active = ctx.doc.is_some();

    let mut toggle_play = false;
    let mut pause_return = false;
    let mut stop_play = false;
    let mut pending_quantize = None;
    let mut pending_file_action = None;

    egui::Panel::top("transport_bar")
        .frame(egui::Frame {
            fill: crate::theme::APP_BG,
            inner_margin: egui::Margin {
                left: 8,
                right: 8,
                top: 0,
                bottom: 8,
            },
            stroke: egui::Stroke::NONE,
            ..Default::default()
        })
        .show_inside(ui, |ui| {
            // Taller buttons for the transport bar
            ui.spacing_mut().interact_size.y = 32.0;

            let mut timecode_rect: Option<egui::Rect> = None;
            let mut button_right: Option<f32> = None;

            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let btn_size = egui::vec2(32.0, 32.0);
                let btn_rounding = egui::CornerRadius::same(2);

                let file_btn = ui.add(
                    egui::Button::new(ICON_DESCRIPTION.rich_text().size(18.0))
                        .min_size(btn_size)
                        .corner_radius(btn_rounding),
                );
                show_file_menu(
                    &file_btn,
                    ctx.file_loader,
                    has_active,
                    &mut pending_file_action,
                );

                if has_active {
                    let is_playing = ctx.doc.map(|d| d.edit.playback.is_playing()).unwrap_or(false);

                    if ui
                        .add(
                            egui::Button::new(
                                (if is_playing {
                                    ICON_PAUSE
                                } else {
                                    ICON_PLAY_ARROW
                                })
                                .rich_text()
                                .size(18.0),
                            )
                            .min_size(btn_size)
                            .corner_radius(btn_rounding),
                        )
                        .clicked()
                    {
                        if is_playing {
                            pause_return = true;
                        } else {
                            toggle_play = true;
                        }
                    }

                    if ui
                        .add(
                            egui::Button::new(ICON_STOP.rich_text().size(18.0))
                                .min_size(btn_size)
                                .corner_radius(btn_rounding),
                        )
                        .clicked()
                    {
                        stop_play = true;
                    }

                    // ── Follow-mode button (cycle: None → Page → Continuous) ──
                    let follow_resp = ui.add(
                        egui::Button::new(ctx.follow_mode.icon().rich_text().size(18.0))
                            .min_size(btn_size)
                            .corner_radius(btn_rounding),
                    );
                    if follow_resp.clicked() {
                        *ctx.follow_mode = ctx.follow_mode.next();
                    }
                    follow_resp.on_hover_text(ctx.follow_mode.tooltip());

                    // ── Quantization preset button + popup ──
                    let ppq = ctx.doc.map(|d| d.midi().ticks_per_beat).unwrap_or(480);
                    let q_label = ctx
                        .doc
                        .map(|d| d.edit.quantize.button_text())
                        .unwrap_or_default();
                    let q_resp = ui.add(
                        egui::Button::new(q_label.as_str())
                            .min_size(egui::vec2(44.0, 32.0))
                            .corner_radius(btn_rounding),
                    );

                    egui::Popup::menu(&q_resp).show(|ui| {
                        ui.set_min_width(120.0);
                        for preset in QuantizePreset::ALL {
                            let active = ctx.doc.map(|d| *preset == d.edit.quantize).unwrap_or(false);
                            if ui
                                .add(egui::Button::selectable(active, preset.display_item(ppq)))
                                .clicked()
                            {
                                pending_quantize = Some(*preset);
                                ui.close();
                            }
                        }

                        ui.separator();

                        let is_custom = ctx
                            .doc
                            .map(|d| matches!(d.edit.quantize, QuantizePreset::Custom(_, _)))
                            .unwrap_or(false);
                        if ui
                            .add(egui::Button::selectable(is_custom, "Custom"))
                            .clicked()
                        {
                            pending_quantize = Some(QuantizePreset::Custom(1, 4));
                            ui.close();
                        }
                    });

                    // ── Custom fraction editor (visible only when Custom is selected) ──
                    if let Some(doc) = ctx.doc
                        && let QuantizePreset::Custom(ref num, ref den) = doc.edit.quantize
                    {
                        let mut edit_num = *num;
                        let mut edit_den = *den;

                        ui.add_space(2.0);
                        ui.label("n:");
                        let num_resp = ui.add(
                            egui::DragValue::new(&mut edit_num)
                                .range(1..=9999)
                                .speed(0.5)
                                .prefix("")
                                .max_decimals(0)
                                .fixed_decimals(0),
                        );
                        ui.label("d:");
                        let den_resp = ui.add(
                            egui::DragValue::new(&mut edit_den)
                                .range(1..=9999)
                                .speed(0.5)
                                .prefix("")
                                .max_decimals(0)
                                .fixed_decimals(0),
                        );

                        if num_resp.dragged()
                            || den_resp.dragged()
                            || num_resp.changed()
                            || den_resp.changed()
                        {
                            let edit_den = edit_den.max(1);
                            pending_quantize = Some(QuantizePreset::Custom(edit_num, edit_den));
                        }
                    }
                }

                if let Some(doc) = ctx.doc {
                    button_right = Some(ui.cursor().min.x);
                    timecode_rect = Some(show_timecode_display(
                        ui,
                        doc,
                        ctx.cpu_usage,
                        ctx.mem_mb,
                        ctx.show_mem_breakdown,
                    ));
                }
            });

            // ── Double-click transport bar blank area to toggle maximize/restore ──
            // Only triggers on the background gaps (between buttons and timecode,
            // and after timecode to the right edge), NOT on buttons or timecode.
            let double_clicked = ui.input(|i| {
                i.pointer
                    .button_double_clicked(egui::PointerButton::Primary)
            });
            if double_clicked {
                let bar_rect = ui.max_rect();
                if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                    let in_bar = bar_rect.contains(pos);
                    let in_timecode = timecode_rect
                        .map(|r: egui::Rect| r.contains(pos))
                        .unwrap_or(false);
                    let in_buttons = button_right
                        .map(|r: f32| pos.x >= bar_rect.min.x && pos.x < r)
                        .unwrap_or(false);
                    if in_bar && !in_timecode && !in_buttons {
                        let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
                        ui.ctx()
                            .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                    }
                }
            }
        });

    TransportResponse {
        toggle_play,
        pause_return,
        stop_play,
        pending_quantize,
        pending_file_action,
    }
}

/// Show the file menu popup. Extracted from `show` for readability.
fn show_file_menu(
    button: &egui::Response,
    file_loader: &FileLoader,
    has_active: bool,
    pending_action: &mut Option<FileAction>,
) {
    egui::Popup::menu(button).show(|ui| {
        ui.set_min_width(180.0);

        fn menu_items(
            ui: &mut egui::Ui,
            items: &[MenuItem],
            pending_action: &mut Option<FileAction>,
        ) {
            for item in items {
                let icon_color = if item.enabled {
                    egui::Color32::WHITE
                } else {
                    egui::Color32::from_gray(80)
                };
                let resp = ui.add_enabled(
                    item.enabled,
                    egui::Button::selectable(
                        false,
                        egui::RichText::new(format!("      {}", item.label)).size(14.0),
                    ),
                );
                if resp.clicked() {
                    *pending_action = Some(item.action);
                    ui.close();
                }
                let icon_pos = egui::pos2(resp.rect.min.x + 4.0, resp.rect.center().y);
                ui.painter().text(
                    icon_pos,
                    egui::Align2::LEFT_CENTER,
                    item.icon.codepoint,
                    egui::FontId::new(16.0, item.icon.font_family()),
                    icon_color,
                );
            }
        }

        let items = [
            MenuItem {
                icon: ICON_NOTE_ADD,
                label: "新建工程",
                action: FileAction::NewProject,
                enabled: !file_loader.is_loading(),
            },
            MenuItem {
                icon: ICON_FOLDER_OPEN,
                label: "打开",
                action: FileAction::Open,
                enabled: !file_loader.is_loading(),
            },
            MenuItem {
                icon: ICON_SAVE,
                label: "保存",
                action: FileAction::Save,
                enabled: has_active,
            },
            MenuItem {
                icon: ICON_SAVE_ALT,
                label: "另存为",
                action: FileAction::SaveAs,
                enabled: has_active,
            },
            MenuItem {
                icon: ICON_CLOSE,
                label: "关闭",
                action: FileAction::CloseDocument,
                enabled: has_active,
            },
        ];
        menu_items(ui, &items, pending_action);

        ui.separator();

        let export_items = [
            MenuItem {
                icon: ICON_AUDIO_FILE,
                label: "导出音频",
                action: FileAction::ExportAudio,
                enabled: has_active,
            },
            MenuItem {
                icon: ICON_MUSIC_NOTE,
                label: "导出MIDI",
                action: FileAction::ExportMidi,
                enabled: has_active,
            },
        ];
        menu_items(ui, &export_items, pending_action);

        ui.separator();

        let misc_items = [
            MenuItem {
                icon: ICON_SETTINGS,
                label: "设置",
                action: FileAction::Settings,
                enabled: true,
            },
            MenuItem {
                icon: ICON_EXIT_TO_APP,
                label: "退出",
                action: FileAction::Exit,
                enabled: true,
            },
        ];
        menu_items(ui, &misc_items, pending_action);
    });
}

/// Show the timecode display panel. Returns the allocated rect.
fn show_timecode_display(
    ui: &mut egui::Ui,
    doc: &Document,
    cpu_usage: f32,
    mem_mb: f64,
    show_mem_breakdown: &mut bool,
) -> egui::Rect {
    let tick = doc.edit.cursor_tick.unwrap_or(0.0);
    let seconds = doc.midi().tick_to_seconds(tick as u64);
    let bpm = doc.midi().bpm_at_time(seconds);
    let (num, _denom_power) = doc.midi().time_sig_at_tick(tick as u32);
    let ppq = doc.midi().ticks_per_beat;

    let bpm_str = time_format::format_bpm(bpm);
    let ts_str = format!(
        "{}  {}",
        time_format::format_time_sig(num, _denom_power),
        ppq
    );
    let time_str = time_format::format_time(seconds);
    let pos_str = time_format::format_tick_bar_beat(tick, ppq, num);

    let col_widths = [70.0, 76.0, 90.0];
    let rect_h = 36.0;
    let rect_w = col_widths.iter().sum::<f32>();
    let bar_cx = ui.max_rect().center().x;
    let cursor_x = ui.cursor().min.x;
    let rect_l = bar_cx - rect_w * 0.5;
    let pad = (rect_l - cursor_x).max(0.0);
    ui.add_space(pad);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(rect_w, rect_h), egui::Sense::hover());

    let c = crate::theme::ACCENT_ACTIVE;
    let font = egui::FontId::proportional(12.0);
    let grid = egui::Stroke::new(1.0, egui::Color32::from_gray(60));

    ui.painter()
        .rect_filled(rect, egui::CornerRadius::same(8), egui::Color32::BLACK);

    let texts_top = [format!("{:.1}%", cpu_usage), bpm_str, pos_str];
    let texts_bot = [format!("{:.1} MB", mem_mb), ts_str, time_str];

    let mut col_x = rect.min.x;
    for i in 0..3 {
        let cx = col_x + col_widths[i] * 0.5;
        if i > 0 {
            ui.painter().line_segment(
                [egui::pos2(col_x, rect.min.y), egui::pos2(col_x, rect.max.y)],
                grid,
            );
        }
        let top_pos = egui::pos2(cx, rect.min.y + rect_h * 0.25);
        let bot_pos = egui::pos2(cx, rect.min.y + rect_h * 0.75);
        ui.painter().text(
            top_pos,
            egui::Align2::CENTER_CENTER,
            &texts_top[i],
            font.clone(),
            c,
        );
        ui.painter().text(
            bot_pos,
            egui::Align2::CENTER_CENTER,
            &texts_bot[i],
            font.clone(),
            c,
        );
        if i == 0 {
            let click_rect = egui::Rect::from_center_size(
                bot_pos,
                egui::vec2(col_widths[i] - 4.0, rect_h * 0.45),
            );
            let resp = ui.interact(click_rect, ui.id().with("mem_text"), egui::Sense::click());
            if resp.clicked() {
                *show_mem_breakdown = true;
            }
            resp.on_hover_text("点击打开内存占用详情");
        }
        col_x += col_widths[i];
    }

    rect
}
