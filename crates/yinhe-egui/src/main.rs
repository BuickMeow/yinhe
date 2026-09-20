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
#[cfg(test)]
mod i18n_tests;
mod mix;
mod piano_view;
mod platform;
mod plugin_scan;
mod render_context;
mod right_panel;
mod scaling;
mod selection;
mod shortcuts;
mod theme;
mod view_interaction;
mod widgets;

fn main() {
    // 子进程扫描模式：不初始化 GUI，扫描单个 bundle 并输出 JSON 后退出。
    // （部分插件要求主线程加载；子进程隔离崩溃与挂起风险。）
    let argv: Vec<String> = std::env::args().collect();
    if let Some(pos) = argv.iter().position(|a| a == plugin_scan::SCAN_CHILD_ARG) {
        let format = argv.get(pos + 1).map(String::as_str).unwrap_or("");
        let path = argv.get(pos + 2).map(std::path::PathBuf::from);
        plugin_scan::run_scan_child(format, path.as_deref());
        return;
    }

    let mut env_filter = tracing_subscriber::EnvFilter::builder()
        .with_default_directive(tracing::level_filters::LevelFilter::INFO.into())
        .from_env_lossy();
    // 静态字符串，解析失败时忽略（保持默认级别）
    // symphonia_format_riff 的 "ignoring unknown chunk" INFO 日志属于
    // 上游解码器的例行提示（smpl chunk 等），不是我们的错误，压到 warn。
    // epaint::text::fonts 启动时会因 material-icons 家族缺替换字符打
    // "Failed to find replacement characters" 警告，纯噪音，直接关掉。
    for directive in [
        "wgpu=warn",
        "naga=warn",
        "symphonia_format_riff=warn",
        "epaint::text::fonts=off",
    ] {
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
                // 关闭 wgpu 的 indirect args CPU 校验：release 构建默认开启
                // VALIDATION_INDIRECT_CALL，会为每条 indirect draw/dispatch 做
                // CPU 侧检查（全曲视图 39 万条 args 约 20ms/帧）。本程序的
                // indirect args 全部由受控的 cull shader 写入（instance_count
                // 有界、first_instance 在槽位内），无需该保护。
                setup.instance_descriptor.flags = setup
                    .instance_descriptor
                    .flags
                    .difference(wgpu::InstanceFlags::VALIDATION_INDIRECT_CALL);
                setup.device_descriptor = std::sync::Arc::new(|adapter| {
                    let base_limits = if adapter.get_info().backend == wgpu::Backend::Gl {
                        wgpu::Limits::downlevel_webgl2_defaults()
                    } else {
                        wgpu::Limits::default()
                    };
                    // 桌面走间接绘制需 INDIRECT_FIRST_INSTANCE（first_instance=chunk*256）
                    let mut required_features = wgpu::Features::empty();
                    if adapter
                        .features()
                        .contains(wgpu::Features::INDIRECT_FIRST_INSTANCE)
                    {
                        required_features |= wgpu::Features::INDIRECT_FIRST_INSTANCE;
                    }
                    wgpu::DeviceDescriptor {
                        label: Some("egui wgpu device"),
                        required_features,
                        required_limits: wgpu::Limits {
                            max_texture_dimension_2d: 8192,
                            // GPU 合成器需要 13 个 storage buffer（采样块 + 段结构 + 指令）
                            max_storage_buffers_per_shader_stage: 16,
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
