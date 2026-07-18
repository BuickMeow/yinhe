use eframe::egui;

pub(crate) fn show_viewport(
    ctx: &egui::Context,
    open: &mut bool,
    mem_mb: f64,
    metal_size: u64,
) {
    let viewport_id = egui::ViewportId::from_hash_of("memory_breakdown_dialog");
    if !*open {
        return;
    }

    let snapshot = yinhe_memtrace::Snapshot::capture();
    let open_rc = std::rc::Rc::new(std::cell::RefCell::new(true));
    let ctx_clone = ctx.clone();
    let open_cb = open_rc.clone();

    ctx_clone.show_viewport_immediate(
        viewport_id,
        crate::chrome::dialog::viewport_builder(
            "内存占用详情",
            crate::theme::MEM_POPUP_SIZE,
            false,
        ),
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
                    crate::chrome::dialog::title_bar(
                        ui,
                        "内存占用详情",
                        &mut close,
                    );
                    egui::Frame::new()
                        .inner_margin(egui::Margin {
                            left: 12,
                            right: 12,
                            top: 0,
                            bottom: 12,
                        })
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .auto_shrink([false; 2])
                                .show(ui, |ui| {
                                    if yinhe_memtrace::enabled() {
                                        ui.label(format!(
                                            "分配器追踪内存: {:.1} MB",
                                            snapshot.total_mb()
                                        ));
                                    } else {
                                        ui.label(
                                            egui::RichText::new(
                                                "内存分类追踪未启用（用 --features memtrace 编译开启）",
                                            )
                                            .color(egui::Color32::from_gray(140)),
                                        );
                                    }
                                    ui.label(format!("系统统计总内存 (RSS): {:.1} MB", mem_mb));
                                    ui.label(format!(
                                        "wgpu 显式 GPU 资源: {:.1} MB",
                                        snapshot.gpu_mb()
                                    ));

                                    #[cfg(target_os = "macos")]
                                    ui.label(format!(
                                        "Metal 驱动真实显存: {:.1} MB",
                                        metal_size as f64 / 1_048_576.0
                                    ));

                                    if yinhe_memtrace::enabled() {
                                        ui.separator();
                                        ui.heading("按子系统分类");
                                        egui::Grid::new("mem_breakdown_grid")
                                            .num_columns(2)
                                            .spacing([12.0, 8.0])
                                            .show(ui, |ui| {
                                                for tag in yinhe_memtrace::AllocTag::ALL {
                                                    if tag == yinhe_memtrace::AllocTag::Unknown
                                                        && snapshot.get(tag) <= 0
                                                    {
                                                        continue;
                                                    }
                                                    ui.label(tag.name());
                                                    ui.label(format!(
                                                        "{:.1} MB",
                                                        snapshot.mb(tag)
                                                    ));
                                                    ui.end_row();
                                                }
                                            });
                                    }

                                    ui.separator();
                                    ui.small(
                                        "注：GPU 资源计数反映应用显式创建的 wgpu Texture/Buffer 大小；\
                                         驱动层额外开销（swapchain、depth、pipeline cache 等）\
                                         不纳入此项统计。",
                                    );
                                });
                        });
                });
            if close {
                vctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                *open_cb.borrow_mut() = false;
            }
        },
    );

    if !*open_rc.borrow() {
        *open = false;
    }
}
