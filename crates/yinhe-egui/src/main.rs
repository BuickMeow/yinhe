#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

rust_i18n::i18n!("locales", fallback = "zh-CN");

use yinhe_memtrace::TaggedAlloc;

#[global_allocator]
static GLOBAL_ALLOC: TaggedAlloc = TaggedAlloc;

mod app;
mod arrange;
mod audio_settings;
mod chrome;
mod dialogs;
mod file_loader;
mod piano_view;
mod platform;
mod render_context;
mod right_panel;
mod selection;
mod theme;
mod view_interaction;
mod widgets;

fn main() {
    let mut env_filter = tracing_subscriber::EnvFilter::builder()
        .with_default_directive(tracing::level_filters::LevelFilter::INFO.into())
        .from_env_lossy();
    // 静态字符串，解析失败时忽略（保持默认级别）
    // symphonia_format_riff 的 "ignoring unknown chunk" INFO 日志属于
    // 上游解码器的例行提示（smpl chunk 等），不是我们的错误，压到 warn。
    for directive in ["wgpu=warn", "naga=warn", "symphonia_format_riff=warn"] {
        if let Ok(d) = directive.parse() {
            env_filter = env_filter.add_directive(d);
        }
    }
    tracing_subscriber::fmt().with_env_filter(env_filter).init();

    let mut viewport = eframe::egui::ViewportBuilder::default().with_inner_size([1400.0, 900.0]);

    // macOS: with_transparent + fullsize_content_view avoids a white flash and
    // allows the traffic-light buttons to overlay the content area.
    // Windows: with_transparent causes a severe white flash; skip it.
    #[cfg(target_os = "macos")]
    {
        viewport = viewport.with_transparent(true);
    }

    let icon_data = yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Ui, || {
        let icon = image::load_from_memory(include_bytes!("../../../assets/icon.png"))
            .expect("Failed to load window icon")
            .to_rgba8();
        let (icon_w, icon_h) = icon.dimensions();
        egui::IconData {
            rgba: icon.into_raw(),
            width: icon_w,
            height: icon_h,
        }
    });
    viewport = viewport.with_icon(icon_data);

    #[cfg(target_os = "macos")]
    {
        viewport = viewport
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false);
    }

    #[cfg(not(target_os = "macos"))]
    {
        viewport = viewport.with_decorations(false);
    }

    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            wgpu_setup: {
                use eframe::egui_wgpu::wgpu;
                let mut setup = eframe::egui_wgpu::WgpuSetupCreateNew::without_display_handle();
                setup.device_descriptor = std::sync::Arc::new(|adapter| {
                    let base_limits = if adapter.get_info().backend == wgpu::Backend::Gl {
                        wgpu::Limits::downlevel_webgl2_defaults()
                    } else {
                        wgpu::Limits::default()
                    };
                    if !adapter
                        .features()
                        .contains(wgpu::Features::INDIRECT_FIRST_INSTANCE)
                    {
                        tracing::error!(
                            "适配器不支持 INDIRECT_FIRST_INSTANCE，GPU cull 会丢失音符"
                        );
                    }
                    wgpu::DeviceDescriptor {
                        label: Some("egui wgpu device"),
                        // GPU cull 的 multi_draw_indirect 依赖 first_instance≠0
                        // 定位 chunk 槽位；feature 未启用时 wgpu（Metal/DX12）
                        // 会静默丢弃这些 draw，音符大面积丢失。
                        required_features: adapter.features()
                            & wgpu::Features::INDIRECT_FIRST_INSTANCE,
                        required_limits: wgpu::Limits {
                            max_texture_dimension_2d: 8192,
                            ..base_limits
                        },
                        ..Default::default()
                    }
                });
                eframe::egui_wgpu::WgpuSetup::CreateNew(setup)
            },
            ..Default::default()
        },
        ..Default::default()
    };

    eframe::run_native(
        "Yinhe MIDI Editor",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
    .unwrap();
}
