use eframe::egui;
use rust_i18n::t;

pub use yinhe_audio::export::ExportProgress;

pub(crate) fn show_settings_viewport(
    ctx: &egui::Context,
    show: &mut bool,
    sample_rate: u32,
    bit_depth: &mut yinhe_audio::export::WavBitDepth,
    layer_count: &mut u32,
    export_sample_rate: &mut u32,
) -> bool {
    let viewport_id = egui::ViewportId::from_hash_of("export_settings_dialog");
    if !*show {
        return false;
    }

    let bd = std::rc::Rc::new(std::cell::Cell::new(*bit_depth));
    let lc = std::rc::Rc::new(std::cell::Cell::new(*layer_count));
    let sr = std::rc::Rc::new(std::cell::Cell::new(*export_sample_rate));
    let open = std::rc::Rc::new(std::cell::RefCell::new(true));
    let started = std::rc::Rc::new(std::cell::RefCell::new(false));
    let ctx_clone = ctx.clone();
    let open_cb = open.clone();
    let started_cb = started.clone();
    let bd_cb = bd.clone();
    let lc_cb = lc.clone();
    let sr_cb = sr.clone();

    ctx_clone.show_viewport_immediate(
        viewport_id,
        crate::chrome::dialog::viewport_builder(t!("dialog.export.settings_title").as_ref(), [320.0, 220.0], false),
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
                    crate::chrome::dialog::title_bar(ui, t!("dialog.export.settings_title").as_ref(), &mut close, false);
                    egui::Frame::new()
                        .inner_margin(egui::Margin {
                            left: 12,
                            right: 12,
                            top: 0,
                            bottom: 12,
                        })
                        .show(ui, |ui| {
                            ui.set_max_width(280.0);
                            let btn_zone_h = crate::chrome::dialog_buttons::btn_zone_h(ui.ctx());
                            crate::chrome::dialog::content_with_bottom_buttons(
                                ui,
                                btn_zone_h,
                                |ui| {
                                    ui.add_space(8.0);

                                    ui.horizontal(|ui| {
                                        ui.label(t!("dialog.export.bit_depth").as_ref());
                                        let bd = bd_cb.get();
                                        let current = match bd {
                                            yinhe_audio::export::WavBitDepth::Bit16 => "16-bit",
                                            yinhe_audio::export::WavBitDepth::Bit24 => "24-bit",
                                            yinhe_audio::export::WavBitDepth::Bit32Float => {
                                                "32-bit float"
                                            }
                                        };
                                        crate::widgets::combo::combo_box(
                                            ui,
                                            "export_bit_depth",
                                            current,
                                            160.0,
                                            |ui| {
                                                if crate::widgets::combo::combo_item(
                                                    ui,
                                                    bd == yinhe_audio::export::WavBitDepth::Bit16,
                                                    "16-bit",
                                                )
                                                .clicked()
                                                {
                                                    bd_cb.set(yinhe_audio::export::WavBitDepth::Bit16);
                                                }
                                                if crate::widgets::combo::combo_item(
                                                    ui,
                                                    bd == yinhe_audio::export::WavBitDepth::Bit24,
                                                    "24-bit",
                                                )
                                                .clicked()
                                                {
                                                    bd_cb.set(yinhe_audio::export::WavBitDepth::Bit24);
                                                }
                                                if crate::widgets::combo::combo_item(
                                                    ui,
                                                    bd == yinhe_audio::export::WavBitDepth::Bit32Float,
                                                    "32-bit float",
                                                )
                                                .clicked()
                                                {
                                                    bd_cb.set(
                                                        yinhe_audio::export::WavBitDepth::Bit32Float,
                                                    );
                                                }
                                            },
                                        );
                                    });

                                    ui.horizontal(|ui| {
                                        ui.label(t!("dialog.export.sample_rate").as_ref());
                                        let r = sr_cb.get();
                                        let sr_text = if r == 0 {
                                            t!("dialog.export.follow_global", n = sample_rate).to_string()
                                        } else {
                                            format!("{} Hz", r)
                                        };
                                        let sample_rates: [u32; 5] = [0, 44100, 48000, 96000, 192000];
                                        crate::widgets::combo::combo_box(
                                            ui,
                                            "export_sample_rate",
                                            &sr_text,
                                            160.0,
                                            |ui| {
                                                for &rate in &sample_rates {
                                                    let label = if rate == 0 {
                                                        t!("dialog.export.follow_global", n = sample_rate).to_string()
                                                    } else {
                                                        format!("{} Hz", rate)
                                                    };
                                                    let selected = r == rate;
                                                    if crate::widgets::combo::combo_item(
                                                        ui, selected, label,
                                                    )
                                                    .clicked()
                                                    {
                                                        sr_cb.set(rate);
                                                    }
                                                }
                                            },
                                        );
                                    });

                                    ui.horizontal(|ui| {
                                        ui.label(t!("dialog.export.xsynth_layers").as_ref());
                                        let mut layers = lc_cb.get() as usize;
                                        ui.add(
                                            crate::widgets::numeric_input::decimal_drag_value(&mut layers)
                                                .range(0..=128)
                                                .speed(1.0),
                                        );
                                        lc_cb.set(layers as u32);
                                        if lc_cb.get() == 0 {
                                            ui.label(t!("common.unlimited").as_ref());
                                        }
                                    });
                                },
                                |ui| {
                                    use crate::chrome::dialog_buttons::{DialogButton, dialog_button_row};
                                    ui.add_space(8.0);
                                    let start = t!("dialog.export.start");
                                    if dialog_button_row(
                                        ui,
                                        &[DialogButton::primary(start.as_ref())],
                                    )
                                    .is_some()
                                    {
                                        *started_cb.borrow_mut() = true;
                                        close = true;
                                    }
                                },
                            );
                        });
                });
            if close {
                vctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                *open_cb.borrow_mut() = false;
            }
        },
    );

    if !*open.borrow() {
        *show = false;
    }
    *bit_depth = bd.get();
    *layer_count = lc.get();
    *export_sample_rate = sr.get();

    *started.borrow()
}

pub(crate) fn format_duration(secs: f64) -> String {
    if secs < 0.0 {
        return "—".into();
    }
    let total = secs as u64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{:02}:{:02}:{:02}", h, m, s)
    } else {
        format!("{:02}:{:02}", m, s)
    }
}
